import Esh.Functions
import Esh.Bridge

namespace Esh.Functions
open Lean

def field (j : Json) (key : String) : Except String Json := j.getObjVal? key
def strField (j : Json) (key : String) : Except String String := j.getObjValAs? String key
def natField (j : Json) (key : String) : Except String Nat := j.getObjValAs? Nat key
def items (j : Json) : Except String (List Json) := (·.toList) <$> j.getArr?

def permission (s : String) : Except String Permission :=
  match s with
  | "read" => .ok .read | "write" => .ok .write
  | "mkdir" => .ok .mkdir | "remove" => .ok .remove
  | _ => .error "kernel: unknown permission"

def readRights (j : Json) : Except String (List Permission) := do
  (← items j).mapM (fun x => do permission (← x.getStr?))

def readRegion (j : Json) : Except String Region :=
  match j.getStr? with
  | .ok r => .ok (.fixed r)
  | .error _ => Region.param <$> fromJson? j

def readBound (j : Json) : Except String Bound := do
  let es ← (← items j).mapM fun e => do
    match natField e "var" with
    | .ok n => pure (.param n)
    | .error _ => pure (.fs (← readRegion (← field e "region"))
        (← permission (← strField e "permission")))
  return normalize es

partial def readType (types : Array Ty) (j : Json) : Except String Ty := do
  if let .ok n := natField j "ref" then
    match types[n]? with
    | some t => return t
    | none => throw "kernel: invalid or cyclic shared type reference"
  match j.getStr? with
  | .ok "Unit" => return .unit
  | .ok "Bool" => return .bool
  | .ok "Text" => return .text
  | .ok "RelPath" => return .path
  | .ok _ => throw "kernel: unknown type"
  | .error _ =>
    let tag ← strField j "tag"
    match tag with
    | "var" => return .param (← natField j "id")
    | "list" => return .list (← readType types (← field j "element"))
    | "result" => return .result (← readType types (← field j "success"))
    | "cap" => return .cap (← readRegion (← field j "region")) (← readRights (← field j "rights"))
    | "fn" =>
        let args ← (← items (← field j "args")).mapM (readType types)
        return .fn (pack args) (← readType types (← field j "result")) (← readBound (← field j "effects"))
    | _ => throw "kernel: unknown type constructor"

def pairs (j : Json) (read : Json → Except String α) : Except String (List (Nat × α)) := do
  (← items j).mapM fun row => do
    let [key, value] ← items row | throw "kernel: invalid substitution entry"
    return (← fromJson? key, ← read value)

def readSubst (types : Array Ty) (j : Json) : Except String Subst := do
  return ⟨← pairs (← field j "types") (readType types),
          ← pairs (← field j "regions") readRegion,
          ← pairs (← field j "effects") readBound⟩

def unpack : Ty → Except String (List Ty)
  | .argsNil => pure []
  | .argsCons t ts => return t :: (← unpack ts)
  | _ => throw "kernel: invalid function argument pack"

def lookup : (ctx : List Ty) → Nat → Option ((t : Ty) × Var ctx t)
  | [], _ => none
  | t :: _, 0 => some ⟨t, .here⟩
  | _ :: ctx, n + 1 => (lookup ctx n).map (fun ⟨t, v⟩ => ⟨t, .there v⟩)

def rightsCert : (required granted : List Permission) →
    Option (PLift (∀ p ∈ required, p ∈ granted))
  | [], _ => some ⟨by simp⟩
  | p :: ps, granted =>
      if h : p ∈ granted then
        match rightsCert ps granted with
        | none => none
        | some hs => some ⟨by
            intro x hx
            rcases List.mem_cons.mp hx with rfl | hx
            · exact h
            · exact hs.down x hx⟩
      else none

structure Packed (policy : Esh.Effects) (ctx : List Ty) where
  type : Ty
  effects : Bound
  expr : Expr policy ctx type effects

instance : Inhabited (Packed policy ctx) := ⟨⟨.unit, [], .val .unit⟩⟩

def expect (t : Ty) (p : Packed policy ctx) : Except String (Expr policy ctx t p.effects) :=
  if h : p.type = t then pure (h ▸ p.expr) else throw "kernel: type mismatch"

structure PackedArgs (policy : Esh.Effects) (ctx ts : List Ty) where
  effects : Bound
  exprs : Exprs policy ctx ts effects

instance : Inhabited (Except String (PackedArgs policy ctx ts)) := ⟨.error "kernel: invalid arguments"⟩

