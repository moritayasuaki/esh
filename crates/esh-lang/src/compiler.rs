use crate::contracts::{self, Contracts};
use crate::ir::{Closure, Expr, Node};
use crate::types::*;
use esh_syntax::{parse, Form};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const BUILTINS: &[&str] = &[
    "text.concat",
    "text.eq",
    "fs.read-text",
    "fs.write-text",
    "fs.read-bytes",
    "fs.write-bytes",
    "fs.list",
    "fs.mkdir",
    "fs.remove",
    "utf8.encode",
    "utf8.decode",
    "path.parse",
    "path.join",
    "list.map",
    "list.fold",
    "list.map-ok",
    "list.fold-ok",
];
pub type Environment = BTreeMap<String, usize>;
type Quantifiers = Vec<(String, Variable)>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub text: String,
}
#[derive(Clone)]
struct Module {
    root: Form,
    imports: BTreeMap<String, usize>,
    defs: Vec<(String, Form)>,
    aliases: Vec<(String, Form)>,
    caps: Environment,
    headers: BTreeMap<String, Form>,
    body: Option<Form>,
    exports: BTreeSet<String>,
}
#[derive(Clone, Debug)]
pub struct Binding {
    pub id: usize,
    pub name: String,
    pub scheme: Scheme,
    pub infer: bool,
    pub authority: bool,
    pub pinned: BTreeSet<Variable>,
    pub form: Form,
}
#[derive(Clone)]
struct Term {
    ty: Type,
    effects: Effects,
    node: Node,
    authority: bool,
    pinned: BTreeSet<Variable>,
}
impl Term {
    fn pure(ty: Type, expr: Expr) -> Self {
        Self {
            ty,
            effects: vec![],
            node: Node::new(expr),
            authority: false,
            pinned: BTreeSet::new(),
        }
    }
}
struct ReviewNode {
    form: Form,
    kind: String,
    ty: Type,
    effects: Effects,
    details: Value,
}
#[derive(Debug)]
pub struct Program {
    pub node: Node,
    pub ty: Type,
    pub effects: Effects,
    pub capabilities: Vec<(String, u8)>,
    pub report: Value,
    pub contracts: Contracts,
}
pub struct Compiler {
    pub sources: Vec<Source>,
    modules: Vec<Module>,
    pub solver: Solver,
    pub bindings: Vec<Binding>,
    definitions: BTreeMap<(usize, String), usize>,
    initializers: Vec<(usize, Node)>,
    compiling: Vec<(usize, String)>,
    alias_stack: Vec<(usize, String)>,
    nodes: Vec<ReviewNode>,
    alias_views: Vec<(String, Form, Type)>,
    analysis_depth: usize,
    remaining: usize,
}

