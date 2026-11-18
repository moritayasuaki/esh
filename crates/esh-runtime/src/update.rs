use crate::Backend;
use esh_host::{
    CapabilityDeclaration, EntryKind, Host, HostError, Observation, Operation, Request,
};
use esh_lang::{
    compiler::Program,
    contracts::{Condition, Contracts, Predicate, Target},
    types::{rights_names, Effect, Region},
};
use esh_policy::{admit_mutation, may_report_success, preflight_allows_update, CheckStatus};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize)]
pub struct CheckRecord {
    pub phase: String,
    pub target: Target,
    pub predicate: Predicate,
    pub status: String,
    pub error: Option<HostError>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub operation: Operation,
    pub target: Target,
}
#[derive(Clone, Copy)]
pub struct CheckLimits {
    pub max_observations: usize,
    pub max_content_bytes: usize,
}
impl Default for CheckLimits {
    fn default() -> Self {
        Self {
            max_observations: 100_000,
            max_content_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct UpdateHost {
    pub host: Host,
    extra: Vec<Condition>,
    contracts: Contracts,
    current: Vec<Condition>,
    protected: Vec<(Target, Observation)>,
    pub checks: Vec<CheckRecord>,
    pub completed: Vec<Change>,
    limits: CheckLimits,
    observations: usize,
    content_bytes: usize,
    initial_passed: bool,
    stopped: Option<HostError>,
    finished: bool,
    effect_bounds: Vec<(String, u8)>,
}
impl UpdateHost {
    pub fn new(host: Host, extra: Vec<Condition>, limits: CheckLimits) -> Self {
        Self {
            host,
            extra,
            contracts: Contracts::default(),
            current: vec![],
            protected: vec![],
            checks: vec![],
            completed: vec![],
            limits,
            observations: 0,
            content_bytes: 0,
            initial_passed: false,
            stopped: None,
            finished: false,
            effect_bounds: vec![],
        }
    }
    fn observation(&mut self, t: &Target, contents: bool) -> Result<Observation, HostError> {
        self.check_effect(&t.region, esh_policy::Permission::Read)?;
        if self.observations >= self.limits.max_observations {
            return Err(HostError::new(
                "check-incomplete",
                "Mandatory validation exceeded max-checks.",
            ));
        }
        self.observations += 1;
        let out = self.host.observe(&t.region, &t.path, contents)?;
        if let Some(b) = &out.bytes {
            self.content_bytes = self.content_bytes.checked_add(b.len()).ok_or_else(|| {
                HostError::new(
                    "check-incomplete",
                    "Mandatory validation exceeded its byte budget.",
                )
            })?;
            if self.content_bytes > self.limits.max_content_bytes {
                return Err(HostError::new(
                    "check-incomplete",
                    "Mandatory validation exceeded max-check-bytes.",
                ));
            }
        }
        Ok(out)
    }
    fn check_effect(
        &mut self,
        region: &str,
        permission: esh_policy::Permission,
    ) -> Result<(), HostError> {
        if self
            .effect_bounds
            .iter()
            .any(|(r, mask)| r == region && esh_policy::allows(*mask, permission))
        {
            return Ok(());
        }
        self.fail(HostError::new(
            "effect-bound",
            "The operation is outside the checked script effect bound.",
        ))
    }
    fn validate(
        &mut self,
        conditions: &[Condition],
        phase: &str,
    ) -> Result<Vec<Observation>, HostError> {
        let mut statuses = vec![];
        let mut observations = vec![];
        let mut first = None;
        for condition in conditions {
            let contents = matches!(
                condition.predicate,
                Predicate::ExactText(_) | Predicate::ContentHash(_)
            );
            let observed = self.observation(&condition.target, contents);
            let (status, error) = match observed {
                Ok(o) => {
                    let matched = match &condition.predicate {
                        Predicate::RegularFile => o.kind == EntryKind::RegularFile,
                        Predicate::Directory => o.kind == EntryKind::Directory,
                        Predicate::Absent => o.kind == EntryKind::Absent,
                        Predicate::ExactText(text) => o.bytes.as_deref() == Some(text.as_bytes()),
                        Predicate::ContentHash(expected) => {
                            o.bytes.as_ref().is_some_and(|b| digest(b) == *expected)
                        }
                    };
                    observations.push(o);
                    if matched {
                        (CheckStatus::Passed, None)
                    } else {
                        (
                            CheckStatus::Mismatch,
                            Some(HostError::new(
                                "condition-mismatch",
                                "A mandatory filesystem condition does not hold.",
                            )),
                        )
                    }
                }
                Err(e) => {
                    let status = if matches!(e.code.as_str(), "check-incomplete" | "resource-limit")
                    {
                        CheckStatus::Incomplete
                    } else {
                        CheckStatus::Unavailable
                    };
                    (status, Some(e))
                }
            };
            let label = match status {
                CheckStatus::Passed => "passed",
                CheckStatus::Mismatch => "mismatch",
                CheckStatus::Incomplete => "incomplete",
                CheckStatus::Unavailable => "unavailable",
                CheckStatus::Pending => "pending",
            };
            self.checks.push(CheckRecord {
                phase: phase.into(),
                target: condition.target.clone(),
                predicate: condition.predicate.clone(),
                status: label.into(),
                error: error.clone(),
            });
            statuses.push(status);
            if first.is_none() {
                first = error;
            }
            // A mandatory failure already prevents admission; avoid useless extra I/O.
            if status != CheckStatus::Passed {
                break;
            }
        }
        if statuses.len() == conditions.len() && preflight_allows_update(&statuses) {
            Ok(observations)
        } else {
            Err(first.unwrap_or_else(|| {
                HostError::new("check-incomplete", "Mandatory validation did not finish.")
            }))
        }
    }
    fn fail<T>(&mut self, error: HostError) -> Result<T, HostError> {
        self.stopped = Some(error.clone());
        Err(error)
    }
    pub fn report(&self) -> Value {
        json!({"checks":self.checks,"completedChanges":self.completed,"rolledBack":false,"initialConditionsPassed":self.initial_passed,"postconditionsPassed":self.finished,
        "observations":self.observations,"observedContentBytes":self.content_bytes,
        "guarantees":{"conditional":true,"assumptions":["No external content or namespace changes to the specified targets during the update.","The operating system honors descriptor and filesystem syscall semantics."],"rechecksProveIsolation":false}})
    }
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn same_target(a: &Observation, b: &Observation) -> bool {
    a.address == b.address
}
fn overlaps(protected: &Observation, target: &Observation) -> bool {
    if same_target(protected, target) {
        return true;
    }
    if let Some(id) = protected.identity {
        if target.identity == Some(id) {
            return true;
        }
        if protected.kind == EntryKind::Directory && target.ancestors.contains(&id) {
            return true;
        }
    }
    protected.address.anchor == target.address.anchor
        && target.address.suffix.starts_with(&protected.address.suffix)
}
impl Backend for UpdateHost {
    fn halted(&self) -> bool {
        self.stopped.is_some()
    }
    fn begin(&mut self, p: &Program) -> Result<(), HostError> {
        if self.initial_passed || self.stopped.is_some() {
            return Err(HostError::new(
                "update-state",
                "An update session can be started only once.",
            ));
        }
        let declarations = p
            .capabilities
            .iter()
            .map(|(name, p)| CapabilityDeclaration {
                name: name.clone(),
                permissions: rights_names(*p).iter().map(|s| (*s).into()).collect(),
            })
            .collect::<Vec<_>>();
        self.host.authorize(&declarations)?;
        for effect in &p.effects {
            match effect {
                Effect::Fs(Region::Fixed(region), mask) => {
                    self.effect_bounds.push((region.clone(), *mask))
                }
                _ => {
                    return self.fail(HostError::new(
                        "effect-bound",
                        "Runtime effect bounds must have concrete regions and permissions.",
                    ))
                }
            }
        }
        self.contracts = p.contracts.clone();
        self.current = self.contracts.requires.clone();
        self.current.extend(self.extra.clone());
        let current = self.current.clone();
        if let Err(e) = self.validate(&current, "preflight") {
            return self.fail(e);
        }
        for t in self.contracts.preserves.clone() {
            let o = match self.observation(&t, false) {
                Ok(o) => o,
                Err(e) => return self.fail(e),
            };
            if o.kind == EntryKind::Other {
                return self.fail(HostError::new(
                    "preserves-unavailable",
                    "A protected target must be a regular file, directory, or absent path.",
                ));
            }
            self.protected.push((t, o));
        }
        self.initial_passed = true;
        Ok(())
    }
    fn finish(&mut self) -> Result<(), HostError> {
        if let Some(e) = &self.stopped {
            return Err(e.clone());
        }
        if !self.initial_passed {
            return Err(HostError::new(
                "update-state",
                "Preflight has not completed.",
            ));
        }
        let ensures = self.contracts.ensures.clone();
        match self.validate(&ensures, "postcondition") {
            Ok(_) => {
                self.finished = may_report_success(true, true);
                Ok(())
            }
            Err(e) => self.fail(e),
        }
    }
    fn request(&mut self, request: &Request) -> Result<Value, HostError> {
        if let Some(e) = &self.stopped {
            return Err(e.clone());
        }
        if !self.initial_passed || self.finished {
            return Err(HostError::new(
                "update-state",
                "Operation is outside an active update.",
            ));
        }
        self.check_effect(&request.region, request.operation.permission())?;
        let mutation = matches!(
            request.operation,
            Operation::Write | Operation::WriteBytes | Operation::Mkdir | Operation::Remove
        );
        if !mutation {
            return self.host.perform(request);
        }
        let current = self.current.clone();
        let observed = match self.validate(&current, "before-mutation") {
            Ok(o) => o,
            Err(e) => return self.fail(e),
        };
        let target = self.host.mutation_target(request)?;
        for (protected, baseline) in self.protected.clone() {
            let now = match self.observation(&protected, false) {
                Ok(o) => o,
                Err(e) => return self.fail(e),
            };
            if now.identity != baseline.identity
                || now.kind != baseline.kind
                || now.address != baseline.address
            {
                return self.fail(HostError::new(
                    "preserves-drift",
                    "A protected target changed since preflight.",
                ));
            }
            if !admit_mutation(self.initial_passed, true, overlaps(&now, &target)) {
                return self.fail(HostError::new(
                    "preserves",
                    "This operation would change a protected target or subtree.",
                ));
            }
        }
        if !admit_mutation(self.initial_passed, true, false) {
            return self.fail(HostError::new("update-state", "Mutation was not admitted."));
        }
        let result = self.host.perform(request)?;
        self.completed.push(Change {
            operation: request.operation,
            target: Target {
                region: request.region.clone(),
                path: request.path.clone(),
            },
        });
        // Evolve only facts whose directory entry this successful operation changes.
        // Never refresh an unrelated requirement from a newly observed external state.
        for (condition, old) in self.current.iter_mut().zip(&observed) {
            if same_target(old, &target) {
                condition.predicate = match request.operation {
                    Operation::Write => Predicate::ExactText(request.contents.clone().unwrap()),
                    Operation::WriteBytes => {
                        Predicate::ContentHash(digest(request.bytes.as_deref().unwrap()))
                    }
                    Operation::Mkdir => Predicate::Directory,
                    Operation::Remove => Predicate::Absent,
                    _ => unreachable!(),
                };
            }
        }
        Ok(result)
    }
}
