//! Pure decisions shared by the Rust host and its verification model.
//!
//! Tests do not constitute a proof of this Rust implementation. The migration's
//! code-to-proof correspondence must be checked by the selected verifier.
#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permission {
    Read,
    Write,
    Mkdir,
    Remove,
}

pub fn permission_bit(permission: Permission) -> u8 {
    match permission {
        Permission::Read => 1,
        Permission::Write => 2,
        Permission::Mkdir => 4,
        Permission::Remove => 8,
    }
}

pub fn valid_rights(rights: u8) -> bool {
    rights & 0xf0 == 0
}

pub fn covers(granted: u8, required: u8) -> bool {
    valid_rights(granted) && valid_rights(required) && granted & required == required
}

pub fn allows(granted: u8, permission: Permission) -> bool {
    covers(granted, permission_bit(permission))
}

/// Validate one relative component, without normalizing or resolving it.
pub fn valid_component(component: &[u8]) -> bool {
    if component.is_empty() {
        return false;
    }
    if component.len() == 1 && component[0] == b'.' {
        return false;
    }
    if component.len() == 2 && component[0] == b'.' && component[1] == b'.' {
        return false;
    }
    valid_component_bytes(component)
}

/// Byte-level component check, kept separate for a single extracted loop.
pub fn valid_component_bytes(component: &[u8]) -> bool {
    let mut i = 0;
    while i < component.len() {
        let byte = component[i];
        if byte < 32 || byte == 127 || byte == b'/' || byte == b'\\' {
            return false;
        }
        i += 1;
    }
    true
}

/// The root marker is valid for listing; individual operations can forbid it.
pub fn valid_relative_path(path: &[u8]) -> bool {
    if path.len() == 1 && path[0] == b'.' {
        return true;
    }
    if path.is_empty() {
        return false;
    }
    let mut start = 0;
    let mut i = 0;
    while i < path.len() {
        if path[i] == b'/' {
            if !valid_component(&path[start..i]) {
                return false;
            }
            start = i + 1;
        }
        i += 1;
    }
    valid_component(&path[start..])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckStatus {
    Pending,
    Passed,
    Mismatch,
    Incomplete,
    Unavailable,
}

/// Only mandatory initial conditions belong in this input. Supplementary
/// checks have a separate policy and cannot discharge a mandatory condition.
pub fn preflight_allows_update(checks: &[CheckStatus]) -> bool {
    let mut i = 0;
    while i < checks.len() {
        if checks[i] != CheckStatus::Passed {
            return false;
        }
        i += 1;
    }
    true
}

/// Rechecking never upgrades an assumption into evidence of continuous isolation.
pub fn admit_mutation(initial_passed: bool, rechecks_passed: bool, protected: bool) -> bool {
    initial_passed && rechecks_passed && !protected
}

pub fn may_report_success(body_succeeded: bool, postconditions_passed: bool) -> bool {
    body_succeeded && postconditions_passed
}

/// A review of an older document cannot establish the current document's status.
pub fn accept_review_revision(current: u64, analyzed: u64) -> bool {
    current == analyzed
}

/// The renderer-independent review state is extracted together with the policy.
pub struct ReviewState {
    revision: u64,
    reviewed: Option<u64>,
}

pub fn new_review_state() -> ReviewState {
    ReviewState {
        revision: 0,
        reviewed: None,
    }
}

pub fn review_revision(state: &ReviewState) -> u64 {
    state.revision
}

/// Refuse overflow rather than allowing a previous revision to become current.
pub fn mark_edited(state: &mut ReviewState) -> bool {
    if state.revision == u64::MAX {
        return false;
    }
    state.revision += 1;
    state.reviewed = None;
    true
}

pub fn accept_review_result(state: &mut ReviewState, analyzed: u64) -> bool {
    if !accept_review_revision(state.revision, analyzed) {
        return false;
    }
    state.reviewed = Some(analyzed);
    true
}

pub fn has_current_review(state: &ReviewState) -> bool {
    match state.reviewed {
        Some(analyzed) => accept_review_revision(state.revision, analyzed),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_boundaries_are_explicit() {
        for path in [
            "", "/a", "../a", "a/..", "a//b", "a/./b", "a/", "a\\b", "a\0b", "a\u{7f}b",
        ] {
            assert!(!valid_relative_path(path.as_bytes()), "{path:?}");
        }
        for path in [".", "a", "a/b", "λ/文件", ".hidden", "...", "a b"] {
            assert!(valid_relative_path(path.as_bytes()), "{path:?}");
        }
    }

    #[test]
    fn malformed_rights_never_authorize_requests() {
        for mask in 16..=u8::MAX {
            for permission in [
                Permission::Read,
                Permission::Write,
                Permission::Mkdir,
                Permission::Remove,
            ] {
                assert!(!allows(mask, permission));
            }
        }
        assert!(covers(3, 1));
        assert!(!covers(1, 3));
        assert!(!covers(15, 16));
    }

    #[test]
    fn unresolved_mandatory_conditions_stop_admission() {
        for status in [
            CheckStatus::Pending,
            CheckStatus::Mismatch,
            CheckStatus::Incomplete,
            CheckStatus::Unavailable,
        ] {
            assert!(!preflight_allows_update(&[
                CheckStatus::Passed,
                status,
                CheckStatus::Passed
            ]));
        }
        assert!(preflight_allows_update(&[]));
        assert!(preflight_allows_update(&[
            CheckStatus::Passed,
            CheckStatus::Passed
        ]));
    }
}
