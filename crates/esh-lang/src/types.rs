use esh_syntax::{Diagnostic, Form};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const RIGHTS: [&str; 4] = ["read", "write", "mkdir", "remove"];
pub fn rights_names(mask: u8) -> Vec<&'static str> {
    RIGHTS
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (mask & (1 << i) != 0).then_some(*p))
        .collect()
}
pub fn error(f: &Form, code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
        source: f.source,
        span: f.span,
    }
}
pub type Checked<T> = Result<T, Diagnostic>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    Type,
    Region,
    Effect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Variable {
    pub kind: Kind,
    pub id: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Region {
    Fixed(String),
    Var(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rights {
    Fixed(u8),
    Meta(usize),
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Effect {
    Fs(Region, u8),
    Var(usize),
}
pub type Effects = Vec<Effect>;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Type {
    Unit,
    Bool,
    Text,
    Bytes,
    Path,
    Var(usize),
    Cap(Region, Rights),
    Fn(Arc<[Type]>, Arc<Type>, Effects),
    List(Arc<Type>),
    Result(Arc<Type>),
}
impl Type {
    /// Bound traversal work even when a shared type graph has a large expansion.
    pub fn within_limit(&self) -> bool {
        fn visit(ty: &Type, left: &mut usize, depth: usize) -> bool {
            if *left == 0 || depth > 128 {
                return false;
            }
            *left -= 1;
            match ty {
                Type::Fn(args, ret, _) => {
                    args.iter().all(|t| visit(t, left, depth + 1)) && visit(ret, left, depth + 1)
                }
                Type::List(t) | Type::Result(t) => visit(t, left, depth + 1),
                _ => true,
            }
        }
        visit(self, &mut 8192, 0)
    }
    pub fn list(self) -> Self {
        Self::List(Arc::new(self))
    }
    pub fn result(self) -> Self {
        Self::Result(Arc::new(self))
    }
    pub fn function(args: Vec<Self>, ret: Self, effects: Effects) -> Self {
        Self::Fn(args.into(), Arc::new(ret), effects)
    }
    pub fn authority(&self) -> bool {
        match self {
            Self::Var(_) | Self::Cap(..) | Self::Fn(..) => true,
            Self::List(t) | Self::Result(t) => t.authority(),
            _ => false,
        }
    }
    pub fn free(&self) -> BTreeSet<Variable> {
        let mut out = BTreeSet::new();
        match self {
            Self::Var(id) => {
                out.insert(Variable {
                    kind: Kind::Type,
                    id: *id,
                });
            }
            Self::Cap(r, _) => {
                out.extend(r.free());
            }
            Self::Fn(args, ret, effects) => {
                for t in args.iter() {
                    out.extend(t.free());
                }
                out.extend(ret.free());
                out.extend(effect_free(effects));
            }
            Self::List(t) | Self::Result(t) => out.extend(t.free()),
            _ => {}
        }
        out
    }
    pub fn subst(&self, s: &Substitution) -> Self {
        match self {
            Self::Var(n) => s.types.get(n).cloned().unwrap_or_else(|| self.clone()),
            Self::Cap(r, p) => Self::Cap(r.subst(s), *p),
            Self::Fn(a, r, e) => Self::function(
                a.iter().map(|t| t.subst(s)).collect(),
                r.subst(s),
                effect_subst(e, s),
            ),
            Self::List(t) => t.subst(s).list(),
            Self::Result(t) => t.subst(s).result(),
            _ => self.clone(),
        }
    }
}
impl Region {
    pub fn free(&self) -> BTreeSet<Variable> {
        match self {
            Self::Var(id) => [Variable {
                kind: Kind::Region,
                id: *id,
            }]
            .into(),
            _ => BTreeSet::new(),
        }
    }
    pub fn subst(&self, s: &Substitution) -> Self {
        match self {
            Self::Var(n) => s.regions.get(n).cloned().unwrap_or_else(|| self.clone()),
            _ => self.clone(),
        }
    }
}
pub fn effect_free(es: &[Effect]) -> BTreeSet<Variable> {
    es.iter()
        .flat_map(|e| match e {
            Effect::Fs(r, _) => r.free(),
            Effect::Var(id) => [Variable {
                kind: Kind::Effect,
                id: *id,
            }]
            .into(),
        })
        .collect()
}
pub fn union(es: impl IntoIterator<Item = Effect>) -> Effects {
    let mut result = Vec::new();
    for e in es {
        if !result.contains(&e) {
            result.push(e);
        }
    }
    result
}
pub fn effect_subst(es: &[Effect], s: &Substitution) -> Effects {
    union(es.iter().flat_map(|e| match e {
        Effect::Fs(r, p) => vec![Effect::Fs(r.subst(s), *p)],
        Effect::Var(n) => s.effects.get(n).cloned().unwrap_or_else(|| vec![e.clone()]),
    }))
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Substitution {
    pub types: BTreeMap<usize, Type>,
    pub regions: BTreeMap<usize, Region>,
    pub effects: BTreeMap<usize, Effects>,
}
#[derive(Clone, Debug)]
pub struct Scheme {
    pub ty: Type,
    pub quant: Vec<Variable>,
}
impl Scheme {
    pub fn mono(ty: Type) -> Self {
        Self { ty, quant: vec![] }
    }
}

#[derive(Default)]
pub struct Solver {
    next: usize,
    pub sub: Substitution,
    pub rigid: BTreeSet<Variable>,
    pub names: BTreeMap<Variable, String>,
    pub rights: BTreeMap<usize, u8>,
}
impl Solver {
    pub fn fresh(&mut self, kind: Kind, name: Option<&str>, rigid: bool) -> Variable {
        let v = Variable {
            kind,
            id: self.next,
        };
        self.next += 1;
        self.names.insert(
            v,
            name.map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{}{}",
                    match kind {
                        Kind::Type => "T",
                        Kind::Region => "R",
                        Kind::Effect => "E",
                    },
                    v.id
                )
            }),
        );
        if rigid {
            self.rigid.insert(v);
        }
        v
    }
    pub fn fresh_type(&mut self) -> Type {
        Type::Var(self.fresh(Kind::Type, None, false).id)
    }
    pub fn is_flexible(&self, kind: Kind, id: usize) -> bool {
        !self.rigid.contains(&Variable { kind, id })
    }
    pub fn region(&self, r: &Region) -> Region {
        match r {
            Region::Var(id) => self
                .sub
                .regions
                .get(id)
                .map(|t| self.region(t))
                .unwrap_or_else(|| r.clone()),
            _ => r.clone(),
        }
    }
    pub fn permissions(&self, p: Rights) -> u8 {
        match p {
            Rights::Fixed(p) => p,
            Rights::Meta(n) => self.rights[&n],
        }
    }
    pub fn effects(&self, es: &[Effect]) -> Effects {
        union(es.iter().flat_map(|e| {
            match e {
                Effect::Fs(r, p) => vec![Effect::Fs(self.region(r), *p)],
                Effect::Var(n) => self
                    .sub
                    .effects
                    .get(n)
                    .map(|x| self.effects(x))
                    .unwrap_or_else(|| vec![e.clone()]),
            }
        }))
    }
    pub fn resolve(&self, t: &Type) -> Type {
        match t {
            Type::Var(n) => self
                .sub
                .types
                .get(n)
                .map(|x| self.resolve(x))
                .unwrap_or_else(|| t.clone()),
            Type::Cap(r, p) => Type::Cap(self.region(r), Rights::Fixed(self.permissions(*p))),
            Type::Fn(a, r, e) => Type::function(
                a.iter().map(|t| self.resolve(t)).collect(),
                self.resolve(r),
                self.effects(e),
            ),
            Type::List(t) => self.resolve(t).list(),
            Type::Result(t) => self.resolve(t).result(),
            _ => t.clone(),
        }
    }
    pub fn bind_type(&mut self, n: usize, t: &Type, f: &Form) -> Checked<()> {
        let t = self.resolve(t);
        if !t.within_limit() {
            return Err(error(
                f,
                "type-limit",
                "Type expansion exceeds the analysis limit.",
            ));
        }
        if t == Type::Var(n) {
            return Ok(());
        }
        if !self.is_flexible(Kind::Type, n) {
            return Err(error(
                f,
                "type-mismatch",
                "A universally quantified parameter cannot be specialized inside its definition.",
            ));
        }
        if t.free().contains(&Variable {
            kind: Kind::Type,
            id: n,
        }) {
            return Err(error(
                f,
                "recursive-type",
                "Recursive types are outside this stage.",
            ));
        }
        self.sub.types.insert(n, t);
        Ok(())
    }
    pub fn unify_region(&mut self, a: &Region, b: &Region, f: &Form) -> Checked<()> {
        let (a, b) = (self.region(a), self.region(b));
        if a == b {
            return Ok(());
        }
        if let Region::Var(n) = a {
            if self.is_flexible(Kind::Region, n) {
                self.sub.regions.insert(n, b);
                return Ok(());
            }
        }
        if let Region::Var(n) = b {
            if self.is_flexible(Kind::Region, n) {
                self.sub.regions.insert(n, a);
                return Ok(());
            }
        }
        Err(error(
            f,
            "region-mismatch",
            "Capability regions do not match.",
        ))
    }
    fn bind_effect(&mut self, n: usize, es: &[Effect], f: &Form) -> Checked<()> {
        let es = self.effects(es);
        if es == [Effect::Var(n)] {
            return Ok(());
        }
        if es.contains(&Effect::Var(n)) {
            return Err(error(
                f,
                "recursive-type",
                "Recursive effects require an explicit later-stage contract.",
            ));
        }
        if !self.is_flexible(Kind::Effect, n) {
            return Err(error(
                f,
                "effect-bound",
                "Cannot specialize a universally quantified effect.",
            ));
        }
        self.sub.effects.insert(n, es);
        Ok(())
    }
    pub fn equal_effects(&mut self, a: &[Effect], b: &[Effect], f: &Form) -> Checked<()> {
        let (a, b) = (self.effects(a), self.effects(b));
        if a.iter().collect::<BTreeSet<_>>() == b.iter().collect() {
            return Ok(());
        }
        if let [Effect::Var(n)] = a.as_slice() {
            if self.is_flexible(Kind::Effect, *n) {
                return self.bind_effect(*n, &b, f);
            }
        }
        if let [Effect::Var(n)] = b.as_slice() {
            if self.is_flexible(Kind::Effect, *n) {
                return self.bind_effect(*n, &a, f);
            }
        }
        Err(error(
            f,
            "effect-annotation-required",
            "Cannot solve an effect equation by choosing an arbitrary partition; add annotations.",
        ))
    }
    pub fn include(&mut self, a: &[Effect], b: &[Effect], f: &Form) -> Checked<()> {
        let (a, b) = (self.effects(a), self.effects(b));
        if let [Effect::Var(n)] = b.as_slice() {
            if self.is_flexible(Kind::Effect, *n) {
                return self.bind_effect(*n, &a, f);
            }
        }
        if a.iter().all(|e| b.contains(e)) {
            return Ok(());
        }
        let unresolved = a
            .iter()
            .chain(&b)
            .any(|e| matches!(e,Effect::Var(n) if self.is_flexible(Kind::Effect,*n)));
        Err(error(
            f,
            if unresolved {
                "effect-annotation-required"
            } else {
                "effect-bound"
            },
            "The effect bound does not establish all operations performed here.",
        ))
    }
    pub fn unify(&mut self, a: &Type, b: &Type, f: &Form) -> Checked<()> {
        let (a, b) = (self.resolve(a), self.resolve(b));
        if a == b {
            return Ok(());
        }
        if let Type::Var(n) = a {
            if self.is_flexible(Kind::Type, n) {
                return self.bind_type(n, &b, f);
            }
        }
        if let Type::Var(n) = b {
            if self.is_flexible(Kind::Type, n) {
                return self.bind_type(n, &a, f);
            }
        }
        match (&a, &b) {
            (Type::List(a), Type::List(b)) | (Type::Result(a), Type::Result(b)) => {
                return self.unify(a, b, f)
            }
            (Type::Cap(a, p), Type::Cap(b, q)) if p == q => return self.unify_region(a, b, f),
            (Type::Fn(a, r, e), Type::Fn(b, s, g)) if a.len() == b.len() => {
                for (x, y) in a.iter().zip(b.iter()) {
                    self.unify(x, y, f)?;
                }
                self.unify(r, s, f)?;
                return self.equal_effects(e, g, f);
            }
            _ => {}
        }
        Err(error(
            f,
            "type-mismatch",
            format!("Expected {}, got {}.", self.show(&b), self.show(&a)),
        ))
    }
    pub fn cap_requirement(&mut self, t: &Type, needed: u8, f: &Form) -> Checked<(Region, u8)> {
        let mut raw = t.clone();
        while let Type::Var(n) = raw {
            if let Some(t) = self.sub.types.get(&n) {
                raw = t.clone();
            } else {
                break;
            }
        }
        match raw {
            Type::Var(n) if self.is_flexible(Kind::Type, n) => {
                let r = Region::Var(self.fresh(Kind::Region, None, false).id);
                let p = self.next;
                self.next += 1;
                self.rights.insert(p, needed);
                self.sub
                    .types
                    .insert(n, Type::Cap(r.clone(), Rights::Meta(p)));
                Ok((r, needed))
            }
            Type::Cap(r, Rights::Meta(n)) => {
                *self.rights.get_mut(&n).unwrap() |= needed;
                Ok((r, needed))
            }
            Type::Cap(r, Rights::Fixed(p)) if p & needed == needed => Ok((r, p)),
            Type::Cap(..) => Err(error(
                f,
                "permission-denied",
                "Capability does not grant the required permissions.",
            )),
            _ => Err(error(f, "type-mismatch", "Expected a capability value.")),
        }
    }
    pub fn name(&self, kind: Kind, id: usize) -> String {
        self.names
            .get(&Variable { kind, id })
            .cloned()
            .unwrap_or_else(|| format!("?{id}"))
    }
    pub fn show_region(&self, r: &Region) -> String {
        match self.region(r) {
            Region::Fixed(n) => n,
            Region::Var(n) => self.name(Kind::Region, n),
        }
    }
    pub fn show_effects(&self, es: &[Effect]) -> String {
        let mut out = "(effects".to_string();
        for e in self.effects(es) {
            out.push(' ');
            out.push_str(&match e {
                Effect::Var(n) => self.name(Kind::Effect, n),
                Effect::Fs(r, p) => format!(
                    "(fs.{} {})",
                    rights_names(p).join(" "),
                    self.show_region(&r)
                ),
            });
        }
        out.push(')');
        out
    }
    pub fn show(&self, t: &Type) -> String {
        match self.resolve(t) {
            Type::Unit => "Unit".into(),
            Type::Bool => "Bool".into(),
            Type::Text => "Text".into(),
            Type::Bytes => "Bytes".into(),
            Type::Path => "RelPath".into(),
            Type::Var(n) => self.name(Kind::Type, n),
            Type::Cap(r, p) => format!(
                "(Cap {} ({}))",
                self.show_region(&r),
                rights_names(self.permissions(p)).join(" ")
            ),
            Type::Fn(a, r, e) => format!(
                "(Fn ({}) {} {})",
                a.iter().map(|t| self.show(t)).collect::<Vec<_>>().join(" "),
                self.show(&r),
                self.show_effects(&e)
            ),
            Type::List(t) => format!("(List {})", self.show(&t)),
            Type::Result(t) => format!("(Result {} FsError)", self.show(&t)),
        }
    }
}
