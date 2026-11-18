use esh_host::{Grant, Host};
use esh_lang::{
    compiler::{compile, Program, Source},
    contracts::initial_state,
};
use esh_runtime::{
    update::{CheckLimits, UpdateHost},
    Limits, Runtime, Value,
};
use std::fs;
use tempfile::tempdir;

fn program(text: &str) -> Program {
    compile(vec![Source {
        name: "entry.esh".into(),
        text: text.into(),
    }])
    .unwrap()
}
fn session(path: &std::path::Path) -> UpdateHost {
    UpdateHost::new(
        Host::new(
            &[Grant {
                name: "root".into(),
                rights: 15,
                directory: path.into(),
            }],
            1024,
        )
        .unwrap(),
        vec![],
        CheckLimits::default(),
    )
}

#[test]
fn preflight_mismatch_leaves_every_target_unchanged() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("version"), "unexpected").unwrap();
    let p=program("(script (cap root (read write)) (requires (exact-text root \"version\" \"v1\")) (do (let p (path \"output\")) (fs.write-text root p \"new\")))");
    let mut host = session(dir.path());
    let e = Runtime::new(&mut host, Limits::default())
        .run(&p)
        .unwrap_err();
    assert_eq!(e.code, "condition-mismatch");
    assert!(!dir.path().join("output").exists());
    assert!(host.completed.is_empty());
}
#[test]
fn own_changes_evolve_preconditions_without_refreshing_unrelated_facts() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("version"), "v1").unwrap();
    let p=program("(script (cap root (read write)) (requires (exact-text root \"version\" \"v1\")) (ensures (exact-text root \"version\" \"v3\")) (do (let p (path \"version\")) (let-ok a (fs.write-text root p \"v2\")) (fs.write-text root p \"v3\")))");
    let mut host = session(dir.path());
    assert!(!Runtime::new(&mut host, Limits::default())
        .run(&p)
        .unwrap()
        .failed());
    assert_eq!(
        fs::read_to_string(dir.path().join("version")).unwrap(),
        "v3"
    );
    assert_eq!(host.completed.len(), 2);
    assert_eq!(host.report()["postconditionsPassed"], true);
}
#[test]
fn a_failed_postcondition_reports_completed_changes_without_rollback() {
    let dir = tempdir().unwrap();
    let p=program("(script (cap root (read write)) (ensures (exact-text root \"output\" \"expected\")) (do (let p (path \"output\")) (fs.write-text root p \"actual\")))");
    let mut host = session(dir.path());
    assert_eq!(
        Runtime::new(&mut host, Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "condition-mismatch"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("output")).unwrap(),
        "actual"
    );
    assert_eq!(host.completed.len(), 1);
    assert_eq!(host.report()["rolledBack"], false);
}
#[test]
fn supplied_state_strengthens_requirements_and_budget_exhaustion_blocks() {
    let dir = tempdir().unwrap();
    let p=program("(script (cap root (read write)) (requires (absent root \"output\")) (do (let p (path \"output\")) (fs.write-text root p \"new\")))");
    let extra = initial_state("(state (regular-file root \"missing\"))").unwrap();
    let h = Host::new(
        &[Grant {
            name: "root".into(),
            rights: 15,
            directory: dir.path().into(),
        }],
        1024,
    )
    .unwrap();
    let mut host = UpdateHost::new(h, extra, CheckLimits::default());
    assert!(Runtime::new(&mut host, Limits::default()).run(&p).is_err());
    assert!(!dir.path().join("output").exists());
    let h = Host::new(
        &[Grant {
            name: "root".into(),
            rights: 15,
            directory: dir.path().into(),
        }],
        1024,
    )
    .unwrap();
    let mut host = UpdateHost::new(
        h,
        vec![],
        CheckLimits {
            max_observations: 0,
            max_content_bytes: 1024,
        },
    );
    assert_eq!(
        Runtime::new(&mut host, Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "check-incomplete"
    );
    assert!(!dir.path().join("output").exists());
}
#[test]
fn preservation_covers_subtrees_through_another_root() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("protected")).unwrap();
    fs::write(dir.path().join("protected/file"), "original").unwrap();
    let p=program("(script (cap root (read)) (cap alias (write)) (preserves (target root \"protected\")) (do (let p (path \"file\")) (fs.write-text alias p \"changed\")))");
    let h = Host::new(
        &[
            Grant {
                name: "root".into(),
                rights: 1,
                directory: dir.path().into(),
            },
            Grant {
                name: "alias".into(),
                rights: 2,
                directory: dir.path().join("protected"),
            },
        ],
        1024,
    )
    .unwrap();
    let mut host = UpdateHost::new(h, vec![], CheckLimits::default());
    assert_eq!(
        Runtime::new(&mut host, Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "preserves"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("protected/file")).unwrap(),
        "original"
    );
}
#[test]
fn map_ok_stops_after_the_first_error_and_keeps_declaration_order() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("first"), "a").unwrap();
    fs::write(dir.path().join("last"), "c").unwrap();
    let p=program("(script (cap root (read remove)) (do (let items (list Text \"first\" \"missing\" \"last\")) (let action (fn (params item) (captures root) (body (do (let-ok p (path.parse item)) (fs.remove root p))))) (list.map-ok action items)))");
    let mut host = session(dir.path());
    let mut runtime = Runtime::new(&mut host, Limits::default());
    let result = runtime.run(&p).unwrap();
    assert!(result.failed());
    assert_eq!(runtime.trace.len(), 2);
    assert!(!dir.path().join("first").exists());
    assert!(dir.path().join("last").exists());
    assert_eq!(host.completed.len(), 1);
}
#[test]
fn bytes_roundtrip_and_ordered_fold() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("binary"), [0, 255, 1, 128]).unwrap();
    let p=program("(script (cap root (read write)) (do (let from (path \"binary\")) (let to (path \"copy\")) (let-ok bytes (fs.read-bytes root from)) (fs.write-bytes root to bytes)))");
    let mut host = session(dir.path());
    assert!(!Runtime::new(&mut host, Limits::default())
        .run(&p)
        .unwrap()
        .failed());
    assert_eq!(
        fs::read(dir.path().join("copy")).unwrap(),
        vec![0, 255, 1, 128]
    );
    let p = program(
        "(script (do (let xs (list Text \"a\" \"b\" \"c\")) (list.fold text.concat \"\" xs)))",
    );
    let mut host = session(dir.path());
    assert!(
        matches!(Runtime::new(&mut host,Limits::default()).run(&p).unwrap(),Value::Text(s) if s.as_ref()=="abc")
    );
}

