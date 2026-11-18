//! Behavioral regressions retained from the independently checked reference frontend.
use esh_lang::compiler::{compile, Source};

#[test]
fn unused_export_is_checked_universally_0() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (def wrong (export) (forall (A Type))\n          (params (x A)) (returns A) (effects) (body \"only text\")) unit)".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn exported_signature_is_required_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (def id (export) (params x) (body x)) unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "public-signature", "{error:?}");
}

#[test]
fn exported_signature_is_required_1() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (def id (export) (params (x Text)) (returns Text) (body x)) unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "public-signature", "{error:?}");
}

#[test]
fn full_arity_and_explicit_partial_application_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (text.concat \"one\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "arity", "{error:?}");
}

#[test]
fn pure_calls_and_callees_must_be_sequenced_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (text.concat (text.concat \"a\" \"b\") \"c\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "evaluation-order", "{error:?}");
}

#[test]
fn pure_calls_and_callees_must_be_sequenced_1() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script ((text.concat \"a\" \"b\") \"c\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "evaluation-order", "{error:?}");
}

#[test]
fn pure_calls_and_callees_must_be_sequenced_2() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (if (text.eq \"a\" \"b\") unit unit))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "evaluation-order", "{error:?}");
}

#[test]
fn pure_calls_and_callees_must_be_sequenced_3() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (if true (text.concat \"a\" \"b\") \"c\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "evaluation-order", "{error:?}");
}

#[test]
fn unknown_type_and_pure_callback_captures_require_declaration_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text:
            "(script (def keep (params x)\n          (body (fn (params ignored) (body x)))) unit)"
                .into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "capture-required", "{error:?}");
}

#[test]
fn unknown_type_and_pure_callback_captures_require_declaration_1() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (def wrap (params (f (Fn () Unit (effects))))\n          (body (fn (params) (body (f))))) unit)".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "capture-required", "{error:?}");
}

#[test]
fn effectful_binding_is_not_generalized_0() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (cap root (read))\n          (def load (params root path)\n            (body (do (let-ok text (fs.read-text root path))\n                      (let id (fn (params x) (body x))) (ok id))))\n          (do (let p (path \"file\")) (let-ok id (load root p))\n              (let bool (id true)) (let text (id \"text\")) (ok text)))".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn callback_parameter_is_monomorphic_within_one_call_0() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (def twice (params callback)\n          (body (do (let a (callback \"text\")) (callback true)))) unit)".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn forward_references_and_recursive_cycles_0() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (def first (params x) (body (second x)))\n          (def second (params x) (body (first x))) unit)".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "definition-cycle", "{error:?}");
}

#[test]
fn transparent_parameterized_alias_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (type Cycle (alias Cycle)) unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "type-cycle", "{error:?}");
}

#[test]
fn import_visibility_and_unused_public_contract_0() {
    let error = compile(vec![
        Source {
            name: "entry.esh".into(),
            text: "(script (import lib \"./library.esh\") (lib.private \"x\"))".into(),
        },
        Source {
            name: "library.esh".into(),
            text: "(module lib (def private (params x) (body x)))".into(),
        },
    ])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "private-name", "{error:?}");
}

#[test]
fn import_visibility_and_unused_public_contract_1() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (import lib \"./library.esh\") unit)".into()},Source{name:"library.esh".into(),text:"(module lib (def bad (export) (params (x Bool)) (returns Text) (effects) (body x)))".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn import_cycles_and_root_escape_0() {
    let error = compile(vec![
        Source {
            name: "entry.esh".into(),
            text: "(script (import a \"./a.esh\") unit)".into(),
        },
        Source {
            name: "a.esh".into(),
            text: "(module a (import b \"./b.esh\"))".into(),
        },
        Source {
            name: "b.esh".into(),
            text: "(module b (import a \"./a.esh\"))".into(),
        },
    ])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "import-cycle", "{error:?}");
}

#[test]
fn import_cycles_and_root_escape_1() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (import outside \"../outside.esh\") unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "project-root", "{error:?}");
}

#[test]
fn invalid_export_and_conflicting_import_name_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (type Name (export extra) (alias Text)) unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "export", "{error:?}");
}

#[test]
fn invalid_export_and_conflicting_import_name_1() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (cap lib (read)) (import lib \"./lib.esh\") unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "duplicate-name", "{error:?}");
}

#[test]
fn let_is_lexically_scoped_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script  (let x \"a\" missing))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "unknown-variable", "{error:?}");
}

#[test]
fn wrong_argument_type_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script  (text.concat true \"b\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn branch_types_must_agree_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script  (if true \"a\" false))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn result_binding_requires_result_body_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script  (let-ok x (ok \"a\") x))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn return_annotation_checked_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (returns Bool) \"a\")".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "return-type", "{error:?}");
}

#[test]
fn effect_annotation_is_upper_bound_0() {
    let error=compile(vec![Source{name:"entry.esh".into(),text:"(script (cap out (write)) (effects) (do (let p (path \"x\")) (fs.write-text out p \"data\")))".into()}]).expect_err("source must be rejected");
    assert_eq!(error.code, "effect-bound", "{error:?}");
}

#[test]
fn unregistered_capability_is_not_inferred_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (cap input (read)) (do (let p (path \"x\")) (fs.read-text _ p)))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "capability-missing", "{error:?}");
}

#[test]
fn permission_cannot_be_amplified_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (cap a (read)) (do (let p (path \"x\")) (fs.write-text a p \"data\")))"
            .into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "permission-denied", "{error:?}");
}

#[test]
fn paths_are_distinct_from_text_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (cap a (read)) (fs.read-text a \"x\"))".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "type-mismatch", "{error:?}");
}

#[test]
fn control_character_in_identifier_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script dangerous\u{001b}name)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "syntax", "{error:?}");
}

#[test]
fn duplicate_capabilities_rejected_0() {
    let error = compile(vec![Source {
        name: "entry.esh".into(),
        text: "(script (cap a (read)) (cap a (write)) unit)".into(),
    }])
    .expect_err("source must be rejected");
    assert_eq!(error.code, "capability", "{error:?}");
}
