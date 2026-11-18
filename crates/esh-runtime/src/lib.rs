//! Sequential Rust evaluator. Host calls remain behind a separately checked boundary.
#![forbid(unsafe_code)]
pub mod update;
use esh_host::{HostError, Operation, Request};
use esh_lang::compiler::Program;
use esh_lang::ir::{Closure, Expr, Node};
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Text(Arc<str>),
    Bytes(Arc<[u8]>),
    Path(Arc<str>),
    List(Arc<[Value]>),
    Ok(Arc<Value>),
    Error(HostError),
    Capability {
        region: String,
        rights: u8,
    },
    Function {
        code: Arc<Closure>,
        captures: Vec<Value>,
    },
}
impl Value {
    pub fn to_json(&self) -> Json {
        match self {
            Self::Unit => Json::Null,
            Self::Bool(b) => json!(b),
            Self::Text(t) | Self::Path(t) => json!(t.as_ref()),
            Self::Bytes(b) => json!({"bytes":b.as_ref()}),
            Self::List(xs) => json!(xs.iter().map(Value::to_json).collect::<Vec<_>>()),
            Self::Ok(v) => json!({"ok":v.to_json()}),
            Self::Error(e) => json!({"error":e}),
            Self::Capability { region, rights } => {
                json!({"capability":region,"permissions":esh_lang::types::rights_names(*rights)})
            }
            Self::Function { .. } => json!({"function":true}),
        }
    }
    pub fn failed(&self) -> bool {
        matches!(self, Self::Error(_))
    }
    fn sized(&self, budget: &mut usize) -> bool {
        let cost = match self {
            Self::Text(t) | Self::Path(t) => t.len().saturating_add(1),
            Self::Bytes(b) => b.len().saturating_add(1),
            _ => 1,
        };
        if cost > *budget {
            return false;
        }
        *budget -= cost;
        match self {
            Self::List(xs) => xs.iter().all(|x| x.sized(budget)),
            Self::Ok(v) => v.sized(budget),
            Self::Function { captures, .. } => captures.iter().all(|x| x.sized(budget)),
            _ => true,
        }
    }
}
pub trait Backend {
    fn begin(&mut self, p: &Program) -> Result<(), HostError> {
        if p.contracts.requires.is_empty()
            && p.contracts.preserves.is_empty()
            && p.contracts.ensures.is_empty()
        {
            Ok(())
        } else {
            Err(HostError::new(
                "contract-backend",
                "Update contracts require a validating backend.",
            ))
        }
    }
    fn finish(&mut self) -> Result<(), HostError> {
        Ok(())
    }
    fn halted(&self) -> bool {
        false
    }
    fn request(&mut self, request: &Request) -> Result<Json, HostError>;
}
impl Backend for esh_host::Host {
    fn request(&mut self, request: &Request) -> Result<Json, HostError> {
        self.perform(request)
    }
}
#[derive(Clone)]
pub struct Limits {
    pub max_steps: u64,
    pub max_value_bytes: usize,
    pub cancelled: Arc<AtomicBool>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            max_value_bytes: 8 * 1024 * 1024,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}
