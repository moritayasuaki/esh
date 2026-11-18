//! Execute the distributed scripts through the real CLI in isolated fixtures.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::{tempdir, TempDir};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}
fn fixture(name: &str) -> TempDir {
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for item in fs::read_dir(from).unwrap() {
            let item = item.unwrap();
            let dest = to.join(item.file_name());
            if item.file_type().unwrap().is_dir() {
                copy(&item.path(), &dest)
            } else {
                fs::copy(item.path(), dest).unwrap();
            }
        }
    }
    let dir = tempdir().unwrap();
    copy(&examples().join("fixtures").join(name), dir.path());
    dir
}
fn run(name: &str, root: &Path, extra: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_esh"))
        .arg("run")
        .arg(examples().join(format!("{name}.esh")))
        .args(["--grant", "workspace", "read,write,mkdir,remove"])
        .arg(root)
        .args(extra)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    assert_eq!(output.status.success(), value["ok"] == true, "{value}");
    value
}
fn changes(report: &Value) -> usize {
    report["update"]["completedChanges"]
        .as_array()
        .unwrap()
        .len()
}
fn text(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap()
}

#[test]
fn config_update_preserves_secrets() {
    let dir = fixture("config");
    let report = run("config-update", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(text(dir.path(), "app.conf"), "mode=production\nport=8080\n");
    assert_eq!(
        text(dir.path(), "secrets/example.txt"),
        "Synthetic protected data.\n"
    );
    assert_eq!(changes(&report), 1);
}
#[test]
fn config_extra_state_blocks_before_mutation() {
    let dir = fixture("config");
    fs::write(dir.path().join("maintenance.lock"), "").unwrap();
    let state = examples().join("expected-config.state");
    let report = run(
        "config-update",
        dir.path(),
        &["--state", state.to_str().unwrap()],
    );
    assert_eq!(report["ok"], false);
    assert_eq!(changes(&report), 0);
    assert_eq!(
        text(dir.path(), "app.conf"),
        "mode=development\nport=8080\n"
    );
}
#[test]
fn batch_transformation_keeps_originals() {
    let dir = fixture("batch");
    let report = run("batch-transform", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    for name in ["a.txt", "b.txt"] {
        let original = text(dir.path(), &format!("inbox/{name}"));
        assert_eq!(
            text(dir.path(), &format!("outbox/{name}")),
            format!("Processed by esh\n{original}")
        );
    }
    assert_eq!(changes(&report), 2);
}
#[test]
fn batch_invalid_utf8_stops_and_reports_earlier_changes() {
    let dir = fixture("batch");
    fs::write(dir.path().join("inbox/b.txt"), [255]).unwrap();
    fs::write(dir.path().join("inbox/c.txt"), "later").unwrap();
    let report = run("batch-transform", dir.path(), &[]);
    assert_eq!(report["ok"], false);
    assert_eq!(changes(&report), 1);
    assert!(!dir.path().join("outbox/b.txt").exists());
    assert!(!dir.path().join("outbox/c.txt").exists());
    assert!(dir.path().join("inbox/a.txt").exists());
}
#[test]
fn organizing_copies_bytes_before_removing_sources() {
    let dir = fixture("organize");
    fs::write(dir.path().join("inbox/b.bin"), [0, 255, 128]).unwrap();
    let report = run("organize-files", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(
        fs::read(dir.path().join("archive/b.bin")).unwrap(),
        [0, 255, 128]
    );
    assert!(!dir.path().join("inbox/a.txt").exists());
    assert!(!dir.path().join("inbox/b.bin").exists());
    assert_eq!(changes(&report), 4);
}
#[test]
fn failed_archive_copy_does_not_delete_its_source() {
    let dir = fixture("organize");
    fs::create_dir(dir.path().join("archive/a.txt")).unwrap();
    let report = run("organize-files", dir.path(), &[]);
    assert_eq!(report["ok"], false);
    assert!(dir.path().join("inbox/a.txt").is_file());
    assert_eq!(changes(&report), 0);
}
#[test]
fn backup_rotation_preserves_current_and_orders_generations() {
    let dir = fixture("backup");
    let report = run("rotate-backups", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(text(dir.path(), "current.bin"), "current generation\n");
    assert_eq!(text(dir.path(), "backup.1.bin"), "current generation\n");
    assert_eq!(text(dir.path(), "backup.2.bin"), "previous generation\n");
    assert_eq!(changes(&report), 2);
}
#[test]
fn backup_missing_generation_blocks_all_replacements() {
    let dir = fixture("backup");
    fs::remove_file(dir.path().join("backup.2.bin")).unwrap();
    let report = run("rotate-backups", dir.path(), &[]);
    assert_eq!(report["ok"], false);
    assert_eq!(changes(&report), 0);
    assert_eq!(text(dir.path(), "backup.1.bin"), "previous generation\n");
}
#[test]
fn migration_changes_settings_before_publishing_version() {
    let dir = fixture("migration");
    let report = run("migrate-v1-v2", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(text(dir.path(), "VERSION"), "2\n");
    assert_eq!(text(dir.path(), "settings.conf"), "format=current\n");
    assert_eq!(
        text(dir.path(), "data/example.txt"),
        "Preserved application data.\n"
    );
    assert_eq!(
        report["update"]["completedChanges"][0]["target"]["path"],
        "settings.conf"
    );
    assert_eq!(
        report["update"]["completedChanges"][1]["target"]["path"],
        "VERSION"
    );
}
#[test]
fn migration_rejects_an_unexpected_starting_version() {
    let dir = fixture("migration");
    fs::write(dir.path().join("VERSION"), "3\n").unwrap();
    let report = run("migrate-v1-v2", dir.path(), &[]);
    assert_eq!(report["ok"], false);
    assert_eq!(changes(&report), 0);
    assert_eq!(text(dir.path(), "settings.conf"), "format=legacy\n");
}
#[test]
fn manifest_is_sorted_and_does_not_modify_inputs() {
    let dir = fixture("manifest");
    let report = run("manifest", dir.path(), &[]);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(text(dir.path(), "manifest.txt"), "a.txt\nz.txt\n");
    assert_eq!(text(dir.path(), "input/a.txt"), "alpha\n");
}
#[test]
fn manifest_requires_an_absent_output() {
    let dir = fixture("manifest");
    fs::write(dir.path().join("manifest.txt"), "original").unwrap();
    let report = run("manifest", dir.path(), &[]);
    assert_eq!(report["ok"], false);
    assert_eq!(changes(&report), 0);
    assert_eq!(text(dir.path(), "manifest.txt"), "original");
}
