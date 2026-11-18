use esh_lang::{
    compiler::{compile, Source},
    contracts::add_initial_state,
    review::file_impacts,
};
fn program(s: &str) -> esh_lang::compiler::Program {
    compile(vec![Source {
        name: "entry.esh".into(),
        text: s.into(),
    }])
    .unwrap()
}

#[test]
fn duplicate_reads_remain_two_ordered_events() {
    let p = program(
        r#"(script (cap root (read))
      (def twice (params f x) (body (do (let-ok first (f x)) (f x))))
      (do (let read (fn (params p) (captures root) (body (fs.read-text root p))))
          (let p (path "file")) (twice read p)))"#,
    );
    let review = file_impacts(&p);
    assert!(review.complete);
    assert_eq!(review.impacts.len(), 2);
    assert_eq!(review.impacts[0].path.as_deref(), Some("file"));
    assert_eq!(review.impacts[1].path.as_deref(), Some("file"));
    assert!(review.impacts[1]
        .conditions
        .iter()
        .any(|c| c.contains("succeeds")));
}
#[test]
fn state_refinement_is_invalidated_by_an_earlier_write() {
    let mut p = program(
        r#"(script (cap root (read write))
      (do (let v (path "version")) (let out (path "output"))
          (let-ok changed (fs.write-text root v "new"))
          (let-ok version (fs.read-text root v)) (let old (text.eq version "old"))
          (if old (do (ok unit)) (do (fs.write-text root out "reached")))))"#,
    );
    add_initial_state(&mut p, r#"(state (exact-text root "version" "old"))"#).unwrap();
    let report = file_impacts(&p);
    assert!(report
        .impacts
        .iter()
        .any(|r| r.path.as_deref() == Some("output")));
}
#[test]
fn unknown_list_targets_and_repetition_are_visible() {
    let p = program(
        r#"(script (cap root (read remove))
      (do (let directory (path ".")) (let-ok names (fs.list root directory))
          (let delete (fn (params name) (captures root)
            (body (do (let-ok p (path.parse name)) (fs.remove root p)))))
          (list.map-ok delete names)))"#,
    );
    let report = file_impacts(&p);
    let removal = report
        .impacts
        .iter()
        .find(|r| r.operation == "fs.remove")
        .unwrap();
    assert!(removal.path.is_none());
    assert!(!removal.unknowns.is_empty());
    assert!(removal.repetition.starts_with("E*"));
}
#[test]
fn supplied_facts_require_read_authority_and_respect_effect_bounds() {
    let mut p = program("(script (cap root (write)) unit)");
    assert_eq!(
        add_initial_state(&mut p, "(state (absent root \"x\"))")
            .unwrap_err()
            .code,
        "contract-permission"
    );
    let mut p = program("(script (cap root (read)) (effects) unit)");
    assert_eq!(
        add_initial_state(&mut p, "(state (absent root \"x\"))")
            .unwrap_err()
            .code,
        "effect-bound"
    );
}
#[test]
fn analysis_budget_returns_a_diagnostic_before_stack_exhaustion() {
    let mut source = String::from("(script (do ");
    for i in 0..300 {
        source.push_str(&format!("(let v{i} unit) "));
    }
    source.push_str("unit))");
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: source,
    }])
    .err()
    .unwrap();
    assert_eq!(error.code, "analysis-limit");
}
