//! Renderer-independent document and review state, suitable for Wasm or a native UI.
#![forbid(unsafe_code)]
use esh_lang::{
    compiler::{compile, Source},
    review::file_impacts,
};
use serde_json::{json, Value};

pub struct Document {
    sources: Vec<Source>,
    initial_state: Option<String>,
    state: esh_policy::ReviewState,
    report: Option<Value>,
}
impl Document {
    pub fn new(sources: Vec<Source>) -> Result<Self, &'static str> {
        if sources.is_empty() {
            return Err("A document requires an entry source.");
        }
        Ok(Self {
            sources,
            initial_state: None,
            state: esh_policy::new_review_state(),
            report: None,
        })
    }
    pub fn with_initial_state(
        sources: Vec<Source>,
        state: Option<String>,
    ) -> Result<Self, &'static str> {
        let mut document = Self::new(sources)?;
        document.initial_state = state;
        Ok(document)
    }
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }
    pub fn revision(&self) -> u64 {
        esh_policy::review_revision(&self.state)
    }
    pub fn current_review(&self) -> Option<&Value> {
        if !esh_policy::has_current_review(&self.state) {
            return None;
        }
        self.report.as_ref()
    }
    pub fn edit_entry(&mut self, text: String) -> Result<(), &'static str> {
        if self.sources[0].text == text {
            return Ok(());
        }
        if !esh_policy::mark_edited(&mut self.state) {
            return Err("Document revision limit reached.");
        }
        self.sources[0].text = text;
        Ok(())
    }
    pub fn analyze(&mut self) -> &Value {
        let revision = self.revision();
        let report = match compile(self.sources.clone()) {
            Ok(mut p) => {
                let extra = self
                    .initial_state
                    .as_deref()
                    .map(|s| esh_lang::contracts::add_initial_state(&mut p, s))
                    .transpose();
                match extra {
                    Ok(_) => {
                        p.report["fileImpact"] = json!(file_impacts(&p));
                        p.report
                    }
                    Err(e) => {
                        json!({"ok":false,"diagnostic":{"code":e.code,"message":e.message,"span":e.span,"source":"supplied state"}})
                    }
                }
            }
            Err(e) => {
                json!({"ok":false,"diagnostic":{"code":e.code,"message":e.message,"span":e.span,"source":self.sources[e.source].name}})
            }
        };
        self.accept_report(revision, report);
        self.current_review().unwrap()
    }
    pub fn accept_report(&mut self, revision: u64, report: Value) -> bool {
        if !esh_policy::accept_review_result(&mut self.state, revision) {
            return false;
        }
        self.report = Some(report);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_invalidate_review_and_old_results_cannot_restore_it() {
        let mut d = Document::new(vec![Source {
            name: "entry.esh".into(),
            text: "(script unit)".into(),
        }])
        .unwrap();
        let old = d.analyze().clone();
        assert!(d.current_review().is_some());
        d.edit_entry("(script true)".into()).unwrap();
        assert!(d.current_review().is_none());
        assert!(!d.accept_report(0, old));
        assert!(d.current_review().is_none());
        assert_eq!(d.analyze()["type"], "Bool");
    }
    #[test]
    fn diagnostics_replace_old_success() {
        let mut d = Document::new(vec![Source {
            name: "entry.esh".into(),
            text: "(script unit)".into(),
        }])
        .unwrap();
        d.analyze();
        d.edit_entry("(script unknown)".into()).unwrap();
        assert_eq!(d.analyze()["ok"], false);
    }

    #[test]
    fn supplied_state_is_conjoined_and_rechecked_after_edits() {
        let mut d = Document::with_initial_state(
            vec![Source {
                name: "entry.esh".into(),
                text: "(script (cap root (read)) unit)".into(),
            }],
            Some("(state (absent root \"output\"))".into()),
        )
        .unwrap();
        assert_eq!(
            d.analyze()["contracts"]["requires"][0]["target"]["path"],
            "output"
        );
        d.edit_entry("(script unit)".into()).unwrap();
        assert_eq!(d.analyze()["diagnostic"]["code"], "contract-permission");
    }
}