pub struct Runtime<'a, B: Backend> {
    backend: &'a mut B,
    limits: Limits,
    pub steps: u64,
    pub trace: Vec<Json>,
    depth: usize,
}
type Env = BTreeMap<usize, Value>;
impl<'a, B: Backend> Runtime<'a, B> {
    pub fn new(backend: &'a mut B, limits: Limits) -> Self {
        Self {
            backend,
            limits,
            steps: 0,
            trace: vec![],
            depth: 0,
        }
    }
    fn boundary(&mut self) -> Result<(), HostError> {
        if self.limits.cancelled.load(Ordering::Relaxed) {
            return Err(HostError::new(
                "cancelled",
                "Execution was cancelled at a semantic boundary.",
            ));
        }
        if self.steps >= self.limits.max_steps {
            return Err(HostError::new(
                "step-limit",
                "Execution exceeded max-steps.",
            ));
        }
        self.steps += 1;
        Ok(())
    }
    pub fn run(&mut self, p: &Program) -> Result<Value, HostError> {
        self.boundary()?;
        self.backend.begin(p)?;
        let result = self.evaluate(&p.node, &Env::new())?;
        if !result.failed() {
            self.backend.finish()?;
        }
        Ok(result)
    }
    fn protocol() -> HostError {
        HostError::new(
            "runtime-type",
            "The executable value did not match the checked type.",
        )
    }
    fn evaluate(&mut self, n: &Node, env: &Env) -> Result<Value, HostError> {
        if self.depth >= 512 {
            return Err(HostError::new(
                "depth-limit",
                "Execution exceeded the structural call-depth limit.",
            ));
        }
        self.depth += 1;
        let result = self.eval(n, env);
        self.depth -= 1;
        let value = result?;
        let mut remaining = self.limits.max_value_bytes;
        if !value.sized(&mut remaining) {
            return Err(HostError::new(
                "resource-limit",
                "A value exceeds max-value-bytes.",
            ));
        }
        Ok(value)
    }
    fn eval(&mut self, n: &Node, env: &Env) -> Result<Value, HostError> {
        match &n.expr {
            Expr::Unit => Ok(Value::Unit),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Text(s) => Ok(Value::Text(s.as_str().into())),
            Expr::Path(s) => Ok(Value::Path(s.as_str().into())),
            Expr::Ref(id) => env.get(id).cloned().ok_or_else(Self::protocol),
            Expr::Cap(region, rights) => Ok(Value::Capability {
                region: region.clone(),
                rights: *rights,
            }),
            Expr::Let {
                binding,
                value,
                body,
                short_circuit,
            } => {
                self.boundary()?;
                let mut value = self.evaluate(value, env)?;
                if *short_circuit {
                    match value {
                        Value::Ok(v) => value = (*v).clone(),
                        Value::Error(_) => return Ok(value),
                        _ => return Err(Self::protocol()),
                    }
                }
                let mut scope = env.clone();
                scope.insert(*binding, value);
                self.evaluate(body, &scope)
            }
            Expr::If { condition, yes, no } => {
                let Value::Bool(c) = self.evaluate(condition, env)? else {
                    return Err(Self::protocol());
                };
                self.evaluate(if c { yes } else { no }, env)
            }
            Expr::Function(c) => {
                let captures = c
                    .captures
                    .iter()
                    .map(|n| self.evaluate(n, env))
                    .collect::<Result<_, _>>()?;
                Ok(Value::Function {
                    code: Arc::new(c.clone()),
                    captures,
                })
            }
            Expr::Call { function, args, .. } => {
                let fun = self.evaluate(function, env)?;
                let args = args
                    .iter()
                    .map(|n| self.evaluate(n, env))
                    .collect::<Result<_, _>>()?;
                self.apply(fun, args)
            }
            Expr::Instantiate { value, .. } | Expr::Widen { value, .. } => {
                self.evaluate(value, env)
            }
            Expr::Narrow { value, rights } => {
                let Value::Capability {
                    region,
                    rights: granted,
                } = self.evaluate(value, env)?
                else {
                    return Err(Self::protocol());
                };
                if !esh_policy::covers(granted, *rights) {
                    return Err(Self::protocol());
                }
                Ok(Value::Capability {
                    region,
                    rights: *rights,
                })
            }
            Expr::Ok(v) => Ok(Value::Ok(Arc::new(self.evaluate(v, env)?))),
            Expr::Error(_, message) => Ok(Value::Error(HostError::new("script", message))),
            Expr::List(_, xs) => Ok(Value::List(
                xs.iter()
                    .map(|n| self.evaluate(n, env))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            )),
            Expr::Primitive { name, args } => {
                let values = args
                    .iter()
                    .map(|n| self.evaluate(n, env))
                    .collect::<Result<Vec<_>, _>>()?;
                self.primitive(name, values)
            }
        }
    }
    fn apply(&mut self, fun: Value, args: Vec<Value>) -> Result<Value, HostError> {
        self.boundary()?;
        let Value::Function { code, captures } = fun else {
            return Err(Self::protocol());
        };
        if code.params.len() != args.len() || code.capture_ids.len() != captures.len() {
            return Err(Self::protocol());
        }
        let env = code
            .params
            .iter()
            .copied()
            .zip(args)
            .chain(code.capture_ids.iter().copied().zip(captures))
            .collect();
        self.evaluate(&code.body, &env)
    }
    fn primitive(&mut self, name: &str, args: Vec<Value>) -> Result<Value, HostError> {
        match (name, args.as_slice()) {
            ("text.concat", [Value::Text(a), Value::Text(b)]) => {
                if a.len().saturating_add(b.len()) > self.limits.max_value_bytes {
                    return Err(HostError::new(
                        "resource-limit",
                        "Text exceeds max-value-bytes.",
                    ));
                }
                Ok(Value::Text(format!("{a}{b}").into()))
            }
            ("text.eq", [Value::Text(a), Value::Text(b)]) => Ok(Value::Bool(a == b)),
            ("utf8.encode", [Value::Text(t)]) => Ok(Value::Bytes(t.as_bytes().into())),
            ("utf8.decode", [Value::Bytes(b)]) => Ok(match std::str::from_utf8(b) {
                Ok(t) => Value::Ok(Arc::new(Value::Text(t.into()))),
                Err(_) => Value::Error(HostError::new("encoding", "Expected valid UTF-8.")),
            }),
            ("path.parse", [Value::Text(t)]) => Ok(Self::checked_path(t)),
            ("path.join", [Value::Path(base), Value::Text(component)]) => {
                if !esh_policy::valid_component(component.as_bytes()) {
                    return Ok(Value::Error(HostError::new(
                        "path",
                        "Expected one valid path component.",
                    )));
                }
                let path = if base.as_ref() == "." {
                    component.to_string()
                } else {
                    format!("{base}/{component}")
                };
                Ok(Self::checked_path(&path))
            }
            (n, _) if n.starts_with("list.") => self.iterate(n, args),
            (n, _) if n.starts_with("fs.") => self.filesystem(n, args),
            _ => Err(Self::protocol()),
        }
    }
    fn checked_path(path: &str) -> Value {
        if esh_policy::valid_relative_path(path.as_bytes()) {
            Value::Ok(Arc::new(Value::Path(path.into())))
        } else {
            Value::Error(HostError::new("path", "Invalid relative path."))
        }
    }
    fn iterate(&mut self, name: &str, args: Vec<Value>) -> Result<Value, HostError> {
        let fold = name.contains("fold");
        let stop = name.ends_with("-ok");
        let (callback, initial, list) = match args.as_slice() {
            [callback, initial, Value::List(xs)] if fold => {
                (callback.clone(), initial.clone(), xs.clone())
            }
            [callback, Value::List(xs)] if !fold => (callback.clone(), Value::Unit, xs.clone()),
            _ => return Err(Self::protocol()),
        };
        let mut accumulator = initial;
        let mut mapped = vec![];
        let mut remaining = self.limits.max_value_bytes;
        for value in list.iter() {
            self.boundary()?;
            let args = if fold {
                vec![accumulator.clone(), value.clone()]
            } else {
                vec![value.clone()]
            };
            let mut result = self.apply(callback.clone(), args)?;
            if stop {
                result = match result {
                    Value::Ok(v) => (*v).clone(),
                    Value::Error(_) => return Ok(result),
                    _ => return Err(Self::protocol()),
                };
            }
            if fold {
                accumulator = result;
            } else {
                if !result.sized(&mut remaining) {
                    return Err(HostError::new(
                        "resource-limit",
                        "Mapped list exceeds max-value-bytes.",
                    ));
                }
                mapped.push(result);
            }
        }
        let result = if fold {
            accumulator
        } else {
            Value::List(mapped.into())
        };
        Ok(if stop {
            Value::Ok(Arc::new(result))
        } else {
            result
        })
    }
    fn filesystem(&mut self, name: &str, args: Vec<Value>) -> Result<Value, HostError> {
        let (operation, permission) = match name {
            "fs.read-text" => (Operation::Read, esh_policy::Permission::Read),
            "fs.read-bytes" => (Operation::ReadBytes, esh_policy::Permission::Read),
            "fs.write-text" => (Operation::Write, esh_policy::Permission::Write),
            "fs.write-bytes" => (Operation::WriteBytes, esh_policy::Permission::Write),
            "fs.list" => (Operation::List, esh_policy::Permission::Read),
            "fs.mkdir" => (Operation::Mkdir, esh_policy::Permission::Mkdir),
            "fs.remove" => (Operation::Remove, esh_policy::Permission::Remove),
            _ => return Err(Self::protocol()),
        };
        let [Value::Capability { region, rights }, Value::Path(path), ..] = args.as_slice() else {
            return Err(Self::protocol());
        };
        if !esh_policy::allows(*rights, permission) {
            return Err(HostError::new(
                "permission",
                "Runtime capability does not authorize this operation.",
            ));
        }
        let contents = match args.get(2) {
            Some(Value::Text(t)) => Some(t.to_string()),
            _ => None,
        };
        let bytes = match args.get(2) {
            Some(Value::Bytes(b)) => Some(b.to_vec()),
            _ => None,
        };
        self.boundary()?;
        let request = Request {
            operation,
            region: region.clone(),
            path: path.to_string(),
            contents,
            bytes,
        };
        let response = self.backend.request(&request);
        self.trace.push(json!({"operation":name,"region":region,"path":path.as_ref(),"status":if response.is_ok(){"ok"}else{"error"}}));
        if self.backend.halted() {
            return Err(response.err().unwrap_or_else(Self::protocol));
        }
        let result = match response {
            Err(e) => return Ok(Value::Error(e)),
            Ok(j) => match operation {
                Operation::Read => Value::Text(j.as_str().ok_or_else(Self::protocol)?.into()),
                Operation::ReadBytes => Value::Bytes(
                    serde_json::from_value::<Vec<u8>>(j)
                        .map_err(|_| Self::protocol())?
                        .into(),
                ),
                Operation::List => Value::List(
                    serde_json::from_value::<Vec<String>>(j)
                        .map_err(|_| Self::protocol())?
                        .into_iter()
                        .map(|s| Value::Text(s.into()))
                        .collect::<Vec<_>>()
                        .into(),
                ),
                _ => {
                    if !j.is_null() {
                        return Err(Self::protocol());
                    }
                    Value::Unit
                }
            },
        };
        Ok(Value::Ok(Arc::new(result)))
    }
}
