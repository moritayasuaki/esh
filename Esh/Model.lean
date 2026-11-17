import Esh.Core

/-! Laws for an abstract file-content store, not a verified POSIX filesystem.
Locations are resolved separately; different regions may resolve to the same key.
-/
namespace Esh.Model

abbrev Store := String → Option String

def read (store : Store) (location : String) : Except FsError String :=
  match store location with
  | some contents => .ok contents
  | none => .error ⟨"missing", "file does not exist"⟩

def write (store : Store) (location contents : String) : Store :=
  fun key => if key = location then some contents else store key

def remove (store : Store) (location : String) : Store :=
  fun key => if key = location then none else store key

theorem read_after_write (store : Store) (location contents : String) :
    read (write store location contents) location = .ok contents := by
  simp [read, write]

theorem write_frame (store : Store) (location other contents : String)
    (distinct : other ≠ location) :
    (write store location contents) other = store other := by
  simp [write, distinct]

theorem remove_absent (store : Store) (location : String) :
    (remove store location) location = none := by
  simp [remove]

theorem remove_frame (store : Store) (location other : String)
    (distinct : other ≠ location) :
    (remove store location) other = store other := by
  simp [remove, distinct]

theorem last_write_wins (store : Store) (location first second : String) :
    write (write store location first) location second = write store location second := by
  funext key
  by_cases same : key = location <;> simp [write, same]

end Esh.Model