fn name(f: &Form) -> Checked<String> {
    match f.atom() {
        Some(s) if !s.contains('.') && !matches!(s, "_" | "true" | "false" | "unit") => {
            Ok(s.into())
        }
        _ => Err(error(
            f,
            "binding",
            "Expected a non-reserved local name without '.'.",
        )),
    }
}
fn fields(f: &Form, allowed: &[&str], offset: usize) -> Checked<BTreeMap<String, Form>> {
    let mut out = BTreeMap::new();
    for x in &f
        .list()
        .ok_or_else(|| error(f, "declaration", "Expected declaration fields."))?[offset..]
    {
        let key = x
            .head()
            .filter(|s| allowed.contains(s))
            .ok_or_else(|| error(x, "declaration", "Unexpected declaration field."))?;
        if out.insert(key.into(), x.clone()).is_some() {
            return Err(error(x, "annotation", format!("Duplicate {key} field.")));
        }
        if key == "export" && x.list().unwrap().len() != 1 {
            return Err(error(x, "export", "Expected (export)."));
        }
    }
    Ok(out)
}
fn one(f: &Form) -> Checked<&Form> {
    let xs = f.list().unwrap();
    if xs.len() != 2 {
        Err(error(
            f,
            "annotation",
            "This annotation requires one value.",
        ))
    } else {
        Ok(&xs[1])
    }
}
fn permissions(f: &Form) -> Checked<u8> {
    let xs = f
        .list()
        .ok_or_else(|| error(f, "permission", "Expected a list of permissions."))?;
    let mut out = 0;
    for x in xs {
        let i = RIGHTS
            .iter()
            .position(|p| x.is_atom(p))
            .ok_or_else(|| error(x, "permission", "Expected read, write, mkdir, or remove."))?;
        let p = 1 << i;
        if out & p != 0 {
            return Err(error(x, "permission", "Duplicate permission."));
        }
        out |= p;
    }
    Ok(out)
}
/// Pure lexical import resolution. The native loader separately enforces descriptor confinement.
pub fn resolve_import(base: &str, path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(|c| c.is_control())
    {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

impl Compiler {
    pub fn new(sources: Vec<Source>) -> Checked<Self> {
        assert!(!sources.is_empty(), "source graph includes an entry");
        let roots: Vec<Form> = sources
            .iter()
            .enumerate()
            .map(|(id, s)| parse(&s.text, id))
            .collect::<Result<_, _>>()?;
        if sources.len() > 128
            || sources.iter().map(|s| s.text.len()).sum::<usize>() > 4 * 1024 * 1024
        {
            return Err(error(
                &roots[0],
                "source-limit",
                "Module graph exceeds 128 files or 4 MiB.",
            ));
        }
        let mut seen = BTreeSet::new();
        for (i, s) in sources.iter().enumerate() {
            if !seen.insert(s.name.clone()) {
                return Err(error(
                    &roots[i],
                    "duplicate-source",
                    "Source names must be unique.",
                ));
            }
        }
        let modules = roots
            .into_iter()
            .map(|root| Module {
                root,
                imports: BTreeMap::new(),
                defs: vec![],
                aliases: vec![],
                caps: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: None,
                exports: BTreeSet::new(),
            })
            .collect();
        let mut c = Self {
            sources,
            modules,
            solver: Solver::default(),
            bindings: vec![],
            definitions: BTreeMap::new(),
            initializers: vec![],
            compiling: vec![],
            alias_stack: vec![],
            nodes: vec![],
            alias_views: vec![],
            analysis_depth: 0,
            remaining: 100_000,
        };
        for id in 0..c.modules.len() {
            c.declarations(id)?;
        }
        for id in 0..c.modules.len() {
            c.check_import_cycle(id, &mut vec![], &mut BTreeSet::new())?;
        }
        Ok(c)
    }
    fn declarations(&mut self, id: usize) -> Checked<()> {
        let root = self.modules[id].root.clone();
        let script = root.has_head("script");
        if !script && !root.has_head("module") {
            return Err(error(
                &root,
                "script",
                "Expected (script ...) or (module name ...).",
            ));
        }
        let xs = root.list().unwrap();
        let offset = if script {
            1
        } else {
            if xs.len() < 2 {
                return Err(error(&root, "module", "A module requires a name."));
            }
            name(&xs[1])?;
            2
        };
        let mut seen = BTreeSet::new();
        for (i, x) in xs.iter().enumerate().skip(offset) {
            match x.head() {
                Some("import")=>{
                    let ys=x.list().unwrap();if ys.len()!=3||ys[2].text().is_none(){return Err(error(x,"import","Expected (import alias \"./file.esh\")."));}
                    let alias=name(&ys[1])?;if !seen.insert(alias.clone()){return Err(error(x,"duplicate-name","Duplicate declaration."));}
                    let target=resolve_import(&self.sources[id].name,ys[2].text().unwrap()).ok_or_else(||error(x,"project-root","Import escapes the project root or is not a relative local path."))?;
                    let mid=self.sources.iter().position(|s|s.name==target).ok_or_else(||error(x,"import-read",format!("Imported source is unavailable: {target}.")))?;
                    if !self.modules[mid].root.has_head("module"){return Err(error(x,"import-module","Only definition-only modules can be imported."));}
                    self.modules[id].imports.insert(alias,mid);
                },
                Some("def"|"type")=>{
                    let ys=x.list().unwrap();if ys.len()<3{return Err(error(x,"declaration","Incomplete definition."));}
                    let n=name(&ys[1])?;if !seen.insert(n.clone()){return Err(error(x,"duplicate-name","Duplicate declaration."));}
                    if ys[2..].iter().any(|f|f.has_head("export")){self.modules[id].exports.insert(n.clone());}
                    if x.has_head("def"){self.modules[id].defs.push((n,x.clone()));}else{self.modules[id].aliases.push((n,x.clone()));}
                },
                Some("cap") if script=>{
                    let ys=x.list().unwrap();if !(3..=4).contains(&ys.len()){return Err(error(x,"capability","Expected (cap name (rights ...) [infer])."));}
                    let n=name(&ys[1])?;if !seen.insert(n.clone()){return Err(error(x,"capability","Duplicate capability or declaration."));}
                    let p=permissions(&ys[2])?;let infer=ys.len()==4;if infer&&!ys[3].is_atom("infer"){return Err(error(x,"capability","Only infer may follow permissions."));}
                    let b=self.binding(n.clone(),Scheme::mono(Type::Cap(Region::Fixed(n.clone()),Rights::Fixed(p))),x,infer,true,BTreeSet::new());
                    self.modules[id].caps.insert(n,b);
                },
                Some("returns"|"effects"|"requires"|"preserves"|"ensures") if script=>{
                    let k=x.head().unwrap().to_owned();if self.modules[id].headers.insert(k.clone(),x.clone()).is_some(){return Err(error(x,"annotation",format!("Duplicate {k} annotation.")));}
                },
                _ if script&&i==xs.len()-1=>self.modules[id].body=Some(x.clone()),
                _=>return Err(error(x,"declaration","Modules contain imports, aliases, and function definitions; scripts finish with one body.")),
            }
        }
        if script && self.modules[id].body.is_none() {
            return Err(error(
                &root,
                "script",
                "A script requires an executable body.",
            ));
        }
        Ok(())
    }
    fn check_import_cycle(
        &self,
        id: usize,
        stack: &mut Vec<usize>,
        done: &mut BTreeSet<usize>,
    ) -> Checked<()> {
        if stack.contains(&id) {
            return Err(error(
                &self.modules[id].root,
                "import-cycle",
                "Cyclic imports are not supported.",
            ));
        }
        if done.contains(&id) {
            return Ok(());
        }
        stack.push(id);
        for child in self.modules[id].imports.values() {
            self.check_import_cycle(*child, stack, done)?;
        }
        stack.pop();
        done.insert(id);
        Ok(())
    }
    fn binding(
        &mut self,
        n: String,
        scheme: Scheme,
        f: &Form,
        infer: bool,
        authority: bool,
        pinned: BTreeSet<Variable>,
    ) -> usize {
        let id = self.bindings.len();
        self.bindings.push(Binding {
            id,
            name: n,
            scheme,
            infer,
            authority,
            pinned,
            form: f.clone(),
        });
        id
    }
    fn quantifiers(&mut self, f: Option<&Form>) -> Checked<Quantifiers> {
        let mut out = vec![];
        if let Some(f) = f {
            for q in &f.list().unwrap()[1..] {
                let xs = q
                    .list()
                    .filter(|x| x.len() == 2)
                    .ok_or_else(|| error(q, "forall", "Expected (name Type|Region|Effect)."))?;
                let n = name(&xs[0])?;
                let k = match xs[1].atom() {
                    Some("Type") => Kind::Type,
                    Some("Region") => Kind::Region,
                    Some("Effect") => Kind::Effect,
                    _ => return Err(error(q, "forall", "Expected Type, Region, or Effect.")),
                };
                if out.iter().any(|(name, _)| *name == n) {
                    return Err(error(q, "forall", "Duplicate quantified name."));
                }
                out.push((n.clone(), self.solver.fresh(k, Some(&n), true)));
            }
        }
        Ok(out)
    }
    fn region_type(&self, f: &Form, m: usize, qs: &Quantifiers) -> Checked<Region> {
        if let Some(n) = f.atom() {
            if let Some((_, v)) = qs.iter().rev().find(|(s, _)| s == n) {
                if v.kind == Kind::Region {
                    return Ok(Region::Var(v.id));
                }
            } else if self.modules[m].caps.contains_key(n) {
                return Ok(Region::Fixed(n.into()));
            }
        }
        Err(error(
            f,
            "region",
            "Expected a declared Region parameter or script capability region.",
        ))
    }
    fn effect_type(&self, f: &Form, m: usize, qs: &Quantifiers) -> Checked<Effects> {
        if !f.has_head("effects") {
            return Err(error(f, "effects", "Expected (effects ...)."));
        }
        let mut out = vec![];
        for x in &f.list().unwrap()[1..] {
            if let Some(n) = x.atom() {
                if let Some((_, v)) = qs
                    .iter()
                    .rev()
                    .find(|(s, v)| s == n && v.kind == Kind::Effect)
                {
                    out.push(Effect::Var(v.id));
                    continue;
                }
            }
            if let Some(xs) = x.list() {
                if xs.len() == 2 {
                    if let Some(p) = xs[0]
                        .atom()
                        .and_then(|s| s.strip_prefix("fs."))
                        .and_then(|s| RIGHTS.iter().position(|p| *p == s))
                    {
                        out.push(Effect::Fs(self.region_type(&xs[1], m, qs)?, 1 << p));
                        continue;
                    }
                }
            }
            return Err(error(
                x,
                "effects",
                "Expected an Effect parameter or (fs.permission Region).",
            ));
        }
        Ok(self.solver.effects(&out))
    }
    fn target(&self, n: &str, m: usize, f: &Form) -> Checked<(usize, String)> {
        if let Some((prefix, n)) = n.split_once('.') {
            let target = *self.modules[m].imports.get(prefix).ok_or_else(|| {
                error(
                    f,
                    "unknown-module",
                    format!("Unknown import alias: {prefix}."),
                )
            })?;
            if !self.modules[target].exports.contains(n) {
                return Err(error(
                    f,
                    "private-name",
                    format!("The imported module does not export {n}."),
                ));
            }
            Ok((target, n.into()))
        } else {
            Ok((m, n.into()))
        }
    }
    fn ty_inner(&mut self, f: &Form, m: usize, qs: &Quantifiers) -> Checked<Type> {
        if let Some(n) = f.atom() {
            let basic = match n {
                "Unit" => Some(Type::Unit),
                "Bool" => Some(Type::Bool),
                "Text" => Some(Type::Text),
                "Bytes" => Some(Type::Bytes),
                "RelPath" => Some(Type::Path),
                _ => None,
            };
            if let Some(t) = basic {
                return Ok(t);
            }
            if let Some((_, v)) = qs.iter().rev().find(|(s, _)| s == n) {
                if v.kind != Kind::Type {
                    return Err(error(f, "kind", "This parameter is not a value type."));
                }
                return Ok(Type::Var(v.id));
            }
            return self.alias_type(n, &[], f, m, qs);
        }
        let xs = f
            .list()
            .filter(|x| !x.is_empty())
            .ok_or_else(|| error(f, "unsupported-type", "Expected a type."))?;
        match (xs[0].atom(), &xs[1..]) {
            (Some("List"), [t]) => Ok(self.ty(t, m, qs)?.list()),
            (Some("Result"), [t, e]) if e.is_atom("FsError") => Ok(self.ty(t, m, qs)?.result()),
            (Some("Cap"), [r, p]) => Ok(Type::Cap(
                self.region_type(r, m, qs)?,
                Rights::Fixed(permissions(p)?),
            )),
            (Some("Fn"), [a, r, e]) => {
                let args = a.list().ok_or_else(|| {
                    error(a, "unsupported-type", "Expected an argument-type list.")
                })?;
                let args = args
                    .iter()
                    .map(|x| self.ty(x, m, qs))
                    .collect::<Result<_, _>>()?;
                Ok(Type::function(
                    args,
                    self.ty(r, m, qs)?,
                    self.effect_type(e, m, qs)?,
                ))
            }
            (Some(n), a) => self.alias_type(n, a, f, m, qs),
            _ => Err(error(f, "unsupported-type", "Expected a type constructor.")),
        }
    }
    fn alias_type(
        &mut self,
        n: &str,
        args: &[Form],
        f: &Form,
        m: usize,
        qs: &Quantifiers,
    ) -> Checked<Type> {
        let (target, n) = self.target(n, m, f)?;
        let key = (target, n.clone());
        if self.alias_stack.contains(&key) {
            return Err(error(
                f,
                "type-cycle",
                "Recursive type aliases are outside this stage.",
            ));
        }
        let declaration = self.modules[target]
            .aliases
            .iter()
            .find(|(s, _)| *s == n)
            .map(|(_, f)| f.clone())
            .ok_or_else(|| error(f, "unsupported-type", format!("Unknown type: {n}.")))?;
        let fs = fields(&declaration, &["export", "forall", "alias"], 2)?;
        let aq = self.quantifiers(fs.get("forall"))?;
        let alias = fs.get("alias").ok_or_else(|| {
            error(
                &declaration,
                "type-alias",
                "A type declaration requires alias.",
            )
        })?;
        if aq.len() != args.len() {
            return Err(error(
                f,
                "type-arity",
                "Wrong number of type-alias arguments.",
            ));
        }
        let mut subst = Substitution::default();
        for ((_, v), a) in aq.iter().zip(args) {
            match v.kind {
                Kind::Type => {
                    subst.types.insert(v.id, self.ty(a, m, qs)?);
                }
                Kind::Region => {
                    subst.regions.insert(v.id, self.region_type(a, m, qs)?);
                }
                Kind::Effect => {
                    subst.effects.insert(v.id, self.effect_type(a, m, qs)?);
                }
            }
        }
        self.alias_stack.push(key);
        let t = self.ty(one(alias)?, target, &aq)?.subst(&subst);
        self.alias_stack.pop();
        self.alias_views.push((n, f.clone(), t.clone()));
        Ok(t)
    }
    fn generalize(&mut self, t: &Term, env: &Environment, explicit: &[Variable]) -> Scheme {
        let ty = self.solver.resolve(&t.ty);
        let mut pinned = t.pinned.clone();
        for id in env.values() {
            let b = &self.bindings[*id];
            pinned.extend(
                self.solver
                    .resolve(&b.scheme.ty)
                    .free()
                    .difference(&b.scheme.quant.iter().copied().collect())
                    .copied(),
            );
            pinned.extend(&b.pinned);
        }
        let mut quant: BTreeSet<Variable> = explicit.iter().copied().collect();
        if self.solver.effects(&t.effects).is_empty() {
            quant.extend(
                ty.free()
                    .difference(&pinned)
                    .filter(|v| !self.solver.rigid.contains(v))
                    .copied(),
            );
        }
        self.solver.rigid.extend(&quant);
        Scheme {
            ty,
            quant: quant.into_iter().collect(),
        }
    }
    fn instantiate(&mut self, id: usize) -> Term {
        let b = self.bindings[id].clone();
        let mut s = Substitution::default();
        for v in &b.scheme.quant {
            let fresh = self.solver.fresh(v.kind, None, false);
            match v.kind {
                Kind::Type => {
                    s.types.insert(v.id, Type::Var(fresh.id));
                }
                Kind::Region => {
                    s.regions.insert(v.id, Region::Var(fresh.id));
                }
                Kind::Effect => {
                    s.effects.insert(v.id, vec![Effect::Var(fresh.id)]);
                }
            }
        }
        let mut node = Node::reference(id);
        if !b.scheme.quant.is_empty() {
            node = Node::new(Expr::Instantiate {
                value: Box::new(node),
                substitution: s.clone(),
            });
        }
        Term {
            ty: b.scheme.ty.subst(&s),
            effects: vec![],
            node,
            authority: b.authority,
            pinned: b.pinned,
        }
    }
    fn record(&mut self, f: &Form, kind: &str, mut t: Term, details: Value) -> Term {
        self.nodes.push(ReviewNode {
            form: f.clone(),
            kind: kind.into(),
            ty: t.ty.clone(),
            effects: t.effects.clone(),
            details,
        });
        if t.node.location.is_none() {
            t.node = t.node.located(f, kind);
        }
        t
    }

    fn builtin(&mut self, n: &str, f: &Form) -> Checked<usize> {
        let key = (usize::MAX, n.into());
        if let Some(id) = self.definitions.get(&key) {
            return Ok(*id);
        }
        let mut quant = vec![];
        let (args, ret, effects) = match n {
            "text.concat" => (vec![Type::Text, Type::Text], Type::Text, vec![]),
            "text.eq" => (vec![Type::Text, Type::Text], Type::Bool, vec![]),
            "utf8.encode" => (vec![Type::Text], Type::Bytes, vec![]),
            "utf8.decode" => (vec![Type::Bytes], Type::Text.result(), vec![]),
            "path.parse" => (vec![Type::Text], Type::Path.result(), vec![]),
            "path.join" => (vec![Type::Path, Type::Text], Type::Path.result(), vec![]),
            n if n.starts_with("fs.") => {
                let r = self.solver.fresh(Kind::Region, Some("R"), true);
                quant.push(r);
                let region = Region::Var(r.id);
                let p = match n {
                    "fs.read-text" | "fs.read-bytes" | "fs.list" => 1,
                    "fs.write-text" | "fs.write-bytes" => 2,
                    "fs.mkdir" => 4,
                    "fs.remove" => 8,
                    _ => unreachable!(),
                };
                let mut args = vec![Type::Cap(region.clone(), Rights::Fixed(p)), Type::Path];
                if p == 2 {
                    args.push(if n == "fs.write-bytes" {
                        Type::Bytes
                    } else {
                        Type::Text
                    });
                }
                let ret = match n {
                    "fs.read-text" => Type::Text,
                    "fs.read-bytes" => Type::Bytes,
                    "fs.list" => Type::Text.list(),
                    _ => Type::Unit,
                };
                (args, ret.result(), vec![Effect::Fs(region, p)])
            }
            n if n.starts_with("list.") => {
                let a = self.solver.fresh(Kind::Type, Some("A"), true);
                let b = self.solver.fresh(Kind::Type, Some("B"), true);
                let e = self.solver.fresh(Kind::Effect, Some("E"), true);
                quant.extend([a, b, e]);
                let a = Type::Var(a.id);
                let b = Type::Var(b.id);
                let eff = vec![Effect::Var(e.id)];
                let fold = n.contains("fold");
                let stop = n.ends_with("-ok");
                let cb_args = if fold {
                    vec![b.clone(), a.clone()]
                } else {
                    vec![a.clone()]
                };
                let cb_ret = if stop { b.clone().result() } else { b.clone() };
                let mut args = vec![Type::function(cb_args, cb_ret, eff.clone())];
                if fold {
                    args.push(b.clone());
                }
                args.push(a.list());
                let ret = if fold { b } else { b.list() };
                (args, if stop { ret.result() } else { ret }, eff)
            }
            _ => return Err(error(f, "unknown-variable", "Unknown built-in function.")),
        };
        let params: Vec<usize> = args
            .iter()
            .enumerate()
            .map(|(i, t)| {
                self.binding(
                    format!("arg{i}"),
                    Scheme::mono(t.clone()),
                    f,
                    false,
                    t.authority(),
                    BTreeSet::new(),
                )
            })
            .collect();
        let body = Node::new(Expr::Primitive {
            name: n.into(),
            args: params.iter().map(|n| Node::reference(*n)).collect(),
        });
        let ty = Type::function(args.clone(), ret.clone(), effects.clone());
        let node = Node::new(Expr::Function(Closure {
            params,
            param_types: args,
            result: ret,
            effects,
            capture_ids: vec![],
            capture_types: vec![],
            captures: vec![],
            body: Box::new(body),
        }));
        let id = self.binding(
            n.into(),
            Scheme { ty, quant },
            f,
            false,
            false,
            BTreeSet::new(),
        );
        self.definitions.insert(key, id);
        self.initializers.push((id, node));
        Ok(id)
    }
    fn lookup(&mut self, n: &str, m: usize, env: &Environment, f: &Form) -> Checked<usize> {
        if let Some(id) = env.get(n) {
            return Ok(*id);
        }
        if BUILTINS.contains(&n) {
            return self.builtin(n, f);
        }
        let (target, n) = self.target(n, m, f)?;
        if !self.modules[target].defs.iter().any(|(s, _)| *s == n) {
            return Err(error(
                f,
                "unknown-variable",
                format!("Unknown variable or function: {n}."),
            ));
        }
        self.compile_def(target, &n)
    }
    fn compile_def(&mut self, m: usize, n: &str) -> Checked<usize> {
        let key = (m, n.into());
        if let Some(id) = self.definitions.get(&key) {
            return Ok(*id);
        }
        let f = self.modules[m]
            .defs
            .iter()
            .find(|(s, _)| s == n)
            .unwrap()
            .1
            .clone();
        if self.compiling.contains(&key) {
            return Err(error(
                &f,
                "definition-cycle",
                "Recursive definition cycles are deferred.",
            ));
        }
        self.compiling.push(key.clone());
        let env = self.modules[m].caps.clone();
        let (term, explicit, mut details) = self.function(&f, m, &env, &vec![], true)?;
        let scheme = self.generalize(&term, &env, &explicit);
        let id = self.binding(
            n.into(),
            scheme.clone(),
            &f,
            false,
            term.authority,
            term.pinned.clone(),
        );
        self.definitions.insert(key, id);
        self.initializers.push((id, term.node.clone()));
        details["name"] = json!(n);
        details["quantifiers"] = json!(scheme.quant);
        details["exported"] = json!(self.modules[m].exports.contains(n));
        self.record(&f, "definition", term, details);
        self.compiling.pop();
        Ok(id)
    }
    fn coerce(&mut self, mut term: Term, wanted: &Type, f: &Form) -> Checked<Term> {
        let target = self.solver.resolve(wanted);
        if let Type::Cap(r, p) = &target {
            let required = self.solver.permissions(*p);
            let (region, granted) = self.solver.cap_requirement(&term.ty, required, f)?;
            self.solver.unify_region(&region, r, f)?;
            term.node = Node::new(Expr::Narrow {
                value: Box::new(term.node),
                rights: required,
            });
            term.authority = true;
            if granted != required {
                self.nodes.push(ReviewNode{form:f.clone(),kind:"narrow".into(),ty:wanted.clone(),effects:vec![],details:json!({"granted":rights_names(granted),"required":rights_names(required),"narrowed":true})});
            }
        } else if let (Type::Fn(a, r, e), Type::Fn(b, s, g)) =
            (self.solver.resolve(&term.ty), &target)
        {
            if a.len() != b.len() {
                return Err(error(f, "arity", "Function arities do not match."));
            }
            for (x, y) in a.iter().zip(b.iter()) {
                self.solver.unify(x, y, f)?;
            }
            self.solver.unify(&r, s, f)?;
            self.solver.include(&e, g, f)?;
            term.node = Node::new(Expr::Widen {
                value: Box::new(term.node),
                effects: g.clone(),
            });
        } else {
            self.solver.unify(&term.ty, wanted, f)?;
        }
        term.ty = wanted.clone();
        Ok(term)
    }
    fn value_form(f: &Form) -> Checked<()> {
        if f.list().is_none() || f.has_head("fn") {
            Ok(())
        } else {
            Err(error(f,"evaluation-order","Bind this computation with let/let-ok before using it as a callee or argument, including pure calls."))
        }
    }
    fn function(
        &mut self,
        f: &Form,
        m: usize,
        env: &Environment,
        outer: &Quantifiers,
        named: bool,
    ) -> Checked<(Term, Vec<Variable>, Value)> {
        let fs = fields(
            f,
            &[
                "export", "forall", "params", "returns", "effects", "body", "captures",
            ],
            if named { 2 } else { 1 },
        )?;
        let parameters = fs
            .get("params")
            .ok_or_else(|| error(f, "function", "A function requires params."))?;
        let body_form = fs
            .get("body")
            .ok_or_else(|| error(f, "function", "A function requires body."))?;
        if !named && (fs.contains_key("export") || fs.contains_key("forall")) {
            return Err(error(f,"rank-1","Anonymous functions infer schemes at their binding; export/forall belong to named definitions."));
        }
        let explicit = self.quantifiers(fs.get("forall"))?;
        let mut qs = outer.clone();
        qs.extend(explicit.clone());
        let mut scope = env.clone();
        let mut names = BTreeSet::new();
        let mut params = vec![];
        let public = fs.contains_key("export");
        for p in &parameters.list().unwrap()[1..] {
            let (n, ty, infer) = if p.atom().is_some() {
                if public {
                    return Err(error(
                        p,
                        "public-signature",
                        "Every exported parameter requires a type.",
                    ));
                }
                (name(p)?, self.solver.fresh_type(), false)
            } else {
                let xs = p
                    .list()
                    .filter(|x| (2..=3).contains(&x.len()))
                    .ok_or_else(|| {
                        error(p, "parameter", "Expected name or (name Type [infer]).")
                    })?;
                let n = name(&xs[0])?;
                let ty = self.ty(&xs[1], m, &qs)?;
                let infer = xs.len() == 3;
                if infer && (!xs[2].is_atom("infer") || !matches!(ty, Type::Cap(..))) {
                    return Err(error(
                        p,
                        "capability",
                        "infer marks an annotated capability parameter.",
                    ));
                }
                (n, ty, infer)
            };
            if !names.insert(n.clone()) {
                return Err(error(p, "duplicate-name", "Duplicate parameter."));
            }
            let id = self.binding(
                n.clone(),
                Scheme::mono(ty.clone()),
                p,
                infer,
                ty.authority(),
                BTreeSet::new(),
            );
            params.push(id);
            scope.insert(n, id);
        }
        if public && (!fs.contains_key("returns") || !fs.contains_key("effects")) {
            return Err(error(
                f,
                "public-signature",
                "Exported functions require returns and effects annotations.",
            ));
        }
        let mut body = self.expression(one(body_form)?, m, &scope, &qs)?;
        if let Some(ret) = fs.get("returns") {
            let ty = self.ty(one(ret)?, m, &qs)?;
            body = self.coerce(body, &ty, ret)?;
        }
        let bound = if let Some(e) = fs.get("effects") {
            let e_ty = self.effect_type(e, m, &qs)?;
            self.solver.include(&body.effects, &e_ty, e)?;
            e_ty
        } else {
            body.effects.clone()
        };
        let used: BTreeSet<_> = body
            .node
            .free_refs()
            .difference(&params.iter().copied().collect())
            .copied()
            .collect();
        let authority: BTreeSet<_> = used
            .iter()
            .filter(|id| {
                let b = &self.bindings[**id];
                b.authority && self.solver.resolve(&b.scheme.ty).authority()
            })
            .copied()
            .collect();
        let declaration = fs.get("captures");
        let inferred = declaration
            .is_some_and(|d| d.list().unwrap().len() == 2 && d.list().unwrap()[1].is_atom("_"));
        let mut declared = BTreeSet::new();
        if let Some(d) = declaration.filter(|_| !inferred) {
            for capture in &d.list().unwrap()[1..] {
                let n = capture.atom().filter(|s| *s != "_").ok_or_else(|| {
                    error(
                        capture,
                        "captures",
                        "Use a list of names or the single marker (captures _).",
                    )
                })?;
                let id = self.lookup(n, m, env, capture)?;
                if !declared.insert(id) {
                    return Err(error(capture, "captures", "Duplicate capture declaration."));
                }
            }
        }
        let missing: Vec<_> = authority
            .difference(&declared)
            .map(|id| self.bindings[*id].name.clone())
            .collect();
        if !inferred && !missing.is_empty() {
            return Err(error(
                f,
                "capture-required",
                format!(
                    "Declare authority captures: {}; or use (captures _).",
                    missing.join(", ")
                ),
            ));
        }
        let (mut captures, mut capture_ids, mut capture_types, mut retained, mut pinned) =
            (vec![], vec![], vec![], vec![], BTreeSet::new());
        for id in used {
            let b = self.bindings[id].clone();
            let mut ty = self.solver.resolve(&b.scheme.ty);
            let mut value = Node::reference(id);
            let mut cid = id;
            let mut note = json!({"name":b.name,"authority":authority.contains(&id),"inferred":inferred&&authority.contains(&id)});
            if let Type::Cap(r, p) = &ty {
                let granted = self.solver.permissions(*p);
                let needed = body.node.capture_rights(id, granted);
                note["region"] = json!(r);
                note["granted"] = json!(rights_names(granted));
                note["required"] = json!(rights_names(needed));
                note["narrowed"] = json!(needed != granted);
                if needed != granted {
                    ty = Type::Cap(r.clone(), Rights::Fixed(needed));
                    cid = self.binding(
                        b.name.clone(),
                        Scheme::mono(ty.clone()),
                        &b.form,
                        b.infer,
                        true,
                        b.pinned.clone(),
                    );
                    body.node.replace_ref(id, cid);
                    value = Node::new(Expr::Narrow {
                        value: Box::new(value),
                        rights: needed,
                    });
                }
            } else if authority.contains(&id) {
                note["retainedAuthority"] =
                    json!("potential; not established by the invocation effect bound");
            }
            pinned.extend(
                ty.free()
                    .difference(&b.scheme.quant.iter().copied().collect())
                    .copied(),
            );
            pinned.extend(&b.pinned);
            captures.push(value);
            capture_ids.push(cid);
            capture_types.push(ty);
            retained.push(note);
        }
        let args = params
            .iter()
            .map(|id| self.bindings[*id].scheme.ty.clone())
            .collect::<Vec<_>>();
        let ty = Type::function(args.clone(), body.ty.clone(), bound.clone());
        let node = Node::new(Expr::Function(Closure {
            params,
            param_types: args,
            result: body.ty,
            effects: bound.clone(),
            capture_ids,
            capture_types,
            captures,
            body: Box::new(body.node),
        }));
        let details = json!({"captures":retained,"captureInference":inferred,"invocationEffects":bound,"creationEffects":[],"arity":names.len()});
        Ok((
            Term {
                ty,
                effects: vec![],
                node,
                authority: !authority.is_empty(),
                pinned,
            },
            explicit.iter().map(|(_, v)| *v).collect(),
            details,
        ))
    }
    fn enter_analysis(&mut self, f: &Form) -> Checked<()> {
        if self.analysis_depth >= 128 || self.remaining == 0 {
            return Err(error(
                f,
                "analysis-limit",
                "Analysis exceeded its depth or work limit.",
            ));
        }
        self.analysis_depth += 1;
        self.remaining -= 1;
        Ok(())
    }
    fn expression(
        &mut self,
        f: &Form,
        m: usize,
        env: &Environment,
        qs: &Quantifiers,
    ) -> Checked<Term> {
        self.enter_analysis(f)?;
        let result = self.expression_inner(f, m, env, qs);
        self.analysis_depth -= 1;
        let term = result?;
        if !self.solver.resolve(&term.ty).within_limit() {
            return Err(error(
                f,
                "type-limit",
                "Inferred type exceeds the structural limit; introduce smaller interfaces.",
            ));
        }
        Ok(term)
    }
    fn block(
        &mut self,
        forms: &[Form],
        m: usize,
        env: &Environment,
        qs: &Quantifiers,
    ) -> Checked<Term> {
        self.enter_analysis(&forms[0])?;
        let result = self.block_inner(forms, m, env, qs);
        self.analysis_depth -= 1;
        result
    }
    fn ty(&mut self, f: &Form, m: usize, qs: &Quantifiers) -> Checked<Type> {
        self.enter_analysis(f)?;
        let result = self.ty_inner(f, m, qs);
        self.analysis_depth -= 1;
        let ty = result?;
        if !ty.within_limit() {
            return Err(error(f, "type-limit", "Type exceeds the structural limit."));
        }
        Ok(ty)
    }
    fn expression_inner(
        &mut self,
        f: &Form,
        m: usize,
        env: &Environment,
        qs: &Quantifiers,
    ) -> Checked<Term> {
        if let Some(text) = f.text() {
            return Ok(self.record(
                f,
                "literal",
                Term::pure(Type::Text, Expr::Text(text.into())),
                json!({}),
            ));
        }
        if let Some(n) = f.atom() {
            let t = match n {
                "true" | "false" => Term::pure(Type::Bool, Expr::Bool(n == "true")),
                "unit" => Term::pure(Type::Unit, Expr::Unit),
                _ => {
                    let id = self.lookup(n, m, env, f)?;
                    self.instantiate(id)
                }
            };
            return Ok(self.record(f, "value", t, json!({"name":n})));
        }
        let xs = f.list().unwrap();
        if xs.is_empty() {
            return Err(error(f, "expression", "Empty application."));
        }
        let (kind, t, details) = match f.head() {
            Some("loop" | "lambda" | "proc.run" | "parallel" | "spawn" | "await") => {
                return Err(error(
                    f,
                    "unsupported-form",
                    "This form is outside the current implementation stage.",
                ))
            }
            Some("fn") => {
                let (t, _, d) = self.function(f, m, env, qs, false)?;
                ("fn", t, d)
            }
            Some("do") => {
                if xs.len() < 2 {
                    return Err(error(f, "do", "A do block needs a final expression."));
                }
                return self.block(&xs[1..], m, env, qs);
            }
            Some("let" | "let-ok") => {
                if xs.len() != 4 {
                    return Err(error(
                        f,
                        "binding",
                        "Use (let name value body), or a binding step inside do.",
                    ));
                }
                return self.let_expr(
                    f.has_head("let-ok"),
                    &xs[1],
                    &xs[2],
                    &xs[3..],
                    f,
                    m,
                    env,
                    qs,
                );
            }
            Some("if") => {
                if xs.len() != 4 {
                    return Err(error(
                        f,
                        "arity",
                        "if requires a condition and two branches.",
                    ));
                }
                Self::value_form(&xs[1])?;
                for b in &xs[2..] {
                    if b.list().is_some()
                        && !matches!(b.head(), Some("fn" | "do" | "let" | "let-ok"))
                    {
                        return Err(error(
                            b,
                            "evaluation-order",
                            "Put branch computations inside their own do block.",
                        ));
                    }
                }
                let c = self.expression(&xs[1], m, env, qs)?;
                let c = self.coerce(c, &Type::Bool, &xs[1])?;
                let mut a = self.expression(&xs[2], m, env, qs)?;
                let mut b = self.expression(&xs[3], m, env, qs)?;
                let target = match (self.solver.resolve(&a.ty), self.solver.resolve(&b.ty)) {
                    (Type::Fn(args, r, e), Type::Fn(other, _, g)) if args.len() == other.len() => {
                        Type::Fn(args, r, union(e.into_iter().chain(g)))
                    }
                    (Type::Cap(r, p), Type::Cap(_, q)) => Type::Cap(
                        r,
                        Rights::Fixed(self.solver.permissions(p) & self.solver.permissions(q)),
                    ),
                    _ => a.ty.clone(),
                };
                a = self.coerce(a, &target, &xs[2])?;
                b = self.coerce(b, &target, &xs[3])?;
                let effects = union(c.effects.into_iter().chain(a.effects).chain(b.effects));
                let pinned = a.pinned.union(&b.pinned).copied().collect();
                let authority = a.authority || b.authority;
                (
                    "if",
                    Term {
                        ty: target,
                        effects,
                        node: Node::new(Expr::If {
                            condition: Box::new(c.node),
                            yes: Box::new(a.node),
                            no: Box::new(b.node),
                        }),
                        authority,
                        pinned,
                    },
                    json!({"branches":"alternatives; only the selected branch executes"}),
                )
            }
            Some("path") => {
                if xs.len() != 2 || xs[1].text().is_none() {
                    return Err(error(f, "path", "path requires a literal relative path."));
                }
                let text = xs[1].text().unwrap();
                if !esh_policy::valid_relative_path(text.as_bytes()) {
                    return Err(error(f,"path","Expected a relative path without traversal, empty, backslash, or control components."));
                }
                (
                    "path",
                    Term::pure(Type::Path, Expr::Path(text.into())),
                    json!({}),
                )
            }
            Some("ok") => {
                if xs.len() != 2 {
                    return Err(error(f, "arity", "ok requires one value."));
                }
                Self::value_form(&xs[1])?;
                let v = self.expression(&xs[1], m, env, qs)?;
                (
                    "ok",
                    Term {
                        ty: v.ty.result(),
                        effects: v.effects,
                        node: Node::new(Expr::Ok(Box::new(v.node))),
                        authority: v.authority,
                        pinned: v.pinned,
                    },
                    json!({}),
                )
            }
            Some("error") => {
                if xs.len() != 3 || xs[2].text().is_none() {
                    return Err(error(f, "error", "Expected (error Type \"message\")."));
                }
                let t = self.ty(&xs[1], m, qs)?;
                (
                    "error",
                    Term::pure(
                        t.clone().result(),
                        Expr::Error(t, xs[2].text().unwrap().into()),
                    ),
                    json!({}),
                )
            }
            Some("list") => {
                if xs.len() < 2 {
                    return Err(error(f, "list", "Expected (list ElementType value ...)."));
                }
                let t = self.ty(&xs[1], m, qs)?;
                let mut values = vec![];
                let mut pinned = BTreeSet::new();
                let mut authority = false;
                for x in &xs[2..] {
                    Self::value_form(x)?;
                    let v = self.expression(x, m, env, qs)?;
                    let v = self.coerce(v, &t, x)?;
                    authority |= v.authority;
                    pinned.extend(v.pinned);
                    values.push(v.node);
                }
                (
                    "list",
                    Term {
                        ty: t.clone().list(),
                        effects: vec![],
                        node: Node::new(Expr::List(t, values)),
                        authority,
                        pinned,
                    },
                    json!({"order":"declaration order"}),
                )
            }
            _ => return self.call(f, m, env, qs),
        };
        Ok(self.record(f, kind, t, details))
    }
    fn block_inner(
        &mut self,
        forms: &[Form],
        m: usize,
        env: &Environment,
        qs: &Quantifiers,
    ) -> Checked<Term> {
        if forms.len() == 1 {
            return self.expression(&forms[0], m, env, qs);
        }
        let f = &forms[0];
        if !matches!(f.head(), Some("let" | "let-ok")) || f.list().unwrap().len() != 3 {
            return Err(error(
                f,
                "evaluation-order",
                "Every non-final computation in do needs a let or let-ok binding.",
            ));
        }
        let xs = f.list().unwrap();
        self.let_expr(
            f.has_head("let-ok"),
            &xs[1],
            &xs[2],
            &forms[1..],
            f,
            m,
            env,
            qs,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn let_expr(
        &mut self,
        short: bool,
        binder: &Form,
        value: &Form,
        rest: &[Form],
        f: &Form,
        m: usize,
        env: &Environment,
        qs: &Quantifiers,
    ) -> Checked<Term> {
        let n = name(binder)?;
        if self.modules[m].caps.contains_key(&n) {
            return Err(error(
                binder,
                "binding",
                "A local binding cannot shadow a script capability.",
            ));
        }
        let v = self.expression(value, m, env, qs)?;
        let mut bound = v.clone();
        if short {
            bound.ty = self.result_element(&v.ty, value)?;
        }
        let scheme = self.generalize(&bound, env, &[]);
        let id = self.binding(
            n.clone(),
            scheme.clone(),
            binder,
            false,
            bound.authority,
            bound.pinned,
        );
        let mut scope = env.clone();
        scope.insert(n.clone(), id);
        let body = self.block(rest, m, &scope, qs)?;
        if short {
            self.result_element(&body.ty, f)?;
        }
        let effects = union(v.effects.clone().into_iter().chain(body.effects));
        let ty = body.ty.clone();
        let term = Term {
            ty,
            effects,
            node: Node::new(Expr::Let {
                binding: id,
                value: Box::new(v.node),
                body: Box::new(body.node),
                short_circuit: short,
            }),
            authority: body.authority,
            pinned: body.pinned,
        };
        let details = json!({"name":n,"quantifiers":scheme.quant,"evaluation":"once, before the remaining block","onError":if short{Some("return Err from this block; skip remaining steps")}else{None},"blockResult":body.ty});
        let out = self.record(f, if short { "let-ok" } else { "let" }, term, details);
        let row = self.nodes.last_mut().unwrap();
        row.ty = scheme.ty;
        row.effects = v.effects;
        Ok(out)
    }
    fn result_element(&mut self, t: &Type, f: &Form) -> Checked<Type> {
        let mut t = self.solver.resolve(t);
        if let Type::Var(n) = t {
            if self.solver.is_flexible(Kind::Type, n) {
                let fresh = self.solver.fresh_type();
                self.solver.bind_type(n, &fresh.result(), f)?;
                t = self.solver.resolve(&Type::Var(n));
            }
        }
        if let Type::Result(t) = t {
            Ok(t.as_ref().clone())
        } else {
            Err(error(
                f,
                "type-mismatch",
                "let-ok requires a Result value and a Result remainder.",
            ))
        }
    }
    fn call(&mut self, f: &Form, m: usize, env: &Environment, qs: &Quantifiers) -> Checked<Term> {
        let xs = f.list().unwrap();
        Self::value_form(&xs[0])?;
        let fun = self.expression(&xs[0], m, env, qs)?;
        let mut ty = self.solver.resolve(&fun.ty);
        if let Type::Var(n) = ty {
            if self.solver.is_flexible(Kind::Type, n) {
                let args = (1..xs.len()).map(|_| self.solver.fresh_type()).collect();
                let ret = self.solver.fresh_type();
                let eff = Effect::Var(self.solver.fresh(Kind::Effect, None, false).id);
                ty = Type::function(args, ret, vec![eff]);
                self.solver.bind_type(n, &ty, &xs[0])?;
            }
        }
        let Type::Fn(params, ret, latent) = ty else {
            return Err(error(
                &xs[0],
                "not-callable",
                "The callee is not a function.",
            ));
        };
        if params.len() != xs.len() - 1 {
            return Err(error(
                f,
                "arity",
                "Supply every argument; use an explicit fn for partial application.",
            ));
        }
        let mut args: Vec<Option<Term>> = vec![None; params.len()];
        let mut originals = vec![None; params.len()];
        let mut resolutions = vec![];
        for (i, (source, expected)) in xs[1..].iter().zip(params.iter()).enumerate() {
            if source.is_atom("_") {
                continue;
            }
            Self::value_form(source)?;
            let arg = self.expression(source, m, env, qs)?;
            originals[i] = Some(arg.ty.clone());
            args[i] = Some(self.coerce(arg, expected, source)?);
        }
        for (i, (source, expected)) in xs[1..].iter().zip(params.iter()).enumerate() {
            if args[i].is_some() {
                continue;
            }
            let expected = self.solver.resolve(expected);
            let Type::Cap(r, p) = &expected else {
                return Err(error(
                    source,
                    "capability",
                    "_ requires a known capability parameter; add an annotation.",
                ));
            };
            let needed = self.solver.permissions(*p);
            let candidates:Vec<_>=env.values().filter(|id|{let b=&self.bindings[**id];if !b.infer{return false;}match self.solver.resolve(&b.scheme.ty){Type::Cap(br,bp)=>self.solver.permissions(bp)&needed==needed&&(r==&br||matches!(r,Region::Var(n) if self.solver.is_flexible(Kind::Region,*n))),_=>false}}).copied().collect();
            if candidates.len() != 1 {
                return Err(error(
                    source,
                    if candidates.is_empty() {
                        "capability-missing"
                    } else {
                        "capability-ambiguous"
                    },
                    "Capability inference requires exactly one eligible opt-in candidate.",
                ));
            }
            let id = candidates[0];
            originals[i] = Some(self.bindings[id].scheme.ty.clone());
            let arg = self.instantiate(id);
            args[i] = Some(self.coerce(arg, &expected, source)?);
            resolutions.push(
                json!({"argument":i+1,"name":self.bindings[id].name,"region":r,"inferred":true}),
            );
        }
        let args: Vec<Term> = args.into_iter().map(Option::unwrap).collect();
        let effects = union(
            fun.effects
                .iter()
                .cloned()
                .chain(args.iter().flat_map(|a| a.effects.clone()))
                .chain(latent.clone()),
        );
        let ret = self.solver.resolve(&ret);
        let mut pinned = fun.pinned;
        for a in &args {
            pinned.extend(&a.pinned);
        }
        let authority = ret.authority();
        let n = xs[0].atom().unwrap_or("anonymous function");
        let mut details = json!({"callee":n,"arguments":resolutions,"invocations":1,"internalOrder":"not specified by the permission contract","internalCount":"unknown from the permission contract"});
        if BUILTINS.contains(&n) && n.starts_with("fs.") {
            if let (Type::Cap(r, p), Type::Cap(_, q)) = (
                self.solver.resolve(&args[0].ty),
                self.solver.resolve(originals[0].as_ref().unwrap()),
            ) {
                let p = self.solver.permissions(p);
                let q = self.solver.permissions(q);
                details["region"] = json!(r);
                details["inferred"] = json!(xs[1].is_atom("_"));
                details["granted"] = json!(rights_names(q));
                details["required"] = json!(rights_names(p));
                details["narrowed"] = json!(p != q);
            }
        }
        let node = Node::new(Expr::Call {
            function: Box::new(fun.node),
            args: args.into_iter().map(|a| a.node).collect(),
            effects: latent,
        });
        Ok(self.record(
            f,
            if BUILTINS.contains(&n) { n } else { "call" },
            Term {
                ty: ret,
                effects,
                node,
                authority,
                pinned,
            },
            details,
        ))
    }
    fn effects_report(&self, es: &[Effect]) -> Value {
        json!(self.solver.effects(es).iter().map(|e|match e{Effect::Var(n)=>json!({"operation":self.solver.name(Kind::Effect,*n),"region":"polymorphic","variable":true}),Effect::Fs(r,p)=>json!({"operation":format!("fs.{}",rights_names(*p).join(" ")),"region":self.solver.show_region(r)})}).collect::<Vec<_>>())
    }
    fn details_report(&self, value: &Value) -> Value {
        match value {
            Value::Array(xs) => json!(xs
                .iter()
                .map(|v| self.details_report(v))
                .collect::<Vec<_>>()),
            Value::Object(o) => {
                let mut out = serde_json::Map::new();
                for (k, v) in o {
                    let v=match k.as_str(){
                        "region"=>serde_json::from_value::<Region>(v.clone()).map(|r|json!(self.solver.show_region(&r))).unwrap_or_else(|_|v.clone()),
                        "invocationEffects"|"creationEffects"=>serde_json::from_value::<Effects>(v.clone()).map(|e|self.effects_report(&e)).unwrap_or_else(|_|v.clone()),
                        "blockResult"=>serde_json::from_value::<Type>(v.clone()).map(|t|json!(self.solver.show(&t))).unwrap_or_else(|_|v.clone()),
                        "quantifiers"=>serde_json::from_value::<Vec<Variable>>(v.clone()).map(|vs|json!(vs.iter().map(|v|json!({"name":self.solver.name(v.kind,v.id),"kind":format!("{:?}",v.kind)})).collect::<Vec<_>>())).unwrap_or_else(|_|v.clone()),
                        _=>self.details_report(v),
                    };
                    out.insert(k.clone(), v);
                }
                Value::Object(out)
            }
            _ => value.clone(),
        }
    }
    fn structure(&self, node: &Node) -> Value {
        if let Expr::Instantiate { value, .. }
        | Expr::Narrow { value, .. }
        | Expr::Widen { value, .. } = &node.expr
        {
            return self.structure(value);
        }
        let mut out =
            json!({"kind":node.location.as_ref().map(|l|l.kind.as_str()).unwrap_or("value")});
        if let Some(l) = &node.location {
            out["source"] = json!(self.sources[l.source].name);
            out["span"] = json!(l.span);
        }
        match &node.expr {
            Expr::Let {
                binding,
                value,
                body,
                short_circuit,
            } => {
                out["kind"] = json!(if *short_circuit { "let-ok" } else { "let" });
                out["binding"] = json!(self.bindings[*binding].name);
                out["value"] = self.structure(value);
                out["next"] = self.structure(body);
            }
            Expr::If { condition, yes, no } => {
                out["condition"] = self.structure(condition);
                out["yes"] = self.structure(yes);
                out["no"] = self.structure(no);
            }
            Expr::Call {
                function,
                args,
                effects,
            } => {
                out["callee"] = self.structure(function);
                out["arguments"] =
                    json!(args.iter().map(|n| self.structure(n)).collect::<Vec<_>>());
                out["invocations"] = json!(1);
                out["effects"] = self.effects_report(effects);
                out["callbackInternals"] = json!(if self.solver.effects(effects).is_empty() {
                    "no filesystem requests"
                } else {
                    "order/count unknown from permission bound alone"
                });
            }
            Expr::Function(c) => out["bodyOnInvocation"] = self.structure(&c.body),
            Expr::Ref(n) => out["name"] = json!(self.bindings[*n].name),
            _ => {}
        }
        out
    }
    pub fn build(mut self) -> Checked<Program> {
        for m in 0..self.modules.len() {
            for (n, _) in self.modules[m].defs.clone() {
                self.compile_def(m, &n)?;
            }
            for (n, f) in self.modules[m].aliases.clone() {
                let fs = fields(&f, &["export", "forall", "alias"], 2)?;
                let qs = self.quantifiers(fs.get("forall"))?;
                let alias = fs
                    .get("alias")
                    .ok_or_else(|| error(&f, "type-alias", "A type declaration requires alias."))?;
                self.alias_stack.push((m, n));
                self.ty(one(alias)?, m, &qs)?;
                self.alias_stack.pop();
            }
        }
        let entry = self.modules[0].clone();
        let mut term = if let Some(body) = &entry.body {
            self.expression(body, 0, &entry.caps, &vec![])?
        } else {
            Term::pure(Type::Unit, Expr::Unit)
        };
        if let Some(f) = entry.headers.get("returns") {
            let t = self.ty(one(f)?, 0, &vec![])?;
            term = self.coerce(term, &t, f).map_err(|mut e| {
                if e.code == "type-mismatch" {
                    e.code = "return-type".into();
                }
                e
            })?;
        }
        let mut contracts = Contracts::default();
        if let Some(f) = entry.headers.get("requires") {
            contracts.requires = contracts::conditions(f)?;
        }
        if let Some(f) = entry.headers.get("ensures") {
            contracts.ensures = contracts::conditions(f)?;
        }
        if let Some(f) = entry.headers.get("preserves") {
            contracts.preserves = contracts::protected_targets(f)?;
        }
        let targets = contracts
            .requires
            .iter()
            .chain(&contracts.ensures)
            .map(|c| &c.target)
            .chain(&contracts.preserves);
        let mut actual = term.effects.clone();
        for target in targets {
            let id = entry.caps.get(&target.region).ok_or_else(|| {
                error(
                    &entry.root,
                    "contract-capability",
                    "Contract target has no declared capability.",
                )
            })?;
            let Type::Cap(_, p) = self.bindings[*id].scheme.ty else {
                unreachable!()
            };
            if self.solver.permissions(p) & 1 == 0 {
                return Err(error(
                    &entry.root,
                    "contract-permission",
                    "Contract observations require explicit read permission.",
                ));
            }
            actual.push(Effect::Fs(Region::Fixed(target.region.clone()), 1));
        }
        let actual = self.solver.effects(&actual);
        let declared = if let Some(f) = entry.headers.get("effects") {
            let e = self.effect_type(f, 0, &vec![])?;
            self.solver.include(&actual, &e, f)?;
            Some(e)
        } else {
            None
        };
        if !effect_free(&actual).is_empty() {
            return Err(error(
                entry.body.as_ref().unwrap_or(&entry.root),
                "effect-annotation-required",
                "The script still has unresolved effect or target-region variables.",
            ));
        }
        let mut graph = BTreeMap::new();
        let mut graph_rows = vec![];
        fn intern_type(
            t: Type,
            solver: &Solver,
            graph: &mut BTreeMap<Type, usize>,
            rows: &mut Vec<Value>,
        ) -> usize {
            if let Some(id) = graph.get(&t) {
                return *id;
            }
            let shape = match &t {
                Type::Fn(args, result, effects) => {
                    json!({"kind":"Fn","params":args.iter().map(|t|intern_type(t.clone(),solver,graph,rows)).collect::<Vec<_>>(),"returns":intern_type(result.as_ref().clone(),solver,graph,rows),"effects":effects})
                }
                Type::List(item) => {
                    json!({"kind":"List","item":intern_type(item.as_ref().clone(),solver,graph,rows)})
                }
                Type::Result(item) => {
                    json!({"kind":"Result","ok":intern_type(item.as_ref().clone(),solver,graph,rows),"error":"FsError"})
                }
                Type::Cap(region, rights) => json!({"kind":"Cap","region":region,"rights":rights}),
                Type::Var(id) => json!({"kind":"Variable","id":id}),
                _ => json!({"kind":solver.show(&t)}),
            };
            let id = rows.len();
            rows.push(json!({"display":solver.show(&t),"shape":shape}));
            graph.insert(t, id);
            id
        }
        let mut type_ref = |t: &Type| {
            intern_type(
                self.solver.resolve(t),
                &self.solver,
                &mut graph,
                &mut graph_rows,
            )
        };
        let nodes=self.nodes.iter().map(|n|json!({"source":self.sources[n.form.source].name,"span":n.form.span,"kind":n.kind,"type":self.solver.show(&n.ty),"typeRef":type_ref(&n.ty),"effects":self.effects_report(&n.effects),"details":self.details_report(&n.details)})).collect::<Vec<_>>();
        let aliases=self.alias_views.iter().map(|(n,f,t)|json!({"name":n,"source":self.sources[f.source].name,"span":f.span,"type":self.solver.show(t),"typeRef":type_ref(t)})).collect::<Vec<_>>();
        let mut capabilities = vec![];
        let mut cap_report = vec![];
        // Binding IDs retain declaration order even though lexical lookup uses a map.
        let mut cap_ids: Vec<_> = entry.caps.values().copied().collect();
        cap_ids.sort();
        for id in &cap_ids {
            let b = &self.bindings[*id];
            let Type::Cap(_, p) = &b.scheme.ty else {
                unreachable!()
            };
            let p = self.solver.permissions(*p);
            capabilities.push((b.name.clone(), p));
            cap_report.push(json!({"name":b.name,"permissions":rights_names(p),"infer":b.infer,"span":b.form.span}));
        }
        let report = json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"source":self.sources[0].text,"entry":self.sources[0].name,
            "sources":self.sources.iter().enumerate().map(|(i,s)|json!({"name":s.name,"source":s.text,"entry":i==0})).collect::<Vec<_>>(),
            "type":self.solver.show(&term.ty),"effects":self.effects_report(&actual),"declaredEffects":declared.as_ref().map(|e|self.effects_report(e)),
            "capabilities":cap_report,"contracts":contracts,"nodes":nodes,"aliases":aliases,"typeGraph":graph_rows,"structure":self.structure(&term.node),
            "scope":"Permission bounds; ordered computations; callback internals require stronger contracts.",
            "assurance":{"rustFrontend":"tested; source-level soundness proof pending","os":"explicit trusted boundary"}});
        let mut node = term.node;
        for (id, value) in self.initializers.into_iter().rev() {
            node = Node::new(Expr::Let {
                binding: id,
                value: Box::new(value),
                body: Box::new(node),
                short_circuit: false,
            });
        }
        for id in cap_ids.into_iter().rev() {
            let b = &self.bindings[id];
            let Type::Cap(_, p) = b.scheme.ty else {
                unreachable!()
            };
            node = Node::new(Expr::Let {
                binding: id,
                value: Box::new(Node::new(Expr::Cap(
                    b.name.clone(),
                    self.solver.permissions(p),
                ))),
                body: Box::new(node),
                short_circuit: false,
            });
        }
        node.resolve(&self.solver);
        Ok(Program {
            node,
            ty: self.solver.resolve(&term.ty),
            effects: self.solver.effects(declared.as_ref().unwrap_or(&actual)),
            capabilities,
            report,
            contracts,
        })
    }
}

pub fn compile(sources: Vec<Source>) -> Checked<Program> {
    Compiler::new(sources)?.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(text: &str) -> Checked<Program> {
        compile(vec![Source {
            name: "entry.esh".into(),
            text: text.into(),
        }])
    }
    #[test]
    fn polymorphic_values_are_instantiated_without_re_evaluation() {
        let p = check(
            "(script (def id (params x) (body x)) (do (let a (id true)) (let b (id \"text\")) b))",
        )
        .unwrap();
        assert_eq!(p.ty, Type::Text);
    }
    #[test]
    fn authority_capture_and_explicit_order_are_enforced() {
        assert_eq!(
            check("(script (cap root (read)) (fn (params) (body root)))")
                .unwrap_err()
                .code,
            "capture-required"
        );
        assert_eq!(
            check("(script (text.concat (text.concat \"a\" \"b\") \"c\"))")
                .unwrap_err()
                .code,
            "evaluation-order"
        );
        let p=check("(script (cap root (read write)) (fn (params) (captures _) (body (do (let p (path \"a\")) (fs.read-text root p)))))").unwrap();
        assert!(p.report["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["details"]["captures"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x["narrowed"] == true))));
    }
    #[test]
    fn ordered_list_callbacks_preserve_effect_polymorphism() {
        let p=check("(script (def id (params x) (body x)) (do (let xs (list Text \"a\" \"b\")) (list.map id xs)))").unwrap();
        assert_eq!(p.ty, Type::Text.list());
        assert!(p.effects.is_empty());
    }
    #[test]
    fn existing_examples_and_definition_only_modules() {
        for text in [
            include_str!("../../../examples/copy.esh"),
            include_str!("../../../examples/conditional.esh"),
        ] {
            check(text).unwrap();
        }
        let p = compile(vec![
            Source {
                name: "functions.esh".into(),
                text: include_str!("../../../examples/functions.esh").into(),
            },
            Source {
                name: "lib/io.esh".into(),
                text: include_str!("../../../examples/lib/io.esh").into(),
            },
        ])
        .unwrap();
        assert!(matches!(p.ty, Type::Result(_)));
        assert_eq!(
            check(include_str!("../../../examples/ambiguous.esh"))
                .unwrap_err()
                .code,
            "capability-ambiguous"
        );
    }
}
