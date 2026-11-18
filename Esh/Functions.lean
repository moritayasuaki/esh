import Esh.Core

/-! Typed higher-order computations. Type instantiation transforms an already
evaluated value; it never evaluates its source binding again. -/
namespace Esh.Functions

inductive Region where
  | fixed (name : String)
  | param (id : Nat)
  deriving Repr, DecidableEq

inductive Atom where
  | fs (region : Region) (permission : Permission)
  | param (id : Nat)
  deriving Repr, DecidableEq

abbrev Bound := List Atom
abbrev Included (a b : Bound) := ∀ e ∈ a, e ∈ b

def Atom.key : Atom → String
  | .param n => s!"effect:{n}"
  | .fs (.param n) p => s!"region:{n}:{p.name}"
  | .fs (.fixed r) p => s!"fixed:{p.name}:{reprStr r}"

def normalize (es : Bound) : Bound :=
  es.eraseDups.mergeSort (fun a b => decide (a.key ≤ b.key))

@[simp] theorem mem_normalize : e ∈ normalize es ↔ e ∈ es := by
  simp [normalize, List.mem_eraseDups]

theorem include_normalize_right (h : Included a b) : Included a (normalize b) :=
  fun e he => mem_normalize.mpr (h e he)

theorem include_normalize_both (h : Included a b) : Included (normalize a) (normalize b) :=
  fun e he => mem_normalize.mpr (h e (mem_normalize.mp he))

def atom (e : Esh.Effect) : Atom := .fs (.fixed e.region) e.permission

def certify : (a b : Bound) → Option (PLift (Included a b))
  | [], _ => some ⟨by intro e he; cases he⟩
  | x :: xs, b =>
      if h : x ∈ b then
        match certify xs b with
        | none => none
        | some hs => some ⟨by
            intro e he
            rcases List.mem_cons.mp he with rfl | he
            · exact h
            · exact hs.down e he⟩
      else none

inductive Ty where
  | unit | bool | text | path
  | param (id : Nat)
  | list (element : Ty)
  | result (success : Ty)
  | cap (region : Region) (rights : List Permission)
  | argsNil
  | argsCons (first rest : Ty)
  | fn (args : Ty) (result : Ty) (effects : Bound)
  deriving Repr, DecidableEq

@[reducible] def pack : List Ty → Ty
  | [] => .argsNil
  | t :: ts => .argsCons t (pack ts)

theorem pack_injective : ∀ {as bs : List Ty}, pack as = pack bs → as = bs := by
  intro as
  induction as with
  | nil => intro bs h; cases bs <;> simp_all [pack]
  | cons a as ih =>
      intro bs h
      cases bs with
      | nil => simp [pack] at h
      | cons b bs =>
          have h' := Ty.argsCons.inj h
          rw [h'.1, ih h'.2]

structure Subst where
  types : List (Nat × Ty) := []
  regions : List (Nat × Region) := []
  effects : List (Nat × Bound) := []
  deriving Repr

@[reducible] def Region.subst (s : Subst) : Region → Region
  | .fixed r => .fixed r
  | .param n => (s.regions.lookup n).getD (.param n)

@[reducible] def Atom.subst (s : Subst) : Atom → Bound
  | .fs r p => [.fs (r.subst s) p]
  | .param n => (s.effects.lookup n).getD [.param n]

@[reducible] def substBound (s : Subst) (es : Bound) : Bound := es.flatMap (Atom.subst s)

@[reducible] def Ty.subst (s : Subst) : Ty → Ty
  | .unit => .unit | .bool => .bool | .text => .text | .path => .path
  | .param n => (s.types.lookup n).getD (.param n)
  | .list a => .list (a.subst s)
  | .result a => .result (a.subst s)
  | .cap r ps => .cap (r.subst s) ps
  | .argsNil => .argsNil
  | .argsCons t ts => .argsCons (t.subst s) (ts.subst s)
  | .fn args ret es => .fn (args.subst s) (ret.subst s) (normalize (substBound s es))

