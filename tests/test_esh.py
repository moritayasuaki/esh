"""Boundary and end-to-end tests; all filesystem changes use temporary roots."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import esh


def script(body, declarations=""):
    return f"(script {declarations} {body})"


class FrontendTests(unittest.TestCase):
    def check(self, source):
        return esh.analyze(source)

    def diagnostic(self, source, code):
        report = self.check(source)
        self.assertFalse(report["ok"], report)
        self.assertEqual(report["diagnostic"]["code"], code, report)
        return report

    def test_pure_inference(self):
        report = self.check(script('(text.concat "a" "b")'))
        self.assertTrue(report["ok"])
        self.assertEqual(report["type"], "Text")
        self.assertEqual(report["effects"], [])

    def test_let_is_lexically_scoped(self):
        report = self.check(script('(let x "a" (let x true x))'))
        self.assertEqual(report["type"], "Bool")
        self.diagnostic(script('(let x "a" missing)'), "unknown-variable")

    def test_wrong_argument_type(self):
        self.diagnostic(script('(text.concat true "b")'), "type-mismatch")

    def test_branch_types_must_agree(self):
        self.diagnostic(script('(if true "a" false)'), "type-mismatch")

    def test_result_binding_requires_result_body(self):
        self.diagnostic(script('(let-ok x (ok "a") x)'), "type-mismatch")

    def test_return_annotation_checked(self):
        self.diagnostic(script('"a"', '(returns Bool)'), "return-type")

    def test_both_branch_effects(self):
        source = script('(if false (fs.write-text out (path "x") "data") (ok unit))', '(cap out (write))')
        report = self.check(source)
        self.assertEqual(report["effects"], [{"region": "out", "operation": "fs.write"}])

    def test_effect_annotation_is_upper_bound(self):
        self.diagnostic(script('(fs.write-text out (path "x") "data")', '(cap out (write)) (effects)'), "effect-bound")
        self.assertTrue(self.check(script('(ok unit)', '(cap out (write)) (effects (fs.write out))'))["ok"])

    def test_unregistered_capability_is_not_inferred(self):
        self.diagnostic(script('(fs.read-text _ (path "x"))', '(cap input (read))'), "capability-missing")

    def test_inference_and_narrowing_are_reported(self):
        report = self.check(script('(fs.read-text _ (path "x"))', '(cap input (read write) infer)'))
        node = next(n for n in report["nodes"] if n["kind"] == "fs.read-text")
        self.assertTrue(node["details"]["inferred"])
        self.assertTrue(node["details"]["narrowed"])
        self.assertEqual(node["details"]["required"], ["read"])

    def test_ambiguity_has_no_least_permission_preference(self):
        source = script('(fs.read-text _ (path "x"))', '(cap a (read) infer) (cap b (read write) infer)')
        self.diagnostic(source, "capability-ambiguous")

    def test_permission_cannot_be_amplified(self):
        self.diagnostic(script('(fs.write-text a (path "x") "data")', '(cap a (read))'), "permission-denied")

    def test_paths_are_distinct_from_text(self):
        self.diagnostic(script('(fs.read-text a "x")', '(cap a (read))'), "type-mismatch")

    def test_invalid_paths(self):
        for path in ["", "/tmp/x", "../x", "a/../x", "a//b", "a/./b", "a\\b", "a\x00b"]:
            with self.subTest(path=repr(path)):
                self.diagnostic(script('(path ' + json.dumps(path) + ')'), "path")

    def test_string_escapes_and_unicode_spans(self):
        source = '; comment\n(script (text.concat "λ\\n" "\\u0041"))'
        report = self.check(source)
        self.assertTrue(report["ok"])
        self.assertTrue(all(n["span"]["line"] == 2 for n in report["nodes"]))

    def test_malformed_sources(self):
        for source in ['(script "bad\\q")', '(script "unfinished)', '(script unit', '(script unit) unit']:
            with self.subTest(source=source):
                self.diagnostic(source, "syntax")

    def test_control_character_in_identifier(self):
        self.diagnostic('(script dangerous\x1bname)', "syntax")

    def test_source_limits(self):
        self.assertFalse(self.check(' ' * (esh.MAX_SOURCE + 1))["ok"])
        self.diagnostic('(script ' + '(ok ' * 150 + 'unit' + ')' * 150 + ')', "syntax")

    def test_unsupported_features_are_rejected(self):
        for body in ['(loop unit)', '(lambda x x)', '(proc.run "echo")']:
            self.diagnostic(script(body), "unsupported-form")

    def test_duplicate_capabilities_rejected(self):
        self.diagnostic(script('unit', '(cap a (read)) (cap a (write))'), "capability")


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="esh-test-")
        self.root = Path(self.temp.name)
        self.input = self.root / "input"
        self.output = self.root / "output"
        self.input.mkdir()
        self.output.mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def run_source(self, source, grants=None, limit=8388608):
        file = self.root / "script.esh"
        file.write_text(source, encoding="utf-8")
        if grants is None:
            grants = [["input", "read,write,mkdir,remove", str(self.input)],
                      ["output", "read,write,mkdir,remove", str(self.output)]]
        return esh.execute(file, grants, limit)

    def test_copy_and_trace(self):
        (self.input / "source.txt").write_text("hello\n", encoding="utf-8")
        source = (esh.PROJECT / "examples" / "copy.esh").read_text()
        code, report = self.run_source(source)
        self.assertEqual(code, 0, report)
        self.assertEqual((self.output / "copy.txt").read_text(), "Copied by esh\nhello\n")
        self.assertEqual(report["traceLength"], 2)

    def test_error_short_circuits_write(self):
        source = (esh.PROJECT / "examples" / "copy.esh").read_text()
        code, report = self.run_source(source)
        self.assertEqual(code, 1)
        self.assertEqual(report["traceLength"], 1)
        self.assertFalse((self.output / "copy.txt").exists())

    def test_trace_keeps_repeated_requests(self):
        source = script('(let-ok first (fs.write-text output (path "x") "one") (fs.write-text output (path "x") "two"))', '(cap output (write))')
        code, report = self.run_source(source)
        self.assertEqual(code, 0)
        self.assertEqual(report["traceLength"], 2)
        self.assertEqual(len(report["trace"]), 2)
        self.assertEqual((self.output / "x").read_text(), "two")

    def test_skipped_branch_has_no_request(self):
        source = script('(if false (fs.write-text output (path "x") "changed") (ok unit))', '(cap output (write))')
        code, report = self.run_source(source)
        self.assertEqual(code, 0)
        self.assertEqual(report["traceLength"], 0)
        self.assertFalse((self.output / "x").exists())

    def test_no_grants_no_write(self):
        source = script('(fs.write-text output (path "x") "changed")', '(cap output (write))')
        with self.assertRaises(esh.OperationError):
            self.run_source(source, [])
        self.assertFalse((self.output / "x").exists())

    def test_utf8_failure_preserves_result_type(self):
        (self.input / "source.txt").write_bytes(b"\xff")
        code, report = self.run_source((esh.PROJECT / "examples" / "copy.esh").read_text())
        self.assertEqual(code, 1)
        self.assertEqual(report["value"]["error"]["code"], "encoding")
        self.assertFalse((self.output / "copy.txt").exists())

    def test_symlink_file_rejected(self):
        outside = self.root / "outside.txt"
        outside.write_text("private")
        (self.input / "link").symlink_to(outside)
        code, report = self.run_source(script('(fs.read-text input (path "link"))', '(cap input (read))'))
        self.assertEqual(code, 1)
        self.assertIn("error", report["value"])

    def test_symlink_directory_rejected(self):
        (self.input / "other").symlink_to(self.output, target_is_directory=True)
        source = script('(fs.write-text input (path "other/x") "changed")', '(cap input (write))')
        code, _ = self.run_source(source)
        self.assertEqual(code, 1)
        self.assertFalse((self.output / "x").exists())

    def test_hardlink_replaced_without_modifying_other_link(self):
        outside = self.root / "outside.txt"
        outside.write_text("original")
        os.link(outside, self.output / "target")
        code, report = self.run_source(script('(fs.write-text output (path "target") "new")', '(cap output (write))'))
        self.assertEqual(code, 0, report)
        self.assertEqual(outside.read_text(), "original")
        self.assertEqual((self.output / "target").read_text(), "new")
        self.assertFalse(list(self.output.glob(".esh-*")))

    def test_aliasing_regions_are_allowed(self):
        source = script('(let-ok ignored (fs.write-text a (path "x") "shared") (fs.read-text b (path "x")))',
                        '(cap a (write)) (cap b (read))')
        code, report = self.run_source(source, [["a", "write", str(self.output)], ["b", "read", str(self.output)]])
        self.assertEqual(code, 0)
        self.assertEqual(report["value"], {"ok": "shared"})

    def test_directory_operations(self):
        source = script('(let-ok created (fs.mkdir output (path "sub")) (fs.list output (path ".")))',
                        '(cap output (mkdir read))')
        code, report = self.run_source(source)
        self.assertEqual(code, 0)
        self.assertEqual(report["value"], {"ok": ["sub"]})

    def test_remove_requires_regular_file(self):
        (self.output / "x").write_text("remove me")
        code, _ = self.run_source(script('(fs.remove output (path "x"))', '(cap output (remove))'))
        self.assertEqual(code, 0)
        self.assertFalse((self.output / "x").exists())
        (self.output / "folder").mkdir()
        code, _ = self.run_source(script('(fs.remove output (path "folder"))', '(cap output (remove))'))
        self.assertEqual(code, 1)
        self.assertTrue((self.output / "folder").is_dir())

    def test_resource_limit_short_circuits(self):
        (self.input / "source.txt").write_text("too long")
        code, report = self.run_source((esh.PROJECT / "examples" / "copy.esh").read_text(), limit=3)
        self.assertEqual(code, 1)
        self.assertEqual(report["value"]["error"]["code"], "resource-limit")
        self.assertFalse((self.output / "copy.txt").exists())

    def test_host_rechecks_authority(self):
        host = esh.Host([["input", "read,write", str(self.input)]])
        try:
            host.authorize({"capabilities": [{"name": "input", "permissions": ["read"]}]})
            result = host.dispatch({"operation": "write", "region": "input", "path": "x", "contents": "bad"})
            self.assertEqual(result["error"]["code"], "permission")
            self.assertFalse((self.input / "x").exists())
        finally:
            host.close()

    def test_ill_typed_host_reply_is_rejected(self):
        file = self.root / "protocol.esh"
        file.write_text(script('(fs.read-text input (path "x"))', '(cap input (read))'))
        process = subprocess.run([str(esh.CORE), "run", str(file)], input='{"ok": 123}\n',
                                 text=True, capture_output=True, timeout=10)
        final = json.loads(process.stdout.splitlines()[-1])
        self.assertEqual(process.returncode, 1)
        self.assertEqual(final["value"]["error"]["code"], "protocol")


class EditorServerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.process = subprocess.Popen([sys_executable(), str(esh.PROJECT / "esh.py"), "serve", "--port", "0"],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        cls.base = cls.process.stdout.readline().strip().split(" ")[-1]
        if not cls.base.startswith("http://"):
            _, error = cls.process.communicate(timeout=5)
            raise RuntimeError("Editor server did not start: " + error)

    @classmethod
    def tearDownClass(cls):
        cls.process.terminate()
        cls.process.communicate(timeout=5)

    def test_editor_and_analysis(self):
        with urlopen(self.base, timeout=5) as response:
            self.assertIn(b"Static review", response.read())
        request = Request(self.base + "/analyze", data=b'(script (ok "hello"))',
                          headers={"Origin": self.base}, method="POST")
        with urlopen(request, timeout=5) as response:
            report = json.load(response)
            self.assertEqual(report["type"], "(Result Text FsError)")

    def test_foreign_origin_is_rejected(self):
        request = Request(self.base + "/analyze", data=b'(script unit)',
                          headers={"Origin": "https://example.invalid"}, method="POST")
        with self.assertRaises(HTTPError) as error:
            urlopen(request, timeout=5)
        self.assertEqual(error.exception.code, 403)

    def test_no_run_endpoint(self):
        request = Request(self.base + "/run", data=b'(script unit)',
                          headers={"Origin": self.base}, method="POST")
        with self.assertRaises(HTTPError) as error:
            urlopen(request, timeout=5)
        self.assertEqual(error.exception.code, 403)


def sys_executable():
    import sys
    return sys.executable


if __name__ == "__main__":
    unittest.main()
