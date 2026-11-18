import Esh.Bridge
import Esh.Model
import Esh.Functions

/-! Build this module to kernel-check the assurance entry points. -/
namespace Esh

example (p : Program policy effects α)
    (handler : {β : Type} → Request β → Id (Except FsError β)) :
    ∀ e ∈ (p.run handler).trace, e ∈ effects := (p.run handler).bounded

example (p : Program policy effects α)
    (handler : {β : Type} → Request β → Id (Except FsError β)) :
    ∀ e ∈ (p.run handler).trace, e ∈ policy := (p.run handler).authorized

example (script : Script)
    (handler : {β : Type} → Request β → Id (Except FsError β)) :
    ∀ e ∈ (script.program.run handler).trace, e ∈ script.bound :=
  (script.program.run handler).bounded

#print axioms restrict_no_amplification
#print axioms execution_effect_sound
#print axioms execution_authorized
#print axioms pure_trace_empty
#print axioms Model.read_after_write
#print axioms Model.write_frame
#print axioms Model.remove_absent
#print axioms Model.remove_frame
#print axioms Model.last_write_wins
#print axioms Functions.execution_effect_sound
#print axioms Functions.execution_authorized
#print axioms Functions.pure_trace_empty
#print axioms Functions.append_preserves_trace
#print axioms Functions.subst_included
#print axioms Functions.substitution_keeps_capability
#print axioms Functions.mem_normalize

end Esh
