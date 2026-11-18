"""Function, module, and polymorphism contracts through the executable kernel."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import esh


class FunctionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="esh-functions-")
        self.root = Path(self.temp.name)
        self.entry = self.root / "main.esh"
        (self.root / "data").mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def check(self, source):
        return esh.analyze(source, self.entry)

    def reject(self, source, code):
        result = self.check(source)
        self.assertFalse(result["ok"], result)
        self.assertEqual(result.get("diagnostic", {}).get("code"), code, result)

    def run_source(self, source, grants=None):
        self.entry.write_text(source)
        return esh.execute(self.entry, grants or [], 4096)

    def test_rank_one_identity_at_two_types(self):
        source = '''(script
          (def identity (export) (forall (A Type)) (params (x A))
            (returns A) (effects) (body x))
          (do (let text (identity "first")) (identity true)))'''
        code, result = self.run_source(source)
        self.assertEqual((code, result["value"]), (0, True))

    def test_unused_export_is_checked_universally(self):
        self.reject('''(script (def wrong (export) (forall (A Type))
          (params (x A)) (returns A) (effects) (body "only text")) unit)''', "type-mismatch")

    def test_exported_signature_is_required(self):
        self.reject('(script (def id (export) (params x) (body x)) unit)', "public-signature")
        self.reject('(script (def id (export) (params (x Text)) (returns Text) (body x)) unit)', "public-signature")

    def test_full_arity_and_explicit_partial_application(self):
        self.reject('(script (text.concat "one"))', "arity")
        code, result = self.run_source('''(script (do
          (let prefix (fn (params x) (body (text.concat "prefix:" x))))
          (prefix "value")))''')
        self.assertEqual((code, result["value"]), (0, "prefix:value"))

    def test_pure_calls_and_callees_must_be_sequenced(self):
        self.reject('(script (text.concat (text.concat "a" "b") "c"))', "evaluation-order")
        self.reject('(script ((text.concat "a" "b") "c"))', "evaluation-order")
        self.reject('(script (if (text.eq "a" "b") unit unit))', "evaluation-order")
        self.reject('(script (if true (text.concat "a" "b") "c"))', "evaluation-order")

    def test_computed_pure_closure_can_be_generalized(self):
        code, result = self.run_source('''(script
          (def keep (params x)
            (body (fn (params ignored) (captures x) (body x))))
          (do (let f (keep "retained"))
              (let first (f true))
              (f "different argument type")))''')
        self.assertEqual((code, result["value"], result["traceLength"]), (0, "retained", 0))

    def test_ordinary_capture_is_inferred(self):
        code, result = self.run_source('''(script
          (def prefix (params (text Text))
            (body (fn (params suffix) (body (text.concat text suffix)))))
          (do (let f (prefix "hello ")) (f "world")))''')
        self.assertEqual((code, result["value"]), (0, "hello world"))

    def test_unknown_type_and_pure_callback_captures_require_declaration(self):
        self.reject('''(script (def keep (params x)
          (body (fn (params ignored) (body x)))) unit)''', "capture-required")
        self.reject('''(script (def wrap (params (f (Fn () Unit (effects))))
          (body (fn (params) (body (f))))) unit)''', "capture-required")

    def test_authority_free_named_function_needs_no_capture_declaration(self):
        code, result = self.run_source('''(script
          (def id (params x) (body x))
          (do (let f (fn (params x) (body (id x)))) (f "ok")))''')
        self.assertEqual((code, result["value"]), (0, "ok"))

    def test_closure_creation_is_pure_and_capture_is_narrowed(self):
        source = '''(script (cap workspace (read write) infer)
          (fn (params p) (captures _) (body (fs.read-text _ p))))'''
        report = self.check(source)
        self.assertTrue(report["ok"], report)
        self.assertEqual(report["effects"], [])
        capture = next(n for n in report["nodes"] if n["kind"] == "fn")["details"]["captures"][0]
        self.assertEqual(capture["required"], ["read"])
        self.assertTrue(capture["narrowed"])
        self.assertTrue(capture["inferred"])

    def test_capability_parameter_opt_in(self):
        source = '''(script (cap input (read))
          (def read (export) (forall (R Region))
            (params (root (Cap R (read)) infer) (path RelPath))
            (returns (Result Text FsError)) (effects (fs.read R))
            (body (fs.read-text _ path)))
          (do (let path (path "file")) (read input path)))'''
        self.assertTrue(self.check(source)["ok"])
        self.reject(source.replace('(read)) infer)', '(read)))'), "capability-missing")

    def test_private_capability_permissions_combine_all_uses(self):
        (self.root / "data" / "file").write_text("old")
        code, result = self.run_source('''(script (cap root (read write))
          (def replace (params root path)
            (body (do (let-ok old (fs.read-text root path))
                      (fs.write-text root path "new"))))
          (do (let p (path "file")) (replace root p)))''',
          [["root", "read,write", str(self.root / "data")]])
        self.assertEqual(code, 0, result)
        self.assertEqual([e["operation"] for e in result["trace"]], ["fs.read", "fs.write"])

    def test_independent_callback_effects_and_order(self):
        (self.root / "data" / "file").write_text("before")
        source = '''(script (cap root (read write))
          (def both (export)
            (forall (A Type) (B Type) (E1 Effect) (E2 Effect))
            (params (first (Fn () A (effects E1))) (second (Fn () B (effects E2))))
            (returns B) (effects E1 E2)
            (body (do (let ignored (first)) (second))))
          (do (let p (path "file"))
              (let read (fn (params) (captures root) (body (fs.read-text root p))))
              (let write (fn (params) (captures root) (body (fs.write-text root p "after"))))
              (both read write)))'''
        code, result = self.run_source(source, [["root", "read,write", str(self.root / "data")]])
        self.assertEqual(code, 0, result)
        self.assertEqual([e["operation"] for e in result["trace"]], ["fs.read", "fs.write"])
        report = self.check(source)
        both = next(n for n in report["nodes"] if n["kind"] == "definition" and n["details"]["name"] == "both")
        self.assertIn("E1", both["type"])
        self.assertIn("E2", both["type"])

    def test_inferred_callback_result_and_repeated_effects(self):
        (self.root / "data" / "file").write_text("hello")
        code, result = self.run_source('''(script (cap root (read))
          (def twice (params f x) (body (do (let-ok first (f x)) (f x))))
          (do (let read (fn (params p) (captures root) (body (fs.read-text root p))))
              (let p (path "file")) (twice read p)))''',
          [["root", "read", str(self.root / "data")]])
        self.assertEqual(code, 0, result)
        self.assertEqual(result["traceLength"], 2)

    def test_effectful_binding_is_not_generalized(self):
        self.reject('''(script (cap root (read))
          (def load (params root path)
            (body (do (let-ok text (fs.read-text root path))
                      (let id (fn (params x) (body x))) (ok id))))
          (do (let p (path "file")) (let-ok id (load root p))
              (let bool (id true)) (let text (id "text")) (ok text)))''', "type-mismatch")

    def test_callback_parameter_is_monomorphic_within_one_call(self):
        self.reject('''(script (def twice (params callback)
          (body (do (let a (callback "text")) (callback true)))) unit)''', "type-mismatch")

    def test_first_class_builtin(self):
        code, result = self.run_source('''(script
          (def apply-two (params f a b) (body (f a b)))
          (apply-two text.concat "left" "right"))''')
        self.assertEqual((code, result["value"]), (0, "leftright"))

    def test_repeated_effect_instantiation_preserves_function_type(self):
        prefix = '''(script (cap root (read))
          (def combine (params f g)
            (body (fn (params) (captures f g)
              (body (do (let first (f)) (g))))))
          (do (let p (path "file"))
              (let read (fn (params) (captures root) (body (fs.read-text root p))))'''
        report = self.check(prefix + '(combine read read)))')
        self.assertTrue(report["ok"], report)
        (self.root / "data" / "file").write_text("twice")
        code, result = self.run_source(prefix + '(let combined (combine read read)) (combined)))',
                                      [["root", "read", str(self.root / "data")]])
        self.assertEqual(code, 0, result)
        self.assertEqual(result["traceLength"], 2)

    def test_effect_union_order_does_not_change_function_type(self):
        source = '''(script (cap root (read write))
          (def use (params (callback (Fn () Unit (effects (fs.read root) (fs.write root)))))
            (body (callback)))
          (do (let callback (fn (params) (effects (fs.write root) (fs.read root)) (body unit)))
              (use callback)))'''
        code, result = self.run_source(source, [["root", "read,write", str(self.root / "data")]])
        self.assertEqual((code, result["value"], result["traceLength"]), (0, None, 0))

    def test_retained_region_remains_fixed(self):
        source = '''(script (cap input (read)) (cap output (read))
          (def reader (export) (forall (R Region))
            (params (root (Cap R (read))))
            (returns (Fn (RelPath) (Result Text FsError) (effects (fs.read R))))
            (effects)
            (body (fn (params path) (captures root) (body (fs.read-text root path)))))
          (do (let read (reader input)) (let p (path "file")) (read p)))'''
        report = self.check(source)
        self.assertTrue(report["ok"], report)
        self.assertEqual(report["effects"], [{"operation": "fs.read", "region": "input"}])

    def test_forward_references_and_recursive_cycles(self):
        code, result = self.run_source('''(script (def first (params x) (body (second x)))
          (def second (params x) (body x)) (first "forward"))''')
        self.assertEqual((code, result["value"]), (0, "forward"))
        self.reject('''(script (def first (params x) (body (second x)))
          (def second (params x) (body (first x))) unit)''', "definition-cycle")

    def test_transparent_parameterized_alias(self):
        code, result = self.run_source('''(script
          (type Outcome (forall (A Type)) (alias (Result A FsError)))
          (def wrap (export) (forall (A Type)) (params (x A))
            (returns (Outcome A)) (effects) (body (ok x)))
          (wrap "value"))''')
        self.assertEqual((code, result["value"]), (0, {"ok": "value"}))
        self.reject('(script (type Cycle (alias Cycle)) unit)', "type-cycle")

    def test_static_imports_are_definition_only(self):
        (self.root / "library.esh").write_text('''(module library
          (def write (export) (forall (R Region))
            (params (root (Cap R (write))) (p RelPath))
            (returns (Result Unit FsError)) (effects (fs.write R))
            (body (fs.write-text root p "not initialized")))
          (def id (export) (forall (A Type)) (params (x A))
            (returns A) (effects) (body x)))''')
        code, result = self.run_source('(script (import lib "./library.esh") (lib.id "loaded"))')
        self.assertEqual((code, result["value"], result["traceLength"]), (0, "loaded", 0))

    def test_import_visibility_and_unused_public_contract(self):
        lib = self.root / "library.esh"
        lib.write_text('(module lib (def private (params x) (body x)))')
        self.reject('(script (import lib "./library.esh") (lib.private "x"))', "private-name")
        lib.write_text('(module lib (def bad (export) (params (x Bool)) (returns Text) (effects) (body x)))')
        self.reject('(script (import lib "./library.esh") unit)', "type-mismatch")

    def test_import_cycles_and_root_escape(self):
        (self.root / "a.esh").write_text('(module a (import b "./b.esh"))')
        (self.root / "b.esh").write_text('(module b (import a "./a.esh"))')
        self.reject('(script (import a "./a.esh") unit)', "import-cycle")
        self.reject('(script (import outside "../outside.esh") unit)', "project-root")
        with tempfile.TemporaryDirectory(prefix="esh-outside-") as other:
            outside = Path(other) / "lib.esh"
            outside.write_text('(module lib)')
            (self.root / "link.esh").symlink_to(outside)
            self.reject('(script (import outside "./link.esh") unit)', "project-root")

    def test_explicit_project_root_and_importer_relative_path(self):
        sub = self.root / "scripts"
        sub.mkdir()
        file = sub / "entry.esh"
        (self.root / "lib.esh").write_text('''(module lib (def value (export)
          (params) (returns Text) (effects) (body "root")))''')
        source = '(script (import lib "../lib.esh") (lib.value))'
        result = esh.analyze(source, file)
        self.assertFalse(result["ok"])
        result = esh.analyze(source, file, self.root)
        self.assertTrue(result["ok"], result)
        self.assertEqual({s["name"] for s in result["sources"]}, {"scripts/entry.esh", "lib.esh"})

    def test_import_requires_file_context(self):
        result = esh.analyze('(script (import lib "./lib.esh") unit)')
        self.assertEqual(result["diagnostic"]["code"], "import-context")

    def test_invalid_export_and_conflicting_import_name(self):
        self.reject('(script (type Name (export extra) (alias Text)) unit)', "export")
        (self.root / "lib.esh").write_text('(module lib)')
        self.reject('(script (cap lib (read)) (import lib "./lib.esh") unit)', "duplicate-name")

    def test_import_symlink_cycle_is_a_diagnostic(self):
        (self.root / "first.esh").symlink_to("second.esh")
        (self.root / "second.esh").symlink_to("first.esh")
        self.reject('(script (import cycle "./first.esh") unit)', "import-path")

    def test_entry_outside_explicit_root_has_a_diagnostic(self):
        result = esh.analyze('(script unit)', self.entry, self.root / "data")
        self.assertEqual(result["diagnostic"]["code"], "project-root")

    def test_repeated_imports_share_one_module(self):
        (self.root / "lib.esh").write_text('''(module lib (def id (export)
          (forall (A Type)) (params (x A)) (returns A) (effects) (body x)))''')
        source = '''(script (import first "./lib.esh") (import second "lib.esh")
          (do (let a (first.id "a")) (second.id true)))'''
        result = self.check(source)
        self.assertTrue(result["ok"], result)
        self.assertEqual(len(result["sources"]), 2)
        self.assertEqual(len([n for n in result["nodes"] if n["kind"] == "definition"]), 1)

    def test_shared_type_references_cannot_cycle(self):
        bundle = esh.elaborate('(script "value")')
        bundle["types"][0] = {"ref": 0}
        process = subprocess.run([str(esh.CORE), "check-ir"], input=json.dumps(bundle),
                                 text=True, capture_output=True, timeout=10)
        self.assertEqual(process.returncode, 2)
        self.assertIn("cyclic shared type reference", process.stdout)

    def test_imported_diagnostics_include_reviewable_source(self):
        (self.root / "bad.esh").write_text('(module bad (def broken (params) (body missing)))')
        result = self.check('(script (import bad "./bad.esh") unit)')
        self.assertFalse(result["ok"])
        self.assertEqual(result["diagnostic"]["source"], "bad.esh")
        self.assertEqual({x["name"] for x in result["sources"]}, {"main.esh", "bad.esh"})

    def test_execution_uses_checked_source_snapshot(self):
        library = self.root / "library.esh"
        library.write_text('''(module lib (def value (export)
          (params) (returns Text) (effects) (body "checked")))''')
        bundle = esh.elaborate('(script (import lib "./library.esh") (lib.value))', self.entry)
        library.write_text('(module lib (def invalid))')
        process = subprocess.run([str(esh.CORE), "run-ir"], input=json.dumps(bundle) + "\n", text=True, capture_output=True, timeout=10)
        result = json.loads(process.stdout.splitlines()[-1])
        self.assertEqual((process.returncode, result["value"]), (0, "checked"))

    def test_kernel_rejects_a_forged_function_bound(self):
        source = '''(script (cap root (write))
          (do (let p (path "file")) (fs.write-text root p "must not run")))'''
        bundle = esh.elaborate(source, self.entry)
        def forge(node):
            if isinstance(node, dict):
                if node.get("op") == "fn" and node.get("effects"):
                    node["effects"] = []
                    return True
                return any(forge(v) for v in node.values())
            if isinstance(node, list):
                return any(forge(v) for v in node)
            return False
        self.assertTrue(forge(bundle["program"]))
        process = subprocess.run([str(esh.CORE), "run-ir"], input=json.dumps(bundle) + "\n", text=True, capture_output=True, timeout=10)
        self.assertEqual(process.returncode, 2)
        self.assertNotIn('"kind":"ready"', process.stdout)
        self.assertIn("effect contract", process.stdout)


if __name__ == "__main__":
    unittest.main()
