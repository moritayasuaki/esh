import Esh.Syntax

namespace Esh
open Lean

structure CapDecl where
  cap : Capability
  infer : Bool
  span : Span
  deriving Repr

def CapDecl.effects (c : CapDecl) : Effects :=
  c.cap.permissions.map (Effect.mk c.cap.region)

def effectJson (e : Effect) : Json := Json.mkObj [
  ("region", toJson e.region), ("operation", toJson s!"fs.{e.permission.name}")]

def effectsJson (es : Effects) : Json := toJson (es.eraseDups.map effectJson)

structure NodeInfo where
  span : Span
  kind : String
  typeName : String
  effects : Effects
  details : Json := Json.null

def NodeInfo.json (n : NodeInfo) : Json := Json.mkObj [
  ("span", n.span.json), ("kind", toJson n.kind), ("type", toJson n.typeName),
  ("effects", effectsJson n.effects), ("details", n.details)]

abbrev CheckM := StateT (Array NodeInfo) (Except Diagnostic)

def failAt (s : Span) (code message : String) : CheckM α :=
  throw ⟨s, code, message⟩

structure Packed (policy : Effects) (ctx : List Ty) where
  type : Ty
  expr : Expr policy ctx type

instance : Inhabited (Packed policy ctx) := ⟨⟨.unit, .unit⟩⟩

def expected (span : Span) (type : Ty) (p : Packed policy ctx) : CheckM (Expr policy ctx type) :=
  if h : p.type = type then pure (h ▸ p.expr)
  else failAt span "type-mismatch" s!"expected {type.name}, got {p.type.name}"

def record (form : Form) (kind : String) (p : Packed policy ctx)
    (details : Json := Json.null) : CheckM (Packed policy ctx) := do
  modify (·.push ⟨form.span, kind, p.type.name, p.expr.effects, details⟩)
  pure p

def parsePermission (f : Form) : Except Diagnostic Permission :=
  match f with
  | .atom _ "read" => .ok .read
  | .atom _ "write" => .ok .write
  | .atom _ "mkdir" => .ok .mkdir
  | .atom _ "remove" => .ok .remove
  | _ => .error ⟨f.span, "permission", "expected read, write, mkdir or remove"⟩

partial def parseType (f : Form) : Except Diagnostic Ty :=
  match f with
  | .atom _ "Unit" => .ok .unit
  | .atom _ "Bool" => .ok .bool
  | .atom _ "Text" => .ok .text
  | .atom _ "RelPath" => .ok .path
  | .list _ [.atom _ "List", t] => .list <$> parseType t
  | .list _ [.atom _ "Result", t, .atom _ "FsError"] => .result <$> parseType t
  | _ => .error ⟨f.span, "unsupported-type", "v0 supports Unit, Bool, Text, RelPath, (List T), (Result T FsError)"⟩

def lookupVar (name : String) : (ctx : List (String × Ty)) →
    Option ((t : Ty) × Var (ctx.map Prod.snd) t)
  | [] => none
  | (n, t) :: ctx =>
      if n == name then some ⟨t, .here⟩
      else (lookupVar name ctx).map (fun ⟨ty, v⟩ => ⟨ty, .there v⟩)

structure ResolvedCap (policy : Effects) (permission : Permission) where
  region : String
  allowed : Effect.mk region permission ∈ policy
  details : Json

def resolveCap (policy : Effects) (caps : List CapDecl) (permission : Permission)
    (form : Form) : CheckM (ResolvedCap policy permission) := do
  let .atom span name := form
    | failAt form.span "capability" "expected a capability name or '_'"
  let candidates := if name == "_" then
      caps.filter (fun c => c.infer && c.cap.permissions.contains permission)
    else caps.filter (fun c => c.cap.region == name)
  let cap ← match candidates with
    | [] => failAt span "capability-missing" s!"no eligible capability for {permission.name}: {name}"
    | [c] => pure c
    | cs => failAt span "capability-ambiguous" s!"ambiguous capability: {String.intercalate ", " (cs.map (·.cap.region))}; specify one explicitly"
  if hasRight : permission ∈ cap.cap.permissions then
    if allowed : Effect.mk cap.cap.region permission ∈ policy then
      let narrowed := cap.cap.restrict [permission] (by
        intro p hp
        have same : p = permission := List.mem_singleton.mp hp
        simpa [same] using hasRight)
      let details := Json.mkObj [
        ("region", toJson narrowed.region), ("inferred", toJson (name == "_")),
        ("granted", toJson (cap.cap.permissions.map Permission.name)),
        ("required", toJson (narrowed.permissions.map Permission.name)),
        ("narrowed", toJson (cap.cap.permissions != narrowed.permissions)),
        ("capabilitySpan", span.json)]
      pure ⟨narrowed.region, allowed, details⟩
    else failAt span "permission-denied" "capability is outside the script policy"
  else failAt span "permission-denied" s!"{cap.cap.region} does not grant {permission.name}"