#[test]
fn drift_between_mutations_stops_without_accepting_a_new_baseline() {
    use esh_host::{Operation, Request};
    use esh_runtime::Backend;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("protected-version"), "old").unwrap();
    let p=program("(script (cap root (read write)) (requires (exact-text root \"protected-version\" \"old\")) (do (let p (path \"first\")) (fs.write-text root p \"data\")))");
    let mut host = session(dir.path());
    host.begin(&p).unwrap();
    let write = |path: &str| Request {
        operation: Operation::Write,
        region: "root".into(),
        path: path.into(),
        contents: Some("data".into()),
        bytes: None,
    };
    host.request(&write("first")).unwrap();
    fs::write(dir.path().join("protected-version"), "external change").unwrap();
    assert_eq!(
        host.request(&write("second")).unwrap_err().code,
        "condition-mismatch"
    );
    assert!(host.halted());
    assert!(!dir.path().join("second").exists());
    assert_eq!(host.completed.len(), 1);
}

#[test]
fn retained_regions_and_repeated_callback_effects_survive_instantiation() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("file"), "twice").unwrap();
    let p = program(
        r#"(script (cap root (read))
      (def combine (params f g)
        (body (fn (params) (captures f g) (body (do (let first (f)) (g))))))
      (do (let p (path "file"))
          (let read (fn (params) (captures root) (body (fs.read-text root p))))
          (let combined (combine read read)) (combined)))"#,
    );
    let mut host = session(dir.path());
    let mut runtime = Runtime::new(&mut host, Limits::default());
    assert!(!runtime.run(&p).unwrap().failed());
    assert_eq!(runtime.trace.len(), 2);
    assert!(runtime.trace.iter().all(|r| r["region"] == "root"));
}

#[test]
fn malformed_backend_responses_stop_before_dependent_mutation() {
    struct Invalid {
        calls: usize,
    }
    impl esh_runtime::Backend for Invalid {
        fn request(
            &mut self,
            _: &esh_host::Request,
        ) -> Result<serde_json::Value, esh_host::HostError> {
            self.calls += 1;
            Ok(serde_json::json!(42))
        }
    }
    let p = program(
        r#"(script (cap root (read write)) (do (let p (path "file")) (let-ok contents (fs.read-text root p)) (fs.write-text root p contents)))"#,
    );
    let mut backend = Invalid { calls: 0 };
    assert_eq!(
        Runtime::new(&mut backend, Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "runtime-type"
    );
    assert_eq!(backend.calls, 1);
}

#[test]
fn runtime_checks_effect_bounds_independently_of_capability_grants() {
    let dir = tempdir().unwrap();
    let mut p = program(
        r#"(script (cap root (read write)) (do (let p (path "output")) (fs.write-text root p "forbidden")))"#,
    );
    p.effects.clear(); // Model inconsistent checked IR at the Rust API boundary.
    let mut host = session(dir.path());
    assert_eq!(
        Runtime::new(&mut host, Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "effect-bound"
    );
    assert!(!dir.path().join("output").exists());
    assert!(host.completed.is_empty());
}

#[test]
fn cancellation_before_start_performs_no_observations_or_writes() {
    let dir = tempdir().unwrap();
    let p = program(
        r#"(script (cap root (read write)) (requires (absent root "output")) (do (let p (path "output")) (fs.write-text root p "new")))"#,
    );
    let mut host = session(dir.path());
    let limits = Limits::default();
    limits
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        Runtime::new(&mut host, limits).run(&p).unwrap_err().code,
        "cancelled"
    );
    assert!(host.checks.is_empty());
    assert!(!dir.path().join("output").exists());
}

#[test]
fn content_hash_preconditions_are_checked_over_bytes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("empty"), []).unwrap();
    let p = program(
        r#"(script (cap root (read)) (requires (content-hash root "empty" "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")) unit)"#,
    );
    assert!(!Runtime::new(&mut session(dir.path()), Limits::default())
        .run(&p)
        .unwrap()
        .failed());
    fs::write(dir.path().join("empty"), [0]).unwrap();
    assert_eq!(
        Runtime::new(&mut session(dir.path()), Limits::default())
            .run(&p)
            .unwrap_err()
            .code,
        "condition-mismatch"
    );
}
