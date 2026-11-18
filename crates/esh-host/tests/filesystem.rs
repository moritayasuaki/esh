#![cfg(unix)]

use esh_host::{CapabilityDeclaration, Grant, Host, Operation, Request};
use serde_json::json;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::TempDir;

fn request(operation: Operation, path: &str, contents: Option<&str>) -> Request {
    Request {
        operation,
        region: "workspace".into(),
        path: path.into(),
        contents: contents.map(str::to_owned),
        bytes: None,
    }
}

fn host(root: &TempDir, declared: &[&str], limit: usize) -> Host {
    let mut host = Host::new(
        &[Grant {
            name: "workspace".into(),
            rights: 15,
            directory: root.path().to_owned(),
        }],
        limit,
    )
    .unwrap();
    host.authorize(&[CapabilityDeclaration {
        name: "workspace".into(),
        permissions: declared.iter().map(|x| (*x).into()).collect(),
    }])
    .unwrap();
    host
}

#[test]
fn grants_must_be_activated_and_cannot_be_amplified() {
    let root = TempDir::new().unwrap();
    let mut host = Host::new(
        &[Grant {
            name: "workspace".into(),
            rights: 1,
            directory: root.path().to_owned(),
        }],
        128,
    )
    .unwrap();
    let read = request(Operation::Read, "file", None);
    assert_eq!(host.dispatch(&read)["error"]["code"], "permission");
    assert!(host
        .authorize(&[CapabilityDeclaration {
            name: "workspace".into(),
            permissions: vec!["write".into()],
        }])
        .is_err());
    assert_eq!(host.dispatch(&read)["error"]["code"], "permission");
    host.authorize(&[CapabilityDeclaration {
        name: "workspace".into(),
        permissions: vec!["read".into()],
    }])
    .unwrap();
    assert_eq!(host.dispatch(&read)["error"]["code"], "ENOENT");
    assert_eq!(
        host.dispatch(&request(Operation::Write, "file", Some("no")))["error"]["code"],
        "permission"
    );
    assert!(!root.path().join("file").exists());
}

#[test]
fn active_policy_does_not_inherit_extra_host_rights() {
    let root = TempDir::new().unwrap();
    let host = host(&root, &["read"], 128);
    for operation in [Operation::Write, Operation::Mkdir, Operation::Remove] {
        assert_eq!(
            host.dispatch(&request(operation, "file", Some("no")))["error"]["code"],
            "permission"
        );
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn replacement_does_not_modify_another_hard_link() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("original"), "old").unwrap();
    fs::set_permissions(
        root.path().join("original"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    fs::hard_link(root.path().join("original"), root.path().join("other")).unwrap();
    let host = host(&root, &["read", "write"], 128);
    assert_eq!(
        host.dispatch(&request(Operation::Write, "original", Some("new"))),
        json!({"ok":null})
    );
    assert_eq!(
        fs::read_to_string(root.path().join("original")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("other")).unwrap(),
        "old"
    );
    assert_eq!(
        fs::metadata(root.path().join("original"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn directory_workflow_and_unicode_names() {
    let root = TempDir::new().unwrap();
    let host = host(&root, &["read", "write", "mkdir", "remove"], 128);
    assert_eq!(
        host.dispatch(&request(Operation::Mkdir, "data", None)),
        json!({"ok":null})
    );
    assert_eq!(
        host.dispatch(&request(Operation::Write, "data/λ.txt", Some("hello\n"))),
        json!({"ok":null})
    );
    assert_eq!(
        host.dispatch(&request(Operation::List, ".", None)),
        json!({"ok":["data"]})
    );
    assert_eq!(
        host.dispatch(&request(Operation::List, "data", None)),
        json!({"ok":["λ.txt"]})
    );
    assert_eq!(
        host.dispatch(&request(Operation::Read, "data/λ.txt", None)),
        json!({"ok":"hello\n"})
    );
    assert_eq!(
        host.dispatch(&request(Operation::Remove, "data/λ.txt", None)),
        json!({"ok":null})
    );
    assert_eq!(
        host.dispatch(&request(Operation::Remove, "data", None))["error"]["code"],
        "file-kind"
    );
    assert!(root.path().join("data").is_dir());
}

#[test]
fn symlinks_are_rejected_for_reads_writes_and_removal() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("file"), "untouched").unwrap();
    symlink(outside.path(), root.path().join("directory-link")).unwrap();
    symlink(outside.path().join("file"), root.path().join("file-link")).unwrap();
    let host = host(&root, &["read", "write", "remove"], 128);
    for operation in [Operation::Read, Operation::Write, Operation::Remove] {
        for path in ["file-link", "directory-link/file"] {
            assert!(host
                .dispatch(&request(operation, path, Some("wrong")))
                .get("error")
                .is_some());
        }
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("file")).unwrap(),
        "untouched"
    );
    assert!(fs::symlink_metadata(root.path().join("file-link"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn invalid_paths_never_touch_the_filesystem() {
    let root = TempDir::new().unwrap();
    let host = host(&root, &["write"], 128);
    for path in ["../file", "/file", "a//b", "a/./b", "a\\b", "file\0"] {
        assert_eq!(
            host.dispatch(&request(Operation::Write, path, Some("wrong")))["error"]["code"],
            "path"
        );
    }
    assert_eq!(
        host.dispatch(&request(Operation::Write, ".", Some("wrong")))["error"]["code"],
        "path"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn resource_limits_and_utf8_errors_preserve_existing_files() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("large"), "123456789").unwrap();
    fs::write(root.path().join("bad"), [0xff]).unwrap();
    let host = host(&root, &["read", "write"], 8);
    assert_eq!(
        host.dispatch(&request(Operation::Read, "large", None))["error"]["code"],
        "resource-limit"
    );
    assert_eq!(
        host.dispatch(&request(Operation::Read, "bad", None))["error"]["code"],
        "encoding"
    );
    assert_eq!(
        host.dispatch(&request(Operation::Write, "large", Some("too large")))["error"]["code"],
        "resource-limit"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("large")).unwrap(),
        "123456789"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn declared_root_names_may_alias() {
    let root = TempDir::new().unwrap();
    let mut host = Host::new(
        &[
            Grant {
                name: "first".into(),
                rights: 2,
                directory: root.path().into(),
            },
            Grant {
                name: "second".into(),
                rights: 1,
                directory: root.path().into(),
            },
        ],
        128,
    )
    .unwrap();
    host.authorize(&[
        CapabilityDeclaration {
            name: "first".into(),
            permissions: vec!["write".into()],
        },
        CapabilityDeclaration {
            name: "second".into(),
            permissions: vec!["read".into()],
        },
    ])
    .unwrap();
    let mut write = request(Operation::Write, "shared", Some("data"));
    write.region = "first".into();
    assert_eq!(host.dispatch(&write), json!({"ok":null}));
    let mut read = request(Operation::Read, "shared", None);
    read.region = "second".into();
    assert_eq!(host.dispatch(&read), json!({"ok":"data"}));
}