theorem subst_pack (s : Subst) (args : List Ty) :
    (pack args).subst s = pack (args.map (Ty.subst s)) := by
  induction args with
  | nil => rfl
  | cons t ts ih => simp [pack, Ty.subst, ih]

theorem subst_included (s : Subst) (h : Included a b) :
    Included (substBound s a) (substBound s b) := by
  intro e he
  obtain ⟨x, hx, hex⟩ := List.mem_flatMap.mp he
  exact List.mem_flatMap.mpr ⟨x, h x hx, hex⟩

theorem subst_append (s : Subst) (a b : Bound) :
    substBound s (a ++ b) = substBound s a ++ substBound s b := by
  simp [substBound]

inductive Var : List Ty → Ty → Type where
  | here : Var (t :: ctx) t
  | there : Var ctx t → Var (u :: ctx) t

def Var.subst (s : Subst) : Var ctx t → Var (ctx.map (Ty.subst s)) (t.subst s)
  | .here => .here
  | .there v => .there (v.subst s)

mutual
  inductive Value (policy : Esh.Effects) : Ty → Type where
    | unit : Value policy .unit
    | bool (b : Bool) : Value policy .bool
    | text (s : String) : Value policy .text
    | path (p : RelPath) : Value policy .path
    | nil : Value policy (.list t)
    | cons : Value policy t → Value policy (.list t) → Value policy (.list t)
    | ok : Value policy t → Value policy (.result t)
    | error (e : FsError) : Value policy (.result t)
    | cap (r : String) (ps : List Permission)
        (authorized : ∀ p ∈ ps, Esh.Effect.mk r p ∈ policy) :
        Value policy (.cap (.fixed r) ps)
    | closure (body : Expr policy (args ++ ctx) ret actual)
        (env : Values policy ctx) (allowed : Included actual bound) :
        Value policy (.fn (pack args) ret bound)

  inductive Values (policy : Esh.Effects) : List Ty → Type where
    | nil : Values policy []
    | cons : Value policy t → Values policy ts → Values policy (t :: ts)

  inductive Expr (policy : Esh.Effects) : List Ty → Ty → Bound → Type where
    | val : Value policy t → Expr policy ctx t []
    | var : Var ctx t → Expr policy ctx t []
    | letE : Expr policy ctx a e₁ → Expr policy (a :: ctx) b e₂ →
        Expr policy ctx b (e₁ ++ e₂)
    | letOk : Expr policy ctx (.result a) e₁ → Expr policy (a :: ctx) (.result b) e₂ →
        Expr policy ctx (.result b) (e₁ ++ e₂)
    | ite : Expr policy ctx .bool ec → Expr policy ctx t ea → Expr policy ctx t eb →
        Expr policy ctx t (ec ++ (ea ++ eb))
    | concat : Expr policy ctx .text ea → Expr policy ctx .text eb →
        Expr policy ctx .text (ea ++ eb)
    | eqText : Expr policy ctx .text ea → Expr policy ctx .text eb →
        Expr policy ctx .bool (ea ++ eb)
    | ok : Expr policy ctx t e → Expr policy ctx (.result t) e
    | error (t : Ty) (message : String) : Expr policy ctx (.result t) []
    | lam (captures : Exprs policy ctx captured [])
        (body : Expr policy (args ++ captured) ret actual) (bound : Bound)
        (allowed : Included actual bound) : Expr policy ctx (.fn (pack args) ret bound) []
    | app : Expr policy ctx (.fn (pack args) ret latent) ef → Exprs policy ctx args ea →
        Expr policy ctx ret (ef ++ (ea ++ latent))
    | inst : Expr policy ctx t [] → (s : Subst) → Expr policy ctx (t.subst s) []
    | narrow : Expr policy ctx (.cap r granted) e → (required : List Permission) →
        (∀ p ∈ required, p ∈ granted) → Expr policy ctx (.cap r required) e
    | widen : Expr policy ctx (.fn args ret actual) e → (bound : Bound) →
        Included actual bound → Expr policy ctx (.fn args ret bound) e
    | weaken : Expr policy ctx t actual → (bound : Bound) →
        Included actual bound → Expr policy ctx t bound
    | read : Expr policy ctx (.cap r [.read]) ec → Expr policy ctx .path ep →
        Expr policy ctx (.result .text) (ec ++ (ep ++ [.fs r .read]))
    | write : Expr policy ctx (.cap r [.write]) ec → Expr policy ctx .path ep →
        Expr policy ctx .text et →
        Expr policy ctx (.result .unit) (ec ++ (ep ++ (et ++ [.fs r .write])))
    | list : Expr policy ctx (.cap r [.read]) ec → Expr policy ctx .path ep →
        Expr policy ctx (.result (.list .text)) (ec ++ (ep ++ [.fs r .read]))
    | mkdir : Expr policy ctx (.cap r [.mkdir]) ec → Expr policy ctx .path ep →
        Expr policy ctx (.result .unit) (ec ++ (ep ++ [.fs r .mkdir]))
    | remove : Expr policy ctx (.cap r [.remove]) ec → Expr policy ctx .path ep →
        Expr policy ctx (.result .unit) (ec ++ (ep ++ [.fs r .remove]))

  inductive Exprs (policy : Esh.Effects) : List Ty → List Ty → Bound → Type where
    | nil : Exprs policy ctx [] []
    | cons : Expr policy ctx t e → Exprs policy ctx ts es → Exprs policy ctx (t :: ts) (e ++ es)
