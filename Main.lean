import Esh.Bridge
import Esh.FunctionBridge

open Lean Esh

def emit (j : Json) : IO Unit := do
  let out ← IO.getStdout
  out.putStrLn j.compress
  out.flush

def main (args : List String) : IO UInt32 := do
  try
    match args with
    | ["check-ir"] =>
        let source ← (← IO.getStdin).readToEnd
        match Json.parse source with
        | .ok input => Esh.Functions.runBundle input false
        | .error message => throw (IO.userError message)
    | ["run-ir"] =>
        let source ← (← IO.getStdin).getLine
        match Json.parse source with
        | .ok input => Esh.Functions.runBundle input true
        | .error message => throw (IO.userError message)
    | _ =>
        (← IO.getStderr).putStrLn "usage: esh-core check-ir | run-ir (JSON host protocol)"
        return 2
  catch e =>
    emit (Json.mkObj [("ok", toJson false), ("error", toJson e.toString)])
    return 2
