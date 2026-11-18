import EshRust.PolicyProofs

open Aeneas Aeneas.Std Result
namespace EshRust

theorem new_document_requires_review :
    (do let state ← new_review_state; has_current_review state) = ok false := by
  simp [new_review_state, has_current_review]

/-- The actual Rust edit transition invalidates the review and advances the
revision, or refuses the edit at the counter limit without wrapping. -/
theorem edit_transition_total (state : ReviewState) :
    mark_edited state ⦃ pair =>
      (pair.1 = true → pair.2.reviewed = none ∧
        pair.2.revision.val = state.revision.val + 1) ∧
      (pair.1 = false → pair.2 = state ∧ state.revision = core.num.U64.MAX) ⦄ := by
  unfold mark_edited
  split
  · simp_all [WP.spec_ok]
  · step as ⟨next, hnext⟩
    · simp_all [U64.rMax, U64.max]
      scalar_tac
    · constructor
      · intro _
        simpa using hnext
      · intro impossible
        cases impossible

theorem acceptance_transition (state : ReviewState) (analyzed : U64) :
    accept_review_result state analyzed =
      if state.revision = analyzed
      then ok (true, { state with reviewed := some analyzed })
      else ok (false, state) := by
  simp [accept_review_result, accept_review_revision]

theorem stale_result_leaves_state_unchanged (state : ReviewState) (analyzed : U64)
    (different : state.revision ≠ analyzed) :
    accept_review_result state analyzed = ok (false, state) := by
  simp [acceptance_transition, different]

theorem accepted_result_is_current (state : ReviewState) :
    (do let (_, next) ← accept_review_result state state.revision
        has_current_review next) = ok true := by
  simp [acceptance_transition, has_current_review, accept_review_revision]

end EshRust
