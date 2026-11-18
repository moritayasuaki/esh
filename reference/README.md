# Reference implementation

The Python host/frontend and handwritten browser UI in `python/` are retained
only as a regression oracle for the original Lean kernel. They are not used by
the Rust CLI, runtime, or Rust/Wasm editor and do not implement update contracts.

The original Lean definitions remain under `Esh/` at the repository root. They
are a separate formal model; their theorems do not automatically apply to the
Rust interpreter. Current Rust proof coverage is documented in
[assurance](../docs/assurance.md).

To check the reference implementation from the repository root:

```sh
lake build
(cd reference/python && python3 -B -m unittest discover -s tests -v)
```
