import Lean

/-! The executable, intrinsically typed first-order kernel.

The theorems concern requests emitted by this kernel, relative to a granted
policy. They do not claim that an operating system implements its specification.
-/
namespace Esh

inductive Permission where
  | read | write | mkdir | remove
  deriving Repr, DecidableEq

def Permission.name : Permission → String
  | .read => "read" | .write => "write"
  | .mkdir => "mkdir" | .remove => "remove"

structure Effect where
  region : String
  permission : Permission
  deriving Repr, DecidableEq

def Effect.name (e : Effect) : String := s!"fs.{e.permission.name} {e.region}"
abbrev Effects := List Effect

structure EffectBound (actual allowed : Effects) : Type where
  included : ∀ e ∈ actual, e ∈ allowed

/-- A successful finite subset check carries its proof into the executable. -/
def certifySubset : (actual allowed : Effects) → Option (EffectBound actual allowed)
  | [], _ => some ⟨by simp⟩
  | first :: rest, allowed =>
      if h : first ∈ allowed then
        match certifySubset rest allowed with
        | none => none
        | some tail => some ⟨by
            intro e he
            rcases List.mem_cons.mp he with rfl | member
            · exact h
            · exact tail.included e member⟩
      else none

structure FsError where
  code : String
  message : String
  deriving Repr, BEq

def validComponent (s : String) : Bool :=
  !s.isEmpty && s != "." && s != ".." && !s.contains '\\' &&
    s.toList.all (fun c => c.toNat ≥ 32 && c.toNat != 127)

structure RelPath where
  parts : List String
  valid : parts.all validComponent = true
  deriving Repr

def RelPath.parse (s : String) : Except String RelPath :=
  let parts := if s == "." then [] else s.splitOn "/"
  if h : parts.all validComponent = true then .ok ⟨parts, h⟩
  else .error "expected a relative path: no empty components, '.', '..', backslashes or controls"

def RelPath.text (p : RelPath) : String :=
  if p.parts.isEmpty then "." else String.intercalate "/" p.parts

structure Capability where
  region : String
  permissions : List Permission
  deriving Repr

def Capability.restrict (cap : Capability) (required : List Permission)
    (_h : ∀ p ∈ required, p ∈ cap.permissions) : Capability :=
  { region := cap.region, permissions := required }

theorem restrict_no_amplification (cap : Capability) (required : List Permission)
    (h : ∀ p ∈ required, p ∈ cap.permissions) :
    ∀ p ∈ (cap.restrict required h).permissions, p ∈ cap.permissions := h

theorem restrict_same_region (cap : Capability) (required : List Permission)
    (h : ∀ p ∈ required, p ∈ cap.permissions) :
    (cap.restrict required h).region = cap.region := rfl

inductive Request : Type → Type where
  | read (region : String) (path : RelPath) : Request String
  | write (region : String) (path : RelPath) (contents : String) : Request Unit
  | list (region : String) (path : RelPath) : Request (List String)
  | mkdir (region : String) (path : RelPath) : Request Unit
  | remove (region : String) (path : RelPath) : Request Unit

def Request.effect : Request α → Effect
  | .read r _ | .list r _ => ⟨r, .read⟩
  | .write r _ _ => ⟨r, .write⟩
  | .mkdir r _ => ⟨r, .mkdir⟩
  | .remove r _ => ⟨r, .remove⟩

/-- A request needs evidence of authority. `weaken` can enlarge a summary only. -/
inductive Program (policy : Effects) : Effects → Type → Type 1 where
  | pure (value : α) : Program policy [] α
  | request (req : Request α) (allowed : req.effect ∈ policy) :
      Program policy [req.effect] (Except FsError α)
  | bind (first : Program policy e₁ α) (next : α → Program policy e₂ β) :
      Program policy (e₁ ++ e₂) β
  | weaken (program : Program policy e₁ α)
      (bound : ∀ e ∈ e₁, e ∈ e₂) : Program policy e₂ α

def Program.pureAt (value : α) : Program policy effects α :=
  .weaken (.pure value) (by simp)

def Program.map (f : α → β) (p : Program policy effects α) :
    Program policy effects β :=
  .weaken (.bind p (fun a => .pure (f a))) (by simp)

structure Execution (policy effects : Effects) (α : Type) where
  value : α
  trace : Effects
  bounded : ∀ e ∈ trace, e ∈ effects
  authorized : ∀ e ∈ trace, e ∈ policy

/-- The mock backend and the real bridge run this very same interpreter. -/
def Program.run {m : Type → Type} [Monad m]
    (handler : {α : Type} → Request α → m (Except FsError α)) :
    Program policy effects α → m (Execution policy effects α)
  | .pure value => Pure.pure ⟨value, [], by simp, by simp⟩
  | .request req allowed => do
      let value ← handler req
      Pure.pure ⟨value, [req.effect], by simp, by simpa using allowed⟩
  | .bind first next => do
      let a ← first.run handler
      let b ← (next a.value).run handler
      Pure.pure ⟨b.value, a.trace ++ b.trace,
        by
          intro e he
          rcases List.mem_append.mp he with h | h
          · exact List.mem_append_left _ (a.bounded e h)
          · exact List.mem_append_right _ (b.bounded e h),
        by
          intro e he
          rcases List.mem_append.mp he with h | h
          · exact a.authorized e h
          · exact b.authorized e h⟩
  | .weaken program bound => do
      let result ← program.run handler
      Pure.pure ⟨result.value, result.trace,
        fun e he => bound e (result.bounded e he), result.authorized⟩

theorem execution_effect_sound (run : Execution policy effects α) :
    ∀ e ∈ run.trace, e ∈ effects := run.bounded

