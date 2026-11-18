use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::{tempdir, TempDir};

const TYPES: &str = "EshRust/Generated/Types.lean";
const FUNCTIONS: &str = "EshRust/Generated/Funs.lean";
const FINGERPRINT: &str = "extraction.sha256";
const OUTPUTS: [&str; 3] = [TYPES, FUNCTIONS, FINGERPRINT];

fn tree() -> TempDir {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("EshRust/Generated")).unwrap();
    dir
}

fn run(task: &str, staged: &Path, tracked: &Path, rights: &str, expected_code: i32) -> Value {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/extraction")
        .join(task);
    let output = Command::new(env!("CARGO_BIN_EXE_esh"))
        .arg("run")
        .arg(script)
        .args(["--grant", "staged", "read"])
        .arg(staged)
        .args(["--grant", "tracked", rights])
        .arg(tracked)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(expected_code), "{report}");
    report
}

fn completed_paths(report: &Value) -> Vec<&str> {
    report["update"]["completedChanges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["target"]["path"].as_str().unwrap())
        .collect()
}

#[test]
fn installs_exact_generated_files_and_publishes_the_fingerprint_last() {
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification");
    let tracked = tree();
    fs::write(tracked.path().join(TYPES), "old types").unwrap();
    let report = run("install.esh", &staged, tracked.path(), "write", 0);
    assert_eq!(completed_paths(&report), OUTPUTS);
    for file in OUTPUTS {
        assert_eq!(
            fs::read(tracked.path().join(file)).unwrap(),
            fs::read(staged.join(file)).unwrap()
        );
    }
}

#[test]
fn invalid_staged_output_stops_before_any_replacement() {
    for (path, bytes, code) in [
        (FINGERPRINT, &[0xff][..], "encoding"),
        ("EshRust/Generated/Extra.lean", &b"extra"[..], "script"),
    ] {
        let staged = tree();
        let tracked = tree();
        for file in OUTPUTS {
            fs::write(staged.path().join(file), "new").unwrap();
            fs::write(tracked.path().join(file), "original").unwrap();
        }
        fs::write(staged.path().join(path), bytes).unwrap();
        let report = run("install.esh", staged.path(), tracked.path(), "write", 1);
        assert_eq!(report["value"]["error"]["code"], code, "{report}");
        assert!(completed_paths(&report).is_empty());
        for file in OUTPUTS {
            assert_eq!(fs::read(tracked.path().join(file)).unwrap(), b"original");
        }
    }
}

#[test]
fn a_late_write_failure_keeps_the_old_fingerprint() {
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification");
    let tracked = tree();
    fs::create_dir(tracked.path().join(FUNCTIONS)).unwrap();
    fs::write(tracked.path().join(FINGERPRINT), "old fingerprint").unwrap();
    let report = run("install.esh", &staged, tracked.path(), "write", 1);
    assert_eq!(completed_paths(&report), [TYPES]);
    assert_eq!(
        fs::read(tracked.path().join(TYPES)).unwrap(),
        fs::read(staged.join(TYPES)).unwrap()
    );
    assert!(tracked.path().join(FUNCTIONS).is_dir());
    assert_eq!(
        fs::read(tracked.path().join(FINGERPRINT)).unwrap(),
        b"old fingerprint"
    );
}

#[test]
fn an_omitted_checksum_entry_fails_the_read_only_check() {
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../verification");
    let tracked = tempdir().unwrap();
    let manifest = fs::read_to_string(staged.join(FINGERPRINT)).unwrap();
    let partial = manifest.split_once('\n').unwrap().1;
    fs::write(tracked.path().join(FINGERPRINT), partial).unwrap();
    let report = run("check-fingerprint.esh", &staged, tracked.path(), "read", 1);
    assert_eq!(report["value"]["error"]["code"], "script", "{report}");
    assert!(completed_paths(&report).is_empty());
    assert_eq!(
        fs::read_to_string(tracked.path().join(FINGERPRINT)).unwrap(),
        partial
    );
}