mutual
  partial def checkExpr (types : Array Ty) (policy : Esh.Effects) (ctx : List Ty) (j : Json) :
      Except String (Packed policy ctx) := do
    let op ← strField j "op"
    match op with
    | "unit" => return ⟨.unit, [], .val .unit⟩
    | "text" => return ⟨.text, [], .val (.text (← strField j "value"))⟩
    | "bool" => return ⟨.bool, [], .val (.bool (← j.getObjValAs? Bool "value"))⟩
    | "path" => return ⟨.path, [], .val (.path (← RelPath.parse (← strField j "value")))⟩
    | "var" =>
        let some ⟨t, v⟩ := lookup ctx (← natField j "index") | throw "kernel: unbound variable"
        return ⟨t, [], .var v⟩
    | "cap" =>
        let r ← strField j "region"
        let ps ← readRights (← field j "rights")
        let some proof := Esh.certifySubset (ps.map (Esh.Effect.mk r)) policy
          | throw "kernel: capability exceeds policy"
        return ⟨.cap (.fixed r) ps, [], .val (.cap r ps (fun p hp =>
          proof.included _ (List.mem_map.mpr ⟨p, hp, rfl⟩)))⟩
    | "let" =>
        let v ← checkExpr types policy ctx (← field j "value")
        let b ← checkExpr types policy (v.type :: ctx) (← field j "body")
        return ⟨b.type, v.effects ++ b.effects, .letE v.expr b.expr⟩
    | "let-ok" =>
        let v ← checkExpr types policy ctx (← field j "value")
        match v with
        | ⟨.result t, ev, v⟩ =>
            let b ← checkExpr types policy (t :: ctx) (← field j "body")
            match b with
            | ⟨.result u, eb, b⟩ => return ⟨.result u, ev ++ eb, .letOk v b⟩
            | _ => throw "kernel: let-ok body must return Result"
        | _ => throw "kernel: let-ok requires Result"
    | "if" =>
        let c ← checkExpr types policy ctx (← field j "condition")
        let a ← checkExpr types policy ctx (← field j "yes")
        let b ← checkExpr types policy ctx (← field j "no")
        return ⟨a.type, c.effects ++ (a.effects ++ b.effects), .ite (← expect .bool c) a.expr (← expect a.type b)⟩
    | "concat" | "eq" =>
        let a ← checkExpr types policy ctx (← field j "left")
        let b ← checkExpr types policy ctx (← field j "right")
        let x ← expect .text a
        let y ← expect .text b
        if op == "concat" then return ⟨.text, a.effects ++ b.effects, .concat x y⟩
        else return ⟨.bool, a.effects ++ b.effects, .eqText x y⟩
    | "ok" =>
        let v ← checkExpr types policy ctx (← field j "value")
        return ⟨.result v.type, v.effects, .ok v.expr⟩
    | "error" =>
        let t ← readType types (← field j "type")
        return ⟨.result t, [], .error t (← strField j "message")⟩
    | "fn" =>
        let args ← (← items (← field j "args")).mapM (readType types)
        let captureTypes ← (← items (← field j "captureTypes")).mapM (readType types)
        let captures ← checkArgs types policy ctx captureTypes (← items (← field j "captures"))
        let ret ← readType types (← field j "result")
        let bound ← readBound (← field j "effects")
        let body ← checkExpr types policy (args ++ captureTypes) (← field j "body")
        let some h := certify body.effects bound | throw "kernel: function body exceeds its effect contract"
        if hc : captures.effects = [] then
          return ⟨.fn (pack args) ret bound, [], .lam (hc ▸ captures.exprs) (← expect ret body) bound h.down⟩
        else throw "kernel: capturing a value must not execute effects"
    | "call" =>
        let f ← checkExpr types policy ctx (← field j "function")
        match f with
        | ⟨.fn domain ret latent, ef, f⟩ =>
            let args ← unpack domain
            if h : domain = pack args then
              let values ← checkArgs types policy ctx args (← items (← field j "args"))
              return ⟨ret, ef ++ (values.effects ++ latent), .app (h ▸ f) values.exprs⟩
            else throw "kernel: malformed argument pack"
        | _ => throw "kernel: callee is not a function"
    | "inst" =>
        let v ← checkExpr types policy ctx (← field j "value")
        if h : v.effects = [] then
          let s ← readSubst types (← field j "subst")
          return ⟨v.type.subst s, [], .inst (h ▸ v.expr) s⟩
        else throw "kernel: instantiation must reference an evaluated value"
    | "narrow" =>
        let v ← checkExpr types policy ctx (← field j "value")
        let ps ← readRights (← field j "rights")
        match v with
        | ⟨.cap r granted, e, x⟩ =>
            let some h := rightsCert ps granted | throw "kernel: authority amplification"
            return ⟨.cap r ps, e, .narrow x ps h.down⟩
        | _ => throw "kernel: expected capability"
    | "widen" =>
        let v ← checkExpr types policy ctx (← field j "value")
        let bound ← readBound (← field j "effects")
        match v with
        | ⟨.fn args ret actual, e, x⟩ =>
            let some h := certify actual bound | throw "kernel: invalid effect widening"
            return ⟨.fn args ret bound, e, .widen x bound h.down⟩
        | _ => throw "kernel: expected function"
    | "read" | "write" | "list" | "mkdir" | "remove" =>
        let c ← checkExpr types policy ctx (← field j "cap")
        let p ← checkExpr types policy ctx (← field j "path")
        let path ← expect .path p
        match c with
        | ⟨.cap _ ps, _, c⟩ =>
            let right ← permission (if op == "list" then "read" else op)
            let some proof := rightsCert [right] ps | throw "kernel: missing permission"
            let cap := Expr.narrow c [right] proof.down
            match right, cap with
            | .read, cap =>
                if op == "list" then return ⟨.result (.list .text), _, .list cap path⟩
                else return ⟨.result .text, _, .read cap path⟩
            | .write, cap =>
                let t ← checkExpr types policy ctx (← field j "text")
                return ⟨.result .unit, _, .write cap path (← expect .text t)⟩
            | .mkdir, cap => return ⟨.result .unit, _, .mkdir cap path⟩
            | .remove, cap => return ⟨.result .unit, _, .remove cap path⟩
        | _ => throw "kernel: filesystem operation requires a capability"
    | _ => throw s!"kernel: unknown expression {op}"

  partial def checkArgs (types : Array Ty) (policy : Esh.Effects) (ctx : List Ty) :
      (ts : List Ty) → List Json → Except String (PackedArgs policy ctx ts)
    | [], [] => pure ⟨[], .nil⟩
    | t :: ts, x :: xs => do
        let a ← checkExpr types policy ctx x
        let b ← checkArgs types policy ctx ts xs
        return ⟨a.effects ++ b.effects, .cons (← expect t a) b.exprs⟩
    | _, _ => throw "kernel: wrong number of arguments"