end

def Values.get : Var ctx t → Values policy ctx → Value policy t
  | .here, .cons v _ => v
  | .there i, .cons _ vs => vs.get i

def Values.append : Values policy as → Values policy bs → Values policy (as ++ bs)
  | .nil, bs => bs
  | .cons a rest, bs => .cons a (rest.append bs)

def Value.narrow : Value policy (.cap r granted) → (required : List Permission) →
    (∀ p ∈ required, p ∈ granted) → Value policy (.cap r required)
  | .cap r _ authorized, required, h => .cap r required (fun p hp => authorized p (h p hp))

def Value.widen : Value policy (.fn args ret actual) → (bound : Bound) →
    Included actual bound → Value policy (.fn args ret bound)
  | .closure body env h, _, h' => .closure body env (fun e he => h' e (h e he))

structure Execution (policy : Esh.Effects) (bound : Bound) (α : Type) where
  value : α
  trace : Esh.Effects
  bounded : ∀ e ∈ trace, atom e ∈ bound
  authorized : ∀ e ∈ trace, e ∈ policy

def Execution.pure (value : α) : Execution policy bound α :=
  ⟨value, [], by simp, by simp⟩

def Execution.map (f : α → β) (x : Execution policy e α) : Execution policy e β :=
  { x with value := f x.value }

def Execution.weaken (x : Execution policy a α) (h : Included a b) : Execution policy b α :=
  { x with bounded := fun e he => h _ (x.bounded e he) }

def Execution.append (x : Execution policy a α) (y : Execution policy b β) :
    Execution policy (a ++ b) β :=
  ⟨y.value, x.trace ++ y.trace, by
    intro e he
    rcases List.mem_append.mp he with h | h
    · exact List.mem_append_left _ (x.bounded e h)
    · exact List.mem_append_right _ (y.bounded e h), by
    intro e he
    rcases List.mem_append.mp he with h | h
    · exact x.authorized e h
    · exact y.authorized e h⟩

theorem execution_effect_sound (x : Execution policy bound α) :
    ∀ e ∈ x.trace, atom e ∈ bound := x.bounded

theorem execution_authorized (x : Execution policy bound α) :
    ∀ e ∈ x.trace, e ∈ policy := x.authorized

theorem pure_trace_empty (x : Execution policy [] α) : x.trace = [] := by
  cases h : x.trace with
  | nil => rfl
  | cons e es =>
      have impossible := x.bounded e (by simp [h])
      simp at impossible

theorem append_preserves_trace (x : Execution policy a α) (y : Execution policy b β) :
    (x.append y).trace = x.trace ++ y.trace := rfl

