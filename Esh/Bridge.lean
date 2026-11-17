import Esh.Frontend

namespace Esh
open Lean

def errorJson (e : FsError) : Json := Json.mkObj [
  ("code", toJson e.code), ("message", toJson e.message)]

def valueJson : (t : Ty) → t.Value → Json
  | .unit, _ => Json.null
  | .bool, b => toJson b
  | .text, s => toJson s
  | .path, p => toJson p.text
  | .list t, xs => toJson (xs.map (valueJson t))
  | .result t, .ok v => Json.mkObj [("ok", valueJson t v)]
  | .result _, .error e => Json.mkObj [("error", errorJson e)]

def resultFailed : (t : Ty) → t.Value → Bool
  | .result _, .error _ => true
  | _, _ => false

def Request.json : Request α → Json
  | .read r p => request "read" r p []
  | .write r p data => request "write" r p [("contents", toJson data)]
  | .list r p => request "list" r p []
  | .mkdir r p => request "mkdir" r p []
  | .remove r p => request "remove" r p []
where
  request (op region : String) (path : RelPath) (extra : List (String × Json)) :=
    Json.mkObj ([("kind", toJson "request"), ("operation", toJson op),
      ("region", toJson region), ("path", toJson path.text)] ++ extra)

def decodeValue : Request α → Json → Except FsError α
  | .read _ _, value => value.getStr?.mapError (fun msg => FsError.mk "protocol" msg)
  | .list _ _, value => (fromJson? value).mapError (fun msg => FsError.mk "protocol" msg)
  | .write _ _ _, value | .mkdir _ _, value | .remove _ _, value =>
      if value == Json.null then .ok () else .error ⟨"protocol", "expected null for Unit"⟩

def decodeReply (request : Request α) (j : Json) : Except FsError α := do
  match j.getObjVal? "error" with
  | .ok error =>
      let code := (error.getObjValAs? String "code").toOption.getD "host-error"
      let message := (error.getObjValAs? String "message").toOption.getD "host operation failed"
      throw ⟨code, message⟩
  | .error _ =>
      let value ← (j.getObjVal? "ok").mapError (fun msg => FsError.mk "protocol" msg)
      decodeValue request value

def bridgeHandler (request : Request α) : IO (Except FsError α) := do
  let stdout ← IO.getStdout
  stdout.putStrLn request.json.compress
  stdout.flush
  let line ← (← IO.getStdin).getLine
  if line.isEmpty then return .error ⟨"protocol", "host closed the request channel"⟩
  match Json.parse line with
  | .error msg => return .error ⟨"protocol", msg⟩
  | .ok reply => return decodeReply request reply

end Esh
