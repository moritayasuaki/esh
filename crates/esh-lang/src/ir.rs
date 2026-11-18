use crate::types::{Effects, Solver, Substitution, Type};
use esh_syntax::{Form, Span};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Location {
    pub source: usize,
    pub span: Span,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub expr: Expr,
    pub location: Option<Location>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Closure {
    pub params: Vec<usize>,
    pub param_types: Vec<Type>,
    pub result: Type,
    pub effects: Effects,
    pub capture_ids: Vec<usize>,
    pub capture_types: Vec<Type>,
    pub captures: Vec<Node>,
    pub body: Box<Node>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Expr {
    Unit,
    Bool(bool),
    Text(String),
    Path(String),
    Ref(usize),
    Cap(String, u8),
    Let {
        binding: usize,
        value: Box<Node>,
        body: Box<Node>,
        short_circuit: bool,
    },
    If {
        condition: Box<Node>,
        yes: Box<Node>,
        no: Box<Node>,
    },
    Function(Closure),
    Call {
        function: Box<Node>,
        args: Vec<Node>,
        effects: Effects,
    },
    Instantiate {
        value: Box<Node>,
        substitution: Substitution,
    },
    Narrow {
        value: Box<Node>,
        rights: u8,
    },
    Widen {
        value: Box<Node>,
        effects: Effects,
    },
    Ok(Box<Node>),
    Error(Type, String),
    List(Type, Vec<Node>),
    Primitive {
        name: String,
        args: Vec<Node>,
    },
}
impl Node {
    pub fn new(expr: Expr) -> Self {
        Self {
            expr,
            location: None,
        }
    }
    pub fn located(mut self, f: &Form, kind: &str) -> Self {
        self.location = Some(Location {
            source: f.source,
            span: f.span,
            kind: kind.into(),
        });
        self
    }
    pub fn reference(id: usize) -> Self {
        Self::new(Expr::Ref(id))
    }
    /// A closure's body is delayed; only its selected captures are evaluated now.
    pub fn children(&self) -> Vec<&Node> {
        match &self.expr {
            Expr::Let { value, body, .. } => vec![value, body],
            Expr::If { condition, yes, no } => vec![condition, yes, no],
            Expr::Function(c) => c.captures.iter().collect(),
            Expr::Call { function, args, .. } => std::iter::once(function.as_ref())
                .chain(args.iter())
                .collect(),
            Expr::Instantiate { value, .. }
            | Expr::Narrow { value, .. }
            | Expr::Widen { value, .. }
            | Expr::Ok(value) => vec![value],
            Expr::List(_, xs) | Expr::Primitive { args: xs, .. } => xs.iter().collect(),
            _ => vec![],
        }
    }
    pub fn free_refs(&self) -> BTreeSet<usize> {
        match &self.expr {
            Expr::Ref(n) => [*n].into(),
            Expr::Let {
                binding,
                value,
                body,
                ..
            } => {
                let mut r = body.free_refs();
                r.remove(binding);
                r.extend(value.free_refs());
                r
            }
            _ => self.children().iter().flat_map(|n| n.free_refs()).collect(),
        }
    }
    pub fn replace_ref(&mut self, old: usize, new: usize) {
        match &mut self.expr {
            Expr::Ref(n) => {
                if *n == old {
                    *n = new;
                }
            }
            Expr::Let { value, body, .. } => {
                value.replace_ref(old, new);
                body.replace_ref(old, new);
            }
            Expr::If { condition, yes, no } => {
                condition.replace_ref(old, new);
                yes.replace_ref(old, new);
                no.replace_ref(old, new);
            }
            Expr::Function(c) => {
                for n in &mut c.captures {
                    n.replace_ref(old, new);
                }
                c.body.replace_ref(old, new);
            }
            Expr::Call { function, args, .. } => {
                function.replace_ref(old, new);
                for n in args {
                    n.replace_ref(old, new);
                }
            }
            Expr::Instantiate { value, .. }
            | Expr::Narrow { value, .. }
            | Expr::Widen { value, .. }
            | Expr::Ok(value) => value.replace_ref(old, new),
            Expr::List(_, xs) | Expr::Primitive { args: xs, .. } => {
                for n in xs {
                    n.replace_ref(old, new);
                }
            }
            _ => {}
        }
    }
    pub fn capture_rights(&self, id: usize, all: u8) -> u8 {
        match &self.expr {
            Expr::Narrow { value, rights } if matches!(value.expr,Expr::Ref(n) if n==id) => *rights,
            Expr::Ref(n) => {
                if *n == id {
                    all
                } else {
                    0
                }
            }
            _ => self
                .children()
                .iter()
                .fold(0, |p, n| p | n.capture_rights(id, all)),
        }
    }
    pub fn resolve(&mut self, s: &Solver) {
        match &mut self.expr {
            Expr::Let { value, body, .. } => {
                value.resolve(s);
                body.resolve(s);
            }
            Expr::If { condition, yes, no } => {
                condition.resolve(s);
                yes.resolve(s);
                no.resolve(s);
            }
            Expr::Function(c) => {
                c.param_types = c.param_types.iter().map(|t| s.resolve(t)).collect();
                c.result = s.resolve(&c.result);
                c.effects = s.effects(&c.effects);
                c.capture_types = c.capture_types.iter().map(|t| s.resolve(t)).collect();
                for n in &mut c.captures {
                    n.resolve(s);
                }
                c.body.resolve(s);
            }
            Expr::Call {
                function,
                args,
                effects,
            } => {
                function.resolve(s);
                for n in args {
                    n.resolve(s);
                }
                *effects = s.effects(effects);
            }
            Expr::Instantiate {
                value,
                substitution,
            } => {
                value.resolve(s);
                for t in substitution.types.values_mut() {
                    *t = s.resolve(t);
                }
                for r in substitution.regions.values_mut() {
                    *r = s.region(r);
                }
                for e in substitution.effects.values_mut() {
                    *e = s.effects(e);
                }
            }
            Expr::Narrow { value, .. } | Expr::Ok(value) => value.resolve(s),
            Expr::Widen { value, effects } => {
                value.resolve(s);
                *effects = s.effects(effects);
            }
            Expr::List(t, xs) => {
                *t = s.resolve(t);
                for n in xs {
                    n.resolve(s);
                }
            }
            Expr::Primitive { args, .. } => {
                for n in args {
                    n.resolve(s);
                }
            }
            Expr::Error(t, _) => *t = s.resolve(t),
            _ => {}
        }
    }
}
