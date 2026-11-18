import EshRust.PolicyProofs

open Aeneas Aeneas.Std Result

namespace EshRust

/-- A path component is nonempty, is neither dot marker, and contains only
bytes permitted by the extracted byte validator. -/
def safeComponentList (bytes : List U8) : Prop :=
  0 < bytes.length ∧ bytes ≠ [46#u8] ∧ bytes ≠ [46#u8, 46#u8] ∧
    ∀ j, j < bytes.length → ∃ byte, bytes[j]? = some byte ∧ safeComponentByte byte

abbrev safeComponent (bytes : Slice U8) : Prop := safeComponentList bytes.val

/-- The complete component validator is total and accepts only safe components. -/
theorem component_total_and_sound (bytes : Slice U8) :
    valid_component bytes ⦃ accepted => accepted = true → safeComponent bytes ⦄ := by
  unfold valid_component
  step as ⟨empty, hempty⟩
  simp only [hempty]
  split
  · simp
  · rename_i nonempty
    split
    · rename_i one
      step as ⟨first, hfirst⟩
      split
      · simp
      · rename_i notdot
        split
        · scalar_tac
        · apply WP.spec_mono (component_bytes_total_and_sound bytes)
          intro accepted safe haccepted
          refine ⟨by scalar_tac, ?_, ?_, safe haccepted⟩
          · intro h
            simp_all [Slice.length]
          · intro h
            have := congrArg List.length h
            scalar_tac
    · rename_i notone
      split
      · rename_i two
        step as ⟨first, hfirst⟩
        split
        · rename_i dot
          step as ⟨second, hsecond⟩
          split
          · simp
          · rename_i notdot
            apply WP.spec_mono (component_bytes_total_and_sound bytes)
            intro accepted safe haccepted
            refine ⟨by scalar_tac, ?_, ?_, safe haccepted⟩
            · intro h
              have := congrArg List.length h
              scalar_tac
            · intro h
              simp_all [Slice.length]
        · rename_i notdot
          apply WP.spec_mono (component_bytes_total_and_sound bytes)
          intro accepted safe haccepted
          refine ⟨by scalar_tac, ?_, ?_, safe haccepted⟩
          · intro h
            have := congrArg List.length h
            scalar_tac
          · intro h
            simp_all [Slice.length]
      · rename_i nottwo
        apply WP.spec_mono (component_bytes_total_and_sound bytes)
        intro accepted safe haccepted
        refine ⟨by scalar_tac, ?_, ?_, safe haccepted⟩
        · intro h
          have := congrArg List.length h
          scalar_tac
        · intro h
          have := congrArg List.length h
          scalar_tac

/-- One or more safe components separated by exactly one slash. This describes
the byte sequence, independently of the implementation's indices or slices. -/
inductive SafeRelativeComponents : List U8 → Prop where
  | last {component} : safeComponentList component → SafeRelativeComponents component
  | prefix {component rest} : safeComponentList component → SafeRelativeComponents rest →
      SafeRelativeComponents (component ++ 47#u8 :: rest)

/-- The scanning loop preserves a safe completed prefix. It terminates without
out-of-bounds indexing, invalid slicing, or arithmetic overflow. -/
theorem path_loop_total_and_sound (path : Slice U8) :
    valid_relative_path_loop0 path 0#usize 0#usize ⦃ accepted =>
      accepted = true → SafeRelativeComponents path.val ⦄ := by
  unfold valid_relative_path_loop0
  apply loop.spec_decr_nat (fun (_, i) => path.length - i.val)
    (fun (start, i) => start.val ≤ i.val ∧ i.val ≤ path.length ∧
      ∀ rest, SafeRelativeComponents rest →
        SafeRelativeComponents (path.val.take start.val ++ rest))
  · rintro ⟨start, i⟩ ⟨hstart, hbound, prefix_ok⟩
    unfold valid_relative_path_loop0.body
    simp only [Slice.len]
    split
    · rename_i before_end
      have hi : i.val < path.length := by simpa using before_end
      apply WP.spec_bind (Slice.index_usize_spec path i hi)
      intro byte hbyte
      split
      · rename_i separator
        step as ⟨chunk, hchunk, hlength⟩
        apply WP.spec_bind (component_total_and_sound chunk)
        intro accepted hsafe
        split
        · rename_i yes
          have good := hsafe yes
          step as ⟨next, hnext⟩
          refine ⟨le_refl _, ?_, ?_, ?_⟩
          · scalar_tac
          · intro rest hrest
            have extended := prefix_ok (chunk.val ++ 47#u8 :: rest)
              (SafeRelativeComponents.prefix good hrest)
            have split_prefix : path.val.take next.val =
                path.val.take start.val ++ chunk.val ++ [47#u8] := by
              rw [hnext, List.take_succ_eq_append_getElem hi, hchunk, List.slice,
                ← List.take_add, Nat.add_sub_of_le hstart, ← hbyte, separator]
            simpa [split_prefix, List.append_assoc] using extended
          · scalar_tac
        · simp
      · step as ⟨next, hnext⟩
        exact ⟨by scalar_tac, by scalar_tac, prefix_ok, by scalar_tac⟩
    · step as ⟨chunk, hchunk, hlength⟩
      apply WP.spec_bind (component_total_and_sound chunk)
      intro accepted hsafe
      simp only [WP.spec_ok]
      intro yes
      have complete := prefix_ok chunk.val (SafeRelativeComponents.last (hsafe yes))
      simpa [hchunk] using complete
  · simp

/-- Every accepted path is the explicit root marker or a sequence of safe
components. Root-marker permission remains the caller's responsibility. -/
theorem relative_path_total_and_sound (path : Slice U8) :
    valid_relative_path path ⦃ accepted => accepted = true →
      path.val = [46#u8] ∨ SafeRelativeComponents path.val ⦄ := by
  unfold valid_relative_path
  simp only [Slice.len]
  split
  · rename_i one
    step as ⟨first, hfirst⟩
    split
    · rename_i dot
      simp only [WP.spec_ok]
      intro _
      left
      have length_one : path.val.length = 1 := by scalar_tac
      obtain ⟨byte, hbytes⟩ := List.length_eq_one_iff.mp length_one
      simp_all
    · step as ⟨empty, hempty⟩
      split
      · simp
      · apply WP.spec_mono (path_loop_total_and_sound path)
        intro accepted safe yes
        exact Or.inr (safe yes)
  · step as ⟨empty, hempty⟩
    split
    · simp
    · have same : valid_relative_path_loop1 path 0#usize 0#usize =
          valid_relative_path_loop0 path 0#usize 0#usize := rfl
      rw [same]
      apply WP.spec_mono (path_loop_total_and_sound path)
      intro accepted safe yes
      exact Or.inr (safe yes)

end EshRust
