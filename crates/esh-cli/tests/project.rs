use serde_json::Value;
use std::{fs, path::Path, process::Command};
use tempfile::tempdir;

fn check(file: &Path, root: Option<&Path>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_esh"));
    command.arg("check").arg(file);
    if let Some(root) = root {
        command.arg("--project-root").arg(root);
    }
    let output = command.output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn importer_relative_paths_respect_the_designated_root() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("scripts")).unwrap();
    fs::write(
        dir.path().join("lib.esh"),
        "(module lib (def value (export) (params) (returns Text) (effects) (body \"root\")))",
    )
    .unwrap();
    let file = dir.path().join("scripts/entry.esh");
    fs::write(&file, "(script (import lib \"../lib.esh\") (lib.value))").unwrap();
    assert_eq!(check(&file, None)["ok"], false);
    let report = check(&file, Some(dir.path()));
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["sources"].as_array().unwrap().len(), 2);
}
#[test]
fn imports_cannot_follow_symlinks_outside_the_root_or_cycles() {
    use std::os::unix::fs::symlink;
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let entry = dir.path().join("entry.esh");
    fs::write(outside.path().join("lib.esh"), "(module lib)").unwrap();
    symlink(outside.path().join("lib.esh"), dir.path().join("link.esh")).unwrap();
    fs::write(&entry, "(script (import lib \"./link.esh\") unit)").unwrap();
    assert_eq!(check(&entry, None)["ok"], false);
    symlink("second.esh", dir.path().join("first.esh")).unwrap();
    symlink("first.esh", dir.path().join("second.esh")).unwrap();
    fs::write(&entry, "(script (import cycle \"./first.esh\") unit)").unwrap();
    assert_eq!(check(&entry, None)["ok"], false);
}
#[test]
fn repeated_imports_share_one_source_snapshot() {
    let dir = tempdir().unwrap();
    let entry = dir.path().join("entry.esh");
    fs::write(dir.path().join("lib.esh"),"(module lib (def id (export) (forall (A Type)) (params (x A)) (returns A) (effects) (body x)))").unwrap();
    fs::write(&entry,"(script (import first \"./lib.esh\") (import second \"lib.esh\") (do (let a (first.id \"a\")) (second.id true)))").unwrap();
    let report = check(&entry, None);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["sources"].as_array().unwrap().len(), 2);
}
#[test]
fn state_fifo_is_rejected_without_waiting_for_a_writer() {
    let dir = tempdir().unwrap();
    let entry = dir.path().join("entry.esh");
    let state = dir.path().join("state");
    fs::write(&entry, "(script unit)").unwrap();
    assert!(Command::new("mkfifo")
        .arg(&state)
        .status()
        .unwrap()
        .success());
    let output = Command::new(env!("CARGO_BIN_EXE_esh"))
        .arg("check")
        .arg(entry)
        .arg("--state")
        .arg(state)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["code"], "source");
}