theorem execution_authorized (run : Execution policy effects α) :
    ∀ e ∈ run.trace, e ∈ policy := run.authorized

theorem pure_trace_empty (run : Execution policy [] α) : run.trace = [] := by
  cases h : run.trace with
  | nil => rfl
  | cons e es =>
      have := run.bounded e (by simp [h])
      simp at this

inductive Ty where
  | unit | bool | text | path
  | list (element : Ty)
  | result (success : Ty)
  deriving Repr, BEq, DecidableEq

def Ty.name : Ty → String
  | .unit => "Unit" | .bool => "Bool" | .text => "Text" | .path => "RelPath"
  | .list t => s!"(List {t.name})"
  | .result t => s!"(Result {t.name} FsError)"

@[reducible] def Ty.Value : Ty → Type
  | .unit => Unit | .bool => Bool | .text => String | .path => RelPath
  | .list t => List t.Value | .result t => Except FsError t.Value

inductive Var : List Ty → Ty → Type where
  | here : Var (t :: ctx) t
  | there : Var ctx t → Var (s :: ctx) t

def Env : List Ty → Type
  | [] => Unit
  | t :: ctx => t.Value × Env ctx

def Var.get : Var ctx t → Env ctx → t.Value
  | .here, env => env.1
  | .there v, env => v.get env.2

inductive Expr (policy : Effects) : List Ty → Ty → Type where
  | text (s : String) : Expr policy ctx .text
  | bool (b : Bool) : Expr policy ctx .bool
  | unit : Expr policy ctx .unit
  | path (p : RelPath) : Expr policy ctx .path
  | var (v : Var ctx t) : Expr policy ctx t
  | letE (value : Expr policy ctx a) (body : Expr policy (a :: ctx) b) : Expr policy ctx b
  | letOk (value : Expr policy ctx (.result a))
      (body : Expr policy (a :: ctx) (.result b)) : Expr policy ctx (.result b)
  | ite (condition : Expr policy ctx .bool) (yes no : Expr policy ctx t) : Expr policy ctx t
  | concat (left right : Expr policy ctx .text) : Expr policy ctx .text
  | eqText (left right : Expr policy ctx .text) : Expr policy ctx .bool
  | ok (value : Expr policy ctx t) : Expr policy ctx (.result t)
  | error (type : Ty) (message : String) : Expr policy ctx (.result type)
  | read (region : String) (allowed : Effect.mk region .read ∈ policy)
      (path : Expr policy ctx .path) : Expr policy ctx (.result .text)
  | write (region : String) (allowed : Effect.mk region .write ∈ policy)
      (path : Expr policy ctx .path) (contents : Expr policy ctx .text) : Expr policy ctx (.result .unit)
  | list (region : String) (allowed : Effect.mk region .read ∈ policy)
      (path : Expr policy ctx .path) : Expr policy ctx (.result (.list .text))
  | mkdir (region : String) (allowed : Effect.mk region .mkdir ∈ policy)
      (path : Expr policy ctx .path) : Expr policy ctx (.result .unit)
  | remove (region : String) (allowed : Effect.mk region .remove ∈ policy)
      (path : Expr policy ctx .path) : Expr policy ctx (.result .unit)

def Expr.effects : Expr policy ctx t → Effects
  | .text _ | .bool _ | .unit | .path _ | .var _ | .error _ _ => []
  | .letE a b | .letOk a b | .concat a b | .eqText a b => a.effects ++ b.effects
  | .ite c a b => c.effects ++ (a.effects ++ b.effects)
  | .ok a => a.effects
  | .read r _ p | .list r _ p => p.effects ++ [⟨r, .read⟩]
  | .write r _ p value => p.effects ++ (value.effects ++ [⟨r, .write⟩])
  | .mkdir r _ p => p.effects ++ [⟨r, .mkdir⟩]
  | .remove r _ p => p.effects ++ [⟨r, .remove⟩]

def Expr.compile (expr : Expr policy ctx t) (env : Env ctx) :
    Program policy expr.effects t.Value :=
  match expr with
  | .text s => .pure s
  | .bool b => .pure b
  | .unit => .pure ()
  | .path p => .pure p
  | .var v => .pure (v.get env)
  | .letE value body => .bind (value.compile env) (fun v => body.compile (v, env))
  | .letOk value body => .bind (value.compile env) (fun result =>
      match result with
      | .ok v => body.compile (v, env)
      | .error e => .pureAt (.error e))
  | .ite condition yes no => .bind (condition.compile env) (fun b =>
      if b then .weaken (yes.compile env) (fun _ h => List.mem_append_left _ h)
      else .weaken (no.compile env) (fun _ h => List.mem_append_right _ h))
  | .concat a b => .bind (a.compile env) (fun x => (b.compile env).map (fun y => x ++ y))
  | .eqText a b => .bind (a.compile env) (fun x => (b.compile env).map (fun y => x == y))
  | .ok value => (value.compile env).map Except.ok
  | .error _ message => .pure (.error ⟨"user", message⟩)
  | .read r allowed pExpr => .bind (pExpr.compile env) (fun p => .request (.read r p) allowed)
  | .write r allowed pExpr contents => .bind (pExpr.compile env) (fun p =>
      .bind (contents.compile env) (fun s => .request (.write r p s) allowed))
  | .list r allowed pExpr => .bind (pExpr.compile env) (fun p => .request (.list r p) allowed)
  | .mkdir r allowed pExpr => .bind (pExpr.compile env) (fun p => .request (.mkdir r p) allowed)
  | .remove r allowed pExpr => .bind (pExpr.compile env) (fun p => .request (.remove r p) allowed)

end Esh