def castExpr (x : Expr policy ctx t e) (ctx' : List Ty) (t' : Ty) (e' : Bound) :
    Except String (Expr policy ctx' t' e') := do
  if hc : ctx = ctx' then
    if ht : t = t' then
      if he : e = e' then return hc ▸ ht ▸ he ▸ x
      else
        match certify e e' with
        | some h => return .weaken (hc ▸ ht ▸ x) e' h.down
        | none => throw "kernel: effect substitution mismatch"
    else throw "kernel: type substitution mismatch"
  else throw "kernel: environment substitution mismatch"

def Subst.mapRanges (outer inner : Subst) : Subst :=
  ⟨inner.types.map (fun (n, t) => (n, t.subst outer)),
   inner.regions.map (fun (n, r) => (n, r.subst outer)),
   inner.effects.map (fun (n, e) => (n, substBound outer e))⟩

mutual
  def Value.subst (s : Subst) : Value policy t → Except String (Value policy (t.subst s))
    | .unit => pure .unit
    | .bool b => pure (.bool b)
    | .text x => pure (.text x)
    | .path p => pure (.path p)
    | .nil => pure .nil
    | .cons v vs => return .cons (← v.subst s) (← vs.subst s)
    | .ok v => return .ok (← v.subst s)
    | .error e => pure (.error e)
    | .cap r ps h => pure (.cap r ps h)
    | .closure body env h => by
        simp only [Ty.subst, subst_pack]
        exact do
          let b ← body.subst s
          let b ← castExpr b _ _ _
          return .closure b (← env.subst s) (include_normalize_right (subst_included s h))

  def Values.subst (s : Subst) : Values policy ctx →
      Except String (Values policy (ctx.map (Ty.subst s)))
    | .nil => pure .nil
    | .cons v vs => return .cons (← v.subst s) (← vs.subst s)

  def Expr.subst (s : Subst) : Expr policy ctx t e →
      Except String (Expr policy (ctx.map (Ty.subst s)) (t.subst s) (substBound s e))
    | .val v => return .val (← v.subst s)
    | .var v => pure (.var (v.subst s))
    | .letE a b => do
        let x := Expr.letE (← a.subst s) (← b.subst s)
        castExpr x _ _ _
    | .letOk a b => do
        let x := Expr.letOk (← a.subst s) (← b.subst s)
        castExpr x _ _ _
    | .ite c a b => do
        let x := Expr.ite (← c.subst s) (← a.subst s) (← b.subst s)
        castExpr x _ _ _
    | .concat a b => do
        let x := Expr.concat (← a.subst s) (← b.subst s)
        castExpr x _ _ _
    | .eqText a b => do
        let x := Expr.eqText (← a.subst s) (← b.subst s)
        castExpr x _ _ _
    | .ok a => return .ok (← a.subst s)
    | .error t m => pure (.error (t.subst s) m)
    | .lam captures body bound h => by
        simp only [Ty.subst, subst_pack]
        exact do
          let b ← body.subst s
          let b ← castExpr b _ _ _
          return .lam (← captures.subst s) b (normalize (substBound s bound)) (include_normalize_right (subst_included s h))
    | @Expr.app _ _ args ret latent ef _ f values => do
        let f' ← f.subst s
        let f' ← castExpr f' _ (.fn (pack (args.map (Ty.subst s)))
          (ret.subst s) (normalize (substBound s latent))) (substBound s ef)
        let x := Expr.app f' (← values.subst s)
        castExpr x _ _ _
    | .inst x inner => do
        let v ← x.subst s
        castExpr (.inst v (s.mapRanges inner)) _ _ _
    | .narrow x ps h => return .narrow (← x.subst s) ps h
    | .widen x bound h => return .widen (← x.subst s) (normalize (substBound s bound)) (include_normalize_both (subst_included s h))
    | .weaken x bound h => return .weaken (← x.subst s) (substBound s bound) (subst_included s h)
    | .read c p => do
        let x := Expr.read (← c.subst s) (← p.subst s)
        castExpr x _ _ _
    | .write c p t => do
        let x := Expr.write (← c.subst s) (← p.subst s) (← t.subst s)
        castExpr x _ _ _
    | .list c p => do
        let x := Expr.list (← c.subst s) (← p.subst s)
        castExpr x _ _ _
    | .mkdir c p => do
        let x := Expr.mkdir (← c.subst s) (← p.subst s)
        castExpr x _ _ _
    | .remove c p => do
        let x := Expr.remove (← c.subst s) (← p.subst s)
        castExpr x _ _ _

  def Exprs.subst (s : Subst) : Exprs policy ctx ts e →
      Except String (Exprs policy (ctx.map (Ty.subst s)) (ts.map (Ty.subst s)) (substBound s e))
    | .nil => pure .nil
    | .cons x xs => by
        simp only [subst_append]
        exact do return .cons (← x.subst s) (← xs.subst s)
end

theorem substitution_keeps_capability (s : Subst) (r : String) (ps : List Permission)
    (h : ∀ p ∈ ps, Esh.Effect.mk r p ∈ policy) :
    (Value.cap r ps h).subst s = Except.ok (Value.cap r ps h) := rfl

def Value.ofStrings : List String → Value policy (.list .text)
  | [] => .nil
  | x :: xs => .cons (.text x) (ofStrings xs)

def Value.ofResult (f : α → Value policy t) : Except FsError α → Value policy (.result t)
  | .ok x => .ok (f x)
  | .error e => .error e

structure Closure (policy : Esh.Effects) (domain ret : Ty) (bound : Bound) where
  args : List Ty
  domainEq : pack args = domain
  ctx : List Ty
  effects : Bound
  body : Expr policy (args ++ ctx) ret effects
  env : Values policy ctx
  allowed : Included effects bound

def Value.asClosure : Value policy (.fn domain ret bound) → Closure policy domain ret bound
  | .closure body env h => ⟨_, rfl, _, _, body, env, h⟩

structure CapValue (policy : Esh.Effects) (region : Region) (rights : List Permission) where
  name : String
  regionEq : Region.fixed name = region
  authorized : ∀ p ∈ rights, Esh.Effect.mk name p ∈ policy

def Value.asCap : Value policy (.cap r ps) → CapValue policy r ps
  | .cap r _ h => ⟨r, rfl, h⟩

abbrev Handler := {α : Type} → Esh.Request α → IO (Except FsError α)
abbrev RunM := ExceptT String IO

def request (handler : Handler) (req : Esh.Request α) (h : req.effect ∈ policy)
    (convert : α → Value policy t) :
    RunM (Execution policy [atom req.effect] (Value policy (.result t))) := do
  let result ← liftM ((Esh.Program.request req h).run (m := IO) handler)
  return ⟨Value.ofResult convert result.value, result.trace,
    fun e he => by
      have eq := List.mem_singleton.mp (result.bounded e he)
      simp [eq], result.authorized⟩

/- The interpreter is partial only in its evaluation strategy (higher-order
application is not structural recursion). Every returned value and every
request still passes through the indexed constructors above. No fuel cutoff is
exposed as a language error. Termination and finite-prefix refinement are
separate proof obligations from the completed-execution bounds. -/
mutual
  partial def Expr.eval (handler : Handler) :
      {ctx : List Ty} → {t : Ty} → {e : Bound} → Expr policy ctx t e →
      Values policy ctx → RunM (Execution policy e (Value policy t))
    | _, _, _, .val v, env => do return .pure v
    | _, _, _, .var v, env => do return .pure (env.get v)
    | _, _, _, .letE a b, env => do
        let av ← a.eval handler env
        let bv ← b.eval handler (.cons av.value env)
        return av.append bv
    | _, _, _, .letOk a b, env => do
        let av ← a.eval handler env
        match av.value with
        | .error err => return av.append (.pure (.error err))
        | .ok v => return av.append (← b.eval handler (.cons v env))
    | _, _, _, .ite c a b, env => do
        let cv ← c.eval handler env
        let .bool yes := cv.value
        if yes then
          let av ← a.eval handler env
          return cv.append (av.weaken (fun _ h => List.mem_append_left _ h))
        else
          let bv ← b.eval handler env
          return cv.append (bv.weaken (fun _ h => List.mem_append_right _ h))
    | _, _, _, .concat a b, env => do
        let av ← a.eval handler env
        let bv ← b.eval handler env
        let .text x := av.value
        let .text y := bv.value
        return (av.append bv).map (fun _ => .text (x ++ y))
    | _, _, _, .eqText a b, env => do
        let av ← a.eval handler env
        let bv ← b.eval handler env
        let .text x := av.value
        let .text y := bv.value
        return (av.append bv).map (fun _ => .bool (x == y))
    | _, _, _, .ok a, env => do return (← a.eval handler env).map Value.ok
    | _, _, _, .error _ message, env => do return .pure (.error ⟨"user", message⟩)
    | _, _, _, .lam captures body _ h, env => do
        let captured ← captures.eval handler env
        return captured.map (fun values => .closure body values h)
    | _, _, _, .app f args, env => do
        let fv ← f.eval handler env
        let av ← args.eval handler env
        let cl := fv.value.asClosure
        have argsEq := pack_injective cl.domainEq
        let values := argsEq.symm ▸ av.value
        let result ← cl.body.eval handler (values.append cl.env)
        return fv.append (av.append (result.weaken cl.allowed))
    | _, _, _, .inst x s, env => do
        let v ← x.eval handler env
        match v.value.subst s with
        | .ok value => return v.map (fun _ => value)
        | .error message => throw message
    | _, _, _, .narrow x ps h, env => do return (← x.eval handler env).map (fun v => v.narrow ps h)
    | _, _, _, .widen x bound h, env => do return (← x.eval handler env).map (fun v => v.widen bound h)
    | _, _, _, .weaken x _ h, env => do return (← x.eval handler env).weaken h
    | _, _, _, .read c p, env => do
        let cv ← c.eval handler env
        let pv ← p.eval handler env
        let cap := cv.value.asCap
        let .path path := pv.value
        let result ← request handler (.read cap.name path) (cap.authorized .read (by simp)) Value.text
        return cv.append (pv.append (by simpa [atom, Esh.Request.effect, cap.regionEq] using result))
    | _, _, _, .write c p t, env => do
        let cv ← c.eval handler env
        let pv ← p.eval handler env
        let tv ← t.eval handler env
        let cap := cv.value.asCap
        let .path path := pv.value
        let .text text := tv.value
        let result ← request handler (.write cap.name path text) (cap.authorized .write (by simp)) (fun _ => .unit)
        return cv.append (pv.append (tv.append (by simpa [atom, Esh.Request.effect, cap.regionEq] using result)))
    | _, _, _, .list c p, env => do
        let cv ← c.eval handler env
        let pv ← p.eval handler env
        let cap := cv.value.asCap
        let .path path := pv.value
        let result ← request handler (.list cap.name path) (cap.authorized .read (by simp)) Value.ofStrings
        return cv.append (pv.append (by simpa [atom, Esh.Request.effect, cap.regionEq] using result))
    | _, _, _, .mkdir c p, env => do
        let cv ← c.eval handler env
        let pv ← p.eval handler env
        let cap := cv.value.asCap
        let .path path := pv.value
        let result ← request handler (.mkdir cap.name path) (cap.authorized .mkdir (by simp)) (fun _ => .unit)
        return cv.append (pv.append (by simpa [atom, Esh.Request.effect, cap.regionEq] using result))
    | _, _, _, .remove c p, env => do
        let cv ← c.eval handler env
        let pv ← p.eval handler env
        let cap := cv.value.asCap
        let .path path := pv.value
        let result ← request handler (.remove cap.name path) (cap.authorized .remove (by simp)) (fun _ => .unit)
        return cv.append (pv.append (by simpa [atom, Esh.Request.effect, cap.regionEq] using result))

  partial def Exprs.eval (handler : Handler) :
      {ctx : List Ty} → {ts : List Ty} → {e : Bound} → Exprs policy ctx ts e →
      Values policy ctx → RunM (Execution policy e (Values policy ts))
    | _, _, _, .nil, _ => do return .pure .nil
    | _, _, _, .cons x xs, env => do
        let v ← x.eval handler env
        let vs ← xs.eval handler env
        return (v.append vs).map (fun tail => .cons v.value tail)
end

end Esh.Functions
