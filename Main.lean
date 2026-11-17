import Esh.Bridge

open Lean Esh

def emit (j : Json) : IO Unit := do
  let out ← IO.getStdout
  out.putStrLn j.compress
  out.flush

def runSource (source : String) (execute : Bool) : IO UInt32 := do
  match check source with
  | .error diagnostic =>
      emit (Json.mkObj [("ok", toJson false), ("diagnostic", diagnostic.json)])
      return 2
  | .ok script =>
      if !execute then
        emit (script.json source)
        return 0
      emit (Json.mkObj [("kind", toJson "ready"), ("analysis", script.json source)])
      let result ← script.program.run bridgeHandler
      emit (Json.mkObj [("kind", toJson "result"), ("type", toJson script.type.name),
        ("value", valueJson script.type result.value), ("trace", toJson (result.trace.map effectJson)),
        ("traceLength", toJson result.trace.length)])
      return if resultFailed script.type result.value then 1 else 0

def main (args : List String) : IO UInt32 := do
  try
    match args with
    | ["check-stdin"] => runSource (← (← IO.getStdin).readToEnd) false
    | ["check", file] => runSource (← IO.FS.readFile file) false
    | ["run", file] => runSource (← IO.FS.readFile file) true
    | _ =>
        (← IO.getStderr).putStrLn "usage: esh-core check FILE | check-stdin | run FILE (host protocol)"
        return 2
  catch e =>
    emit (Json.mkObj [("ok", toJson false), ("error", toJson e.toString)])
    return 2
