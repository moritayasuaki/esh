import EshRust.Generated.Funs

open Aeneas Aeneas.Std Result

namespace EshRust

def isPassed : CheckStatus → Bool
  | .Passed => true
  | _ => false

theorem status_comparison (s : CheckStatus) :
    core.cmp.PartialEq.ne.trait_default CheckStatus.Insts.CoreCmpPartialEqCheckStatus
      s CheckStatus.Passed = ok (!isPassed s) := by
  cases s <;> simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    CheckStatus.Insts.CoreCmpPartialEqCheckStatus.eq, CheckStatus.read_discriminant, isPassed]

/-- Every mandatory check has passed whenever the extracted loop admits an update.
The specification also establishes termination without an indexing/overflow failure. -/
theorem preflight_total_and_sound (checks : Slice CheckStatus) :
    preflight_allows_update checks ⦃ b =>
      b = true → ∀ j, j < checks.length → checks.val[j]? = some CheckStatus.Passed ⦄ := by
  unfold preflight_allows_update preflight_allows_update_loop
  apply loop.spec_decr_nat (fun i => checks.length - i.val)
    (fun i => i.val ≤ checks.length ∧ ∀ j, j < i.val → checks.val[j]? = some CheckStatus.Passed)
  · intro i inv
    unfold preflight_allows_update_loop.body
    simp only [Slice.len]
    split
    · rename_i hi
      have hi' : i.val < checks.length := by simpa using hi
      apply WP.spec_bind (Slice.index_usize_spec checks i hi')
      intro status hstatus
      rw [status_comparison]
      cases status <;> simp only [isPassed, Bool.not_false, Bool.not_true]
      all_goals try simp
      step as ⟨next, hnext⟩
      refine ⟨?_, ?_, ?_⟩
      · scalar_tac
      · intro j hj
        by_cases same : j = i.val
        · subst j
          simpa [List.getElem?_eq_getElem hi'] using congrArg some hstatus.symm
        · apply inv.2
          scalar_tac
      · scalar_tac
    · simp only [WP.spec_ok]
      intro _ j hj
      apply inv.2
      scalar_tac
  · simp

/-- The extracted Rust guard is total and implements the stated admission rule. -/
theorem admission_rule (initial rechecked blocked : Bool) :
    admit_mutation initial rechecked blocked = ok (initial && rechecked && !blocked) := by
  cases initial <;> cases rechecked <;> cases blocked <;> rfl

/-- No mutation is admitted without both completed checks and preservation permission. -/
theorem admitted_requires_checks (initial rechecked blocked : Bool)
    (h : admit_mutation initial rechecked blocked = ok true) :
    initial = true ∧ rechecked = true ∧ blocked = false := by
  rw [admission_rule] at h
  simpa using h

theorem success_rule (body postconditions : Bool) :
    may_report_success body postconditions = ok (body && postconditions) := by
  cases body <;> cases postconditions <;> rfl

theorem success_requires_postconditions (body postconditions : Bool)
    (h : may_report_success body postconditions = ok true) :
    body = true ∧ postconditions = true := by
  rw [success_rule] at h
  simpa using h

/-- Source extraction preserves the permission assigned to each command class. -/
theorem permission_bit_spec (permission : Permission) :
    permission_bit permission = ok (match permission with
      | .Read => 1#u8 | .Write => 2#u8 | .Mkdir => 4#u8 | .Remove => 8#u8) := by
  cases permission <;> rfl

/-- An old analysis can never certify a different document revision. -/
theorem review_requires_current_revision (current analyzed : U64)
    (h : accept_review_revision current analyzed = ok true) : current = analyzed := by
  simpa [accept_review_revision] using h

theorem current_review_is_accepted (revision : U64) :
    accept_review_revision revision revision = ok true := by
  simp [accept_review_revision]

/-- Rights are four independent permission bits; no high bits are accepted. -/
theorem valid_rights_rule (rights : U8) :
    valid_rights rights = ok (decide (rights.val &&& 240 = 0)) := by
  simp [valid_rights, lift, UScalar.eq_equiv, UScalar.val_and]

/-- Restriction checks both masks and requires every requested bit to be granted. -/
theorem covers_rule (granted required : U8) :
    covers granted required = ok (decide
      (granted.val &&& 240 = 0 ∧ required.val &&& 240 = 0 ∧
       granted.val &&& required.val = required.val)) := by
  simp [covers, valid_rights_rule, lift, UScalar.eq_equiv, UScalar.val_and]
  split <;> simp_all
  split <;> simp_all

def safeComponentByte (byte : U8) : Prop :=
  32 ≤ byte.val ∧ byte.val ≠ 127 ∧ byte.val ≠ 47 ∧ byte.val ≠ 92

/-- An accepted component contains no separator, backslash, DEL, or ASCII control.
The extracted scan terminates without an indexing or arithmetic failure. -/
theorem component_bytes_total_and_sound (bytes : Slice U8) :
    valid_component_bytes bytes ⦃ accepted => accepted = true →
      ∀ j, j < bytes.length → ∃ byte, bytes.val[j]? = some byte ∧ safeComponentByte byte ⦄ := by
  unfold valid_component_bytes valid_component_bytes_loop
  apply loop.spec_decr_nat (fun i => bytes.length - i.val)
    (fun i => i.val ≤ bytes.length ∧ ∀ j, j < i.val →
      ∃ byte, bytes.val[j]? = some byte ∧ safeComponentByte byte)
  · intro i inv
    unfold valid_component_bytes_loop.body
    simp only [Slice.len]
    split
    · rename_i hi
      have hi' : i.val < bytes.length := by simpa using hi
      apply WP.spec_bind (Slice.index_usize_spec bytes i hi')
      intro byte hbyte
      split
      · simp
      · split
        · simp
        · split
          · simp
          · split
            · simp
            · have safe : safeComponentByte byte := by
                unfold safeComponentByte
                scalar_tac
              step as ⟨next, hnext⟩
              refine ⟨?_, ?_, ?_⟩
              · scalar_tac
              · intro j hj
                by_cases same : j = i.val
                · subst j
                  refine ⟨byte, ?_, safe⟩
                  simpa [List.getElem?_eq_getElem hi'] using congrArg some hbyte.symm
                · apply inv.2
                  scalar_tac
              · scalar_tac
    · simp only [WP.spec_ok]
      intro _ j hj
      apply inv.2
      scalar_tac
  · simp

end EshRust
