//! Bounded static projection of ordered operations. This never touches the OS.
use crate::{
    compiler::Program,
    contracts::{Contracts, Predicate},
    ir::{Closure, Expr, Location, Node},
    types::{rights_names, Effect, Region},
};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
enum Symbolic {
    Unknown,
    Unit,
    Bool(bool),
    Text(String),
    Path(String),
    Cap(String),
    List(Vec<Symbolic>),
    Function(Arc<Closure>, Vec<Symbolic>),
    Result {
        value: Box<Symbolic>,
        condition: String,
        failed: bool,
    },
    Choices(Vec<(Vec<String>, Symbolic)>),
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    pub order: usize,
    pub operation: String,
    pub region: Option<String>,
    pub path: Option<String>,
    pub behavior: String,
    pub conditions: Vec<String>,
    pub repetition: String,
    pub location: Option<Location>,
    pub unknowns: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub impacts: Vec<Impact>,
    pub complete: bool,
    pub assumptions: Vec<String>,
}
struct Projection<'a> {
    contracts: &'a Contracts,
    rows: Vec<Impact>,
    remaining: usize,
    complete: bool,
    initial_facts_valid: bool,
}
type Env = BTreeMap<usize, Symbolic>;

pub fn file_impacts(program: &Program) -> ImpactReport {
    let mut p = Projection {
        contracts: &program.contracts,
        rows: vec![],
        remaining: 10_000,
        complete: true,
        initial_facts_valid: true,
    };
    p.eval(&program.node, &Env::new(), &[], None, 0);
    ImpactReport {
        impacts: p.rows,
        complete: p.complete,
        assumptions: vec![
            "Supplied initial conditions hold when validated.".into(),
            "State-dependent predictions require the declared non-interference assumptions.".into(),
        ],
    }
}
impl Projection<'_> {
    fn row(
        &mut self,
        operation: &str,
        region: Option<String>,
        path: Option<String>,
        conditions: &[String],
        location: Option<Location>,
        opaque: bool,
    ) {
        let behavior=match operation{"fs.write-text"|"fs.write-bytes"=>"Replace all contents if present; create otherwise.","fs.remove"=>"Remove the regular-file directory entry.","fs.mkdir"=>"Create a directory; fail if the target already exists.","fs.read-text"|"fs.read-bytes"=>"Read file contents.","fs.list"=>"Read sorted directory entry names.",_=>"Operation permitted by an opaque callback contract; internal order and count are unknown."};
        let mut unknowns = vec![];
        if path.is_none() {
            unknowns.push("Target path is determined at runtime.".into());
        }
        if region.is_none() {
            unknowns.push("Target capability region is unresolved in this projection.".into());
        }
        if opaque {
            unknowns.push("The callback body is unavailable to this projection.".into());
        }
        let order = self.rows.len() + 1;
        self.rows.push(Impact {
            order,
            operation: operation.into(),
            region,
            path,
            behavior: behavior.into(),
            conditions: conditions.to_vec(),
            repetition: "once per reached call".into(),
            location,
            unknowns,
        });
    }
    fn eval(
        &mut self,
        n: &Node,
        env: &Env,
        conditions: &[String],
        call_site: Option<Location>,
        depth: usize,
    ) -> Symbolic {
        if self.remaining == 0 || depth >= 256 {
            self.complete = false;
            return Symbolic::Unknown;
        }
        self.remaining -= 1;
        let site = n.location.clone().or(call_site.clone());
        match &n.expr {
            Expr::Unit => Symbolic::Unit,
            Expr::Bool(b) => Symbolic::Bool(*b),
            Expr::Text(s) => Symbolic::Text(s.clone()),
            Expr::Path(p) => Symbolic::Path(p.clone()),
            Expr::Cap(r, _) => Symbolic::Cap(r.clone()),
            Expr::Ref(id) => env.get(id).cloned().unwrap_or(Symbolic::Unknown),
            Expr::Instantiate { value, .. }
            | Expr::Widen { value, .. }
            | Expr::Narrow { value, .. } => self.eval(value, env, conditions, site, depth + 1),
            Expr::Let {
                binding,
                value,
                body,
                short_circuit,
            } => {
                let value = self.eval(value, env, conditions, site.clone(), depth + 1);
                let mut cs = conditions.to_vec();
                let value = if *short_circuit {
                    match value {
                        Symbolic::Result {
                            value,
                            condition,
                            failed,
                        } => {
                            if failed {
                                return Symbolic::Result {
                                    value,
                                    condition,
                                    failed,
                                };
                            }
                            if !condition.is_empty() {
                                cs.push(condition);
                            }
                            *value
                        }
                        _ => {
                            cs.push("The preceding Result is Ok.".into());
                            Symbolic::Unknown
                        }
                    }
                } else {
                    value
                };
                let mut scope = env.clone();
                scope.insert(*binding, value);
                self.eval(body, &scope, &cs, site, depth + 1)
            }
            Expr::Function(c) => Symbolic::Function(
                Arc::new(c.clone()),
                c.captures
                    .iter()
                    .map(|n| self.eval(n, env, conditions, site.clone(), depth + 1))
                    .collect(),
            ),
            Expr::Call {
                function,
                args,
                effects,
            } => {
                let f = self.eval(function, env, conditions, site.clone(), depth + 1);
                let args = args
                    .iter()
                    .map(|n| self.eval(n, env, conditions, site.clone(), depth + 1))
                    .collect();
                self.apply(f, args, effects, conditions, site, depth + 1)
            }
            Expr::If { condition, yes, no } => {
                match self.eval(condition, env, conditions, site.clone(), depth + 1) {
                    Symbolic::Bool(b) => {
                        self.eval(if b { yes } else { no }, env, conditions, site, depth + 1)
                    }
                    _ => {
                        let mut a = conditions.to_vec();
                        a.push("The branch condition is true.".into());
                        let mut b = conditions.to_vec();
                        b.push("The branch condition is false.".into());
                        let yes = self.eval(yes, env, &a, site.clone(), depth + 1);
                        let no = self.eval(no, env, &b, site, depth + 1);
                        Symbolic::Choices(vec![(a, yes), (b, no)])
                    }
                }
            }
            Expr::Ok(v) => Symbolic::Result {
                value: Box::new(self.eval(v, env, conditions, site, depth + 1)),
                condition: String::new(),
                failed: false,
            },
            Expr::Error(..) => Symbolic::Result {
                value: Box::new(Symbolic::Unknown),
                condition: String::new(),
                failed: true,
            },
            Expr::List(_, xs) => Symbolic::List(
                xs.iter()
                    .map(|n| self.eval(n, env, conditions, site.clone(), depth + 1))
                    .collect(),
            ),
            Expr::Primitive { name, args } => {
                let args = args
                    .iter()
                    .map(|n| self.eval(n, env, conditions, site.clone(), depth + 1))
                    .collect();
                self.primitive(name, args, conditions, site, depth + 1)
            }
        }
    }
    fn apply(
        &mut self,
        f: Symbolic,
        args: Vec<Symbolic>,
        effects: &[Effect],
        conditions: &[String],
        site: Option<Location>,
        depth: usize,
    ) -> Symbolic {
        match f {
            Symbolic::Function(c, captures) => {
                let env = c
                    .params
                    .iter()
                    .copied()
                    .zip(args)
                    .chain(c.capture_ids.iter().copied().zip(captures))
                    .collect();
                self.eval(&c.body, &env, conditions, site, depth + 1)
            }
            Symbolic::Choices(choices) => Symbolic::Choices(
                choices
                    .into_iter()
                    .map(|(cs, f)| {
                        let mut combined = conditions.to_vec();
                        for c in cs {
                            if !combined.contains(&c) {
                                combined.push(c);
                            }
                        }
                        let value = self.apply(
                            f,
                            args.clone(),
                            effects,
                            &combined,
                            site.clone(),
                            depth + 1,
                        );
                        (combined, value)
                    })
                    .collect(),
            ),
            _ => {
                self.initial_facts_valid = false;
                if effects.is_empty() {
                    self.complete = false;
                    self.row("unknown call", None, None, conditions, site.clone(), true);
                }
                for e in effects {
                    match e {
                        Effect::Fs(r, p) => self.row(
                            &format!("fs.{}", rights_names(*p).join(" ")),
                            if let Region::Fixed(r) = r {
                                Some(r.clone())
                            } else {
                                None
                            },
                            None,
                            conditions,
                            site.clone(),
                            true,
                        ),
                        Effect::Var(_) => {
                            self.complete = false;
                            self.row("unknown effect", None, None, conditions, site.clone(), true);
                        }
                    }
                }
                Symbolic::Unknown
            }
        }
    }
    fn primitive(
        &mut self,
        name: &str,
        args: Vec<Symbolic>,
        conditions: &[String],
        site: Option<Location>,
        depth: usize,
    ) -> Symbolic {
        match (name, args.as_slice()) {
            ("text.concat", [Symbolic::Text(a), Symbolic::Text(b)]) => {
                Symbolic::Text(format!("{a}{b}"))
            }
            ("text.eq", [Symbolic::Text(a), Symbolic::Text(b)]) => Symbolic::Bool(a == b),
            ("path.parse", [Symbolic::Text(s)]) => Symbolic::Result {
                value: Box::new(Symbolic::Path(s.clone())),
                condition: String::new(),
                failed: !esh_policy::valid_relative_path(s.as_bytes()),
            },
            ("path.join", [Symbolic::Path(base), Symbolic::Text(part)]) => Symbolic::Result {
                value: Box::new(Symbolic::Path(if base == "." {
                    part.clone()
                } else {
                    format!("{base}/{part}")
                })),
                condition: String::new(),
                failed: !esh_policy::valid_component(part.as_bytes()),
            },
            (n, _) if n.starts_with("fs.") => {
                let region = args.first().and_then(|v| {
                    if let Symbolic::Cap(r) = v {
                        Some(r.clone())
                    } else {
                        None
                    }
                });
                let path = args.get(1).and_then(|v| {
                    if let Symbolic::Path(p) = v {
                        Some(p.clone())
                    } else {
                        None
                    }
                });
                self.row(n, region.clone(), path.clone(), conditions, site, false);
                let value = if n == "fs.read-text" && self.initial_facts_valid {
                    self.contracts
                        .requires
                        .iter()
                        .find_map(|c| {
                            if Some(&c.target.region) == region.as_ref()
                                && Some(&c.target.path) == path.as_ref()
                            {
                                if let Predicate::ExactText(s) = &c.predicate {
                                    return Some(Symbolic::Text(s.clone()));
                                }
                            }
                            None
                        })
                        .unwrap_or(Symbolic::Unknown)
                } else {
                    Symbolic::Unknown
                };
                if matches!(
                    n,
                    "fs.write-text" | "fs.write-bytes" | "fs.remove" | "fs.mkdir"
                ) {
                    self.initial_facts_valid = false;
                }
                Symbolic::Result {
                    value: Box::new(value),
                    condition: format!(
                        "{n} at {}:/{} succeeds.",
                        region.unwrap_or_else(|| "?".into()),
                        path.unwrap_or_else(|| "<dynamic>".into())
                    ),
                    failed: false,
                }
            }
            (n, _) if n.starts_with("list.") => {
                let fold = n.contains("fold");
                let stop = n.ends_with("-ok");
                let callback = args.first().cloned().unwrap_or(Symbolic::Unknown);
                let mut accumulator = if fold {
                    args.get(1).cloned().unwrap_or(Symbolic::Unknown)
                } else {
                    Symbolic::Unit
                };
                let list = args.last().cloned().unwrap_or(Symbolic::Unknown);
                let mut mapped = vec![];
                let mut cs = conditions.to_vec();
                let (items, unknown) = if let Symbolic::List(xs) = list {
                    (xs, false)
                } else {
                    (vec![Symbolic::Unknown], true)
                };
                let start = self.rows.len();
                if unknown {
                    cs.push(
                        "For each runtime list element, in list order; the list may be empty."
                            .into(),
                    );
                }
                for item in items {
                    if self.remaining == 0 {
                        self.complete = false;
                        break;
                    }
                    let values = if fold {
                        vec![accumulator.clone(), item]
                    } else {
                        vec![item]
                    };
                    let mut value =
                        self.apply(callback.clone(), values, &[], &cs, site.clone(), depth + 1);
                    if stop {
                        if let Symbolic::Result {
                            value: v,
                            condition,
                            failed,
                        } = value
                        {
                            if failed {
                                return Symbolic::Result {
                                    value: v,
                                    condition,
                                    failed,
                                };
                            }
                            if !condition.is_empty() {
                                cs.push(condition);
                            }
                            value = *v;
                        } else {
                            cs.push("All preceding callback results are Ok.".into());
                        }
                    }
                    if fold {
                        accumulator = value;
                    } else {
                        mapped.push(value);
                    }
                }
                if unknown {
                    for row in &mut self.rows[start..] {
                        row.repetition =
                            "E*: finite ordered repetition; stop at first Err for -ok".into();
                    }
                    accumulator = Symbolic::Unknown;
                }
                let value = if fold {
                    accumulator
                } else if unknown {
                    Symbolic::Unknown
                } else {
                    Symbolic::List(mapped)
                };
                if stop {
                    Symbolic::Result {
                        value: Box::new(value),
                        condition: "The entire ordered traversal succeeds.".into(),
                        failed: false,
                    }
                } else {
                    value
                }
            }
            _ => Symbolic::Unknown,
        }
    }
}