private def checkBinder (caps : List CapDecl) (form : Form) : CheckM String := do
  let .atom s name := form | failAt form.span "binding" "expected a binding name"
  if name == "_" || name == "true" || name == "false" || name == "unit" ||
      caps.any (fun c => c.cap.region == name) then
    failAt s "binding" "binding name is reserved or conflicts with a capability"
  else pure name

partial def infer (policy : Effects) (caps : List CapDecl)
    (ctx : List (String × Ty)) (form : Form) : CheckM (Packed policy (ctx.map Prod.snd)) := do
  match form with
  | .string _ s => record form "literal" ⟨.text, .text s⟩
  | .atom _ "true" => record form "literal" ⟨.bool, .bool true⟩
  | .atom _ "false" => record form "literal" ⟨.bool, .bool false⟩
  | .atom _ "unit" => record form "literal" ⟨.unit, .unit⟩
  | .atom s name =>
      match lookupVar name ctx with
      | some ⟨t, v⟩ => record form "variable" ⟨t, .var v⟩ (Json.mkObj [("name", toJson name)])
      | none => failAt s "unknown-variable" s!"unknown variable: {name}"
  | .list s items =>
      match items with
      | [.atom _ "path", .string _ text] =>
          let p ← (RelPath.parse text).mapError (fun msg => Diagnostic.mk s "path" msg)
          record form "path" ⟨.path, .path p⟩
      | [.atom _ "let", binder, value, body] =>
          let name ← checkBinder caps binder
          let v ← infer policy caps ctx value
          let b ← infer policy caps ((name, v.type) :: ctx) body
          record form "let" ⟨b.type, .letE v.expr b.expr⟩ (Json.mkObj [("name", toJson name)])
      | [.atom _ "let-ok", binder, value, body] =>
          let name ← checkBinder caps binder
          let v ← infer policy caps ctx value
          match v with
          | ⟨.result a, valueExpr⟩ =>
              let b ← infer policy caps ((name, a) :: ctx) body
              match b with
              | ⟨.result t, bodyExpr⟩ =>
                  record form "let-ok" ⟨.result t, .letOk valueExpr bodyExpr⟩
                    (Json.mkObj [("name", toJson name), ("onError", toJson "return Err; skip body")])
              | other => failAt body.span "type-mismatch" s!"let-ok body must return Result; got {other.type.name}"
          | other => failAt value.span "type-mismatch" s!"let-ok requires Result; got {other.type.name}"
      | [.atom _ "if", condition, yes, no] =>
          let c ← expected condition.span .bool (← infer policy caps ctx condition)
          let a ← infer policy caps ctx yes
          let b ← expected no.span a.type (← infer policy caps ctx no)
          record form "if" ⟨a.type, .ite c a.expr b⟩
      | [.atom _ "text.concat", left, right] =>
          let a ← expected left.span .text (← infer policy caps ctx left)
          let b ← expected right.span .text (← infer policy caps ctx right)
          record form "text.concat" ⟨.text, .concat a b⟩
      | [.atom _ "text.eq", left, right] =>
          let a ← expected left.span .text (← infer policy caps ctx left)
          let b ← expected right.span .text (← infer policy caps ctx right)
          record form "text.eq" ⟨.bool, .eqText a b⟩
      | [.atom _ "ok", value] =>
          let v ← infer policy caps ctx value
          record form "ok" ⟨.result v.type, .ok v.expr⟩
      | [.atom _ "error", ty, .string _ message] =>
          let t ← parseType ty
          record form "error" ⟨.result t, .error t message⟩
      | [.atom _ "fs.read-text", capForm, path] =>
          let cap ← resolveCap policy caps .read capForm
          let p ← expected path.span .path (← infer policy caps ctx path)
          record form "fs.read-text" ⟨.result .text, .read cap.region cap.allowed p⟩ cap.details
      | [.atom _ "fs.write-text", capForm, path, contents] =>
          let cap ← resolveCap policy caps .write capForm
          let p ← expected path.span .path (← infer policy caps ctx path)
          let value ← expected contents.span .text (← infer policy caps ctx contents)
          record form "fs.write-text" ⟨.result .unit, .write cap.region cap.allowed p value⟩ cap.details
      | [.atom _ "fs.list", capForm, path] =>
          let cap ← resolveCap policy caps .read capForm
          let p ← expected path.span .path (← infer policy caps ctx path)
          record form "fs.list" ⟨.result (.list .text), .list cap.region cap.allowed p⟩ cap.details
      | [.atom _ "fs.mkdir", capForm, path] =>
          let cap ← resolveCap policy caps .mkdir capForm
          let p ← expected path.span .path (← infer policy caps ctx path)
          record form "fs.mkdir" ⟨.result .unit, .mkdir cap.region cap.allowed p⟩ cap.details
      | [.atom _ "fs.remove", capForm, path] =>
          let cap ← resolveCap policy caps .remove capForm
          let p ← expected path.span .path (← infer policy caps ctx path)
          record form "fs.remove" ⟨.result .unit, .remove cap.region cap.allowed p⟩ cap.details
      | .atom _ op :: _ => failAt s "unsupported-form" s!"unknown form or wrong arity: {op}; functions, recursion and external commands are not implemented in v0"
      | _ => failAt s "syntax" "expected an operator followed by arguments"

