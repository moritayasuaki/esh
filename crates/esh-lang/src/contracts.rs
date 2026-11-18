//! Declared filesystem facts. Omission means unknown, never absent.
use crate::types::{error, Checked};
use esh_syntax::Form;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub region: String,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "expected", rename_all = "kebab-case")]
pub enum Predicate {
    RegularFile,
    Directory,
    Absent,
    ExactText(String),
    ContentHash(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub target: Target,
    pub predicate: Predicate,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contracts {
    pub requires: Vec<Condition>,
    pub preserves: Vec<Target>,
    pub ensures: Vec<Condition>,
}

fn target(xs: &[Form], f: &Form) -> Checked<Target> {
    let region = xs.first().and_then(Form::atom).ok_or_else(|| {
        error(
            f,
            "contract-target",
            "A contract target requires a capability name.",
        )
    })?;
    let path = xs.get(1).and_then(Form::text).ok_or_else(|| {
        error(
            f,
            "contract-target",
            "A contract target requires a literal relative path.",
        )
    })?;
    if !esh_policy::valid_relative_path(path.as_bytes()) {
        return Err(error(f, "contract-target", "Invalid contract path."));
    }
    Ok(Target {
        region: region.into(),
        path: path.into(),
    })
}
pub fn conditions(field: &Form) -> Checked<Vec<Condition>> {
    field.list().ok_or_else(||error(field,"contract","Expected a condition list."))?[1..].iter().map(|f|{
        let xs=f.list().filter(|xs|xs.len()>=3).ok_or_else(||error(f,"contract","Expected (predicate capability \"relative-path\" [expected])."))?;
        let t=target(&xs[1..],f)?;
        let predicate=match (xs[0].atom(),&xs[3..]){
            (Some("regular-file"),[])=>Predicate::RegularFile,
            (Some("directory"),[])=>Predicate::Directory,
            (Some("absent"),[])=>Predicate::Absent,
            (Some("exact-text"),[text]) if text.text().is_some()=>Predicate::ExactText(text.text().unwrap().into()),
            (Some("content-hash"),[hash]) if hash.text().is_some()=>{
                let s=hash.text().unwrap();if s.len()!=64||!s.bytes().all(|c|c.is_ascii_hexdigit()){return Err(error(hash,"contract-hash","content-hash requires a 64-digit SHA-256 digest."));}Predicate::ContentHash(s.to_ascii_lowercase())
            },_=>return Err(error(f,"contract","Expected regular-file, directory, absent, exact-text, or content-hash with the required arguments.")),
        };Ok(Condition{target:t,predicate})
    }).collect()
}
pub fn protected_targets(field: &Form) -> Checked<Vec<Target>> {
    field
        .list()
        .ok_or_else(|| error(field, "contract", "Expected a target list."))?[1..]
        .iter()
        .map(|f| {
            let xs = f
                .list()
                .filter(|xs| xs.len() == 3 && xs[0].is_atom("target"))
                .ok_or_else(|| {
                    error(
                        f,
                        "contract",
                        "Expected (target capability \"relative-path\").",
                    )
                })?;
            target(&xs[1..], f)
        })
        .collect()
}
pub fn initial_state(text: &str) -> Checked<Vec<Condition>> {
    let f = esh_syntax::parse(text, 0)?;
    if !f.has_head("state") {
        return Err(error(&f, "state", "Expected (state condition ...)."));
    }
    conditions(&f)
}

/// Conjoin caller-supplied facts with the script contract, using the same checks
/// in the CLI and the editor. This only parses facts; it does not observe the OS.
pub fn add_initial_state(program: &mut crate::compiler::Program, text: &str) -> Checked<()> {
    use crate::types::{Effect, Region};
    use serde_json::json;
    let form = esh_syntax::parse(text, 0)?;
    let conditions = initial_state(text)?;
    for condition in &conditions {
        let region = &condition.target.region;
        if !program
            .capabilities
            .iter()
            .any(|(r, rights)| r == region && rights & 1 != 0)
        {
            return Err(error(
                &form,
                "contract-permission",
                "Supplied initial conditions require a declared read capability.",
            ));
        }
        let effect = Effect::Fs(Region::Fixed(region.clone()), 1);
        if !program.report["declaredEffects"].is_null() && !program.effects.contains(&effect) {
            return Err(error(
                &form,
                "effect-bound",
                "The declared effect bound does not include observations from the supplied state.",
            ));
        }
    }
    for condition in &conditions {
        let effect = Effect::Fs(Region::Fixed(condition.target.region.clone()), 1);
        if !program.effects.contains(&effect) {
            program.effects.push(effect);
            program.report["effects"]
                .as_array_mut()
                .unwrap()
                .push(json!({"operation":"fs.read","region":condition.target.region}));
        }
    }
    program.contracts.requires.extend(conditions);
    program.report["contracts"] = json!(program.contracts);
    program.report["suppliedState"] = json!(text);
    Ok(())
}