end

def valueJson : Value policy t → Json
  | .unit => Json.null
  | .bool b => toJson b
  | .text s => toJson s
  | .path p => toJson p.text
  | .nil => toJson ([] : List Json)
  | .cons v vs => match valueJson vs with
      | .arr xs => .arr (#[valueJson v] ++ xs)
      | _ => .arr #[valueJson v]
  | .ok v => Json.mkObj [("ok", valueJson v)]
  | .error e => Json.mkObj [("error", Esh.errorJson e)]
  | .cap r ps _ => Json.mkObj [("capability", toJson r), ("permissions", toJson (ps.map Permission.name))]
  | .closure _ _ _ => Json.mkObj [("function", toJson true)]

def failed : Value policy t → Bool
  | .error _ => true
  | _ => false

structure Bundle where
  policy : Esh.Effects
  program : Packed policy []
  report : Json

def readBundle (j : Json) : Except String Bundle := do
  let mut types : Array Ty := #[]
  for encoded in (← items ((field j "types").toOption.getD (.arr #[]))) do
    types := types.push (← readType types encoded)
  let policy ← (← items (← field j "policy")).mapM fun x => do
    pure (Esh.Effect.mk (← strField x "region") (← permission (← strField x "permission")))
  let p ← checkExpr types policy [] (← field j "program")
  let expected ← readType types (← field j "type")
  let _ ← expect expected p
  let bound ← readBound (← field j "bound")
  let some _ := certify p.effects bound | throw "kernel: script exceeds its effect contract"
  return ⟨policy, p, ← field j "report"⟩

def runBundle (input : Json) (execute : Bool) : IO UInt32 := do
  let emit (j : Json) := do
    let out ← IO.getStdout
    out.putStrLn j.compress
    out.flush
  match readBundle input with
  | .error message =>
      emit (Json.mkObj [("ok", toJson false), ("error", toJson message)])
      return 2
  | .ok bundle =>
      if !execute then
        emit bundle.report
        return 0
      emit (Json.mkObj [("kind", toJson "ready"), ("analysis", bundle.report)])
      match ← (bundle.program.expr.eval Esh.bridgeHandler .nil).run with
      | .error message =>
          emit (Json.mkObj [("ok", toJson false), ("error", toJson message)])
          return 2
      | .ok result =>
          emit (Json.mkObj [("kind", toJson "result"),
            ("type", (bundle.report.getObjVal? "type").toOption.getD Json.null),
            ("value", valueJson result.value),
            ("trace", toJson (result.trace.map Esh.effectJson)), ("traceLength", toJson result.trace.length)])
          return if failed result.value then 1 else 0

end Esh.Functions