structure Script where
  caps : List CapDecl
  policy : Effects
  type : Ty
  body : Expr policy [] type
  declared : Option Effects
  bound : Effects
  boundSound : ∀ e ∈ body.effects, e ∈ bound
  nodes : Array NodeInfo

def Script.program (script : Script) : Program script.policy script.bound script.type.Value :=
  .weaken (script.body.compile ()) script.boundSound

def Script.json (script : Script) (source : String) : Json := Json.mkObj [
  ("ok", toJson true), ("version", toJson "0.1"), ("source", toJson source),
  ("type", toJson script.type.name), ("effects", effectsJson script.body.effects),
  ("declaredEffects", match script.declared with | none => Json.null | some es => effectsJson es),
  ("capabilities", toJson (script.caps.map fun c => Json.mkObj [
    ("name", toJson c.cap.region), ("permissions", toJson (c.cap.permissions.map Permission.name)),
    ("infer", toJson c.infer), ("span", c.span.json)])),
  ("nodes", toJson (script.nodes.map NodeInfo.json)),
  ("scope", toJson "static effect upper bound; not a concrete execution plan")]

def parseCap (form : Form) : Except Diagnostic CapDecl := do
  let (span, name, perms, auto) ← match form with
    | .list s [.atom _ "cap", .atom _ n, .list _ ps] => pure (s, n, ps, false)
    | .list s [.atom _ "cap", .atom _ n, .list _ ps, .atom _ "infer"] => pure (s, n, ps, true)
    | _ => throw ⟨form.span, "capability", "expected (cap name (permissions...) [infer])"⟩
  if name == "_" || name.isEmpty then
    throw ⟨span, "capability", "invalid capability name"⟩
  let permissions ← perms.mapM parsePermission
  if permissions.eraseDups.length != permissions.length then
    throw ⟨span, "capability", "duplicate permission"⟩
  pure ⟨⟨name, permissions⟩, auto, span⟩

def parseEffect (caps : List CapDecl) (form : Form) : Except Diagnostic Effect := do
  let .list span [.atom _ op, .atom _ region] := form
    | throw ⟨form.span, "effect", "expected (fs.operation region)"⟩
  let permission ← match op with
    | "fs.read" => pure Permission.read
    | "fs.write" => pure Permission.write
    | "fs.mkdir" => pure Permission.mkdir
    | "fs.remove" => pure Permission.remove
    | _ => throw ⟨span, "effect", s!"unsupported effect: {op}"⟩
  if !(caps.any fun c => c.cap.region == region && c.cap.permissions.contains permission) then
    throw ⟨span, "effect", s!"effect is outside declared capabilities: {op} {region}"⟩
  pure ⟨region, permission⟩

def check (source : String) : Except Diagnostic Script := do
  let root ← parse source
  let .list rootSpan (.atom _ "script" :: forms) := root
    | throw ⟨root.span, "script", "expected (script declarations... expression)"⟩
  let some body := forms.getLast? | throw ⟨rootSpan, "script", "script has no body"⟩
  let headers := forms.dropLast
  let mut caps : List CapDecl := []
  let mut returnType : Option Ty := none
  let mut effectForm : Option (List Form) := none
  for header in headers do
    match header with
    | .list _ (.atom _ "cap" :: _) =>
        let cap ← parseCap header
        if caps.any (fun c => c.cap.region == cap.cap.region) then
          throw ⟨header.span, "capability", "duplicate capability name"⟩
        caps := caps ++ [cap]
    | .list _ [.atom _ "returns", type] =>
        if returnType.isSome then throw ⟨header.span, "annotation", "duplicate returns annotation"⟩
        returnType := some (← parseType type)
    | .list _ (.atom _ "effects" :: effects) =>
        if effectForm.isSome then throw ⟨header.span, "annotation", "duplicate effects annotation"⟩
        effectForm := some effects
    | _ => throw ⟨header.span, "declaration", "expected cap, returns or effects declaration"⟩
  let policy := caps.flatMap CapDecl.effects
  let (packed, nodes) ← (infer policy caps [] body).run #[]
  if let some expectedType := returnType then
    if packed.type != expectedType then
      throw ⟨body.span, "return-type", s!"expected {expectedType.name}, got {packed.type.name}"⟩
  let declared ← effectForm.mapM (·.mapM (parseEffect caps))
  if let some allowed := declared then
    for effect in packed.expr.effects do
      if !allowed.contains effect then
        throw ⟨body.span, "effect-bound", s!"inferred effect exceeds annotation: {effect.name}"⟩
  let bound := declared.getD packed.expr.effects
  let some proof := certifySubset packed.expr.effects bound
    | throw ⟨body.span, "effect-bound", "could not certify the effect upper bound"⟩
  pure ⟨caps, policy, packed.type, packed.expr, declared, bound, proof.included, nodes⟩

end Esh
