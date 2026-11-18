#!/usr/bin/env python3
"""Local CLI and POSIX host adapter for the Lean esh kernel. No dependencies."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import errno
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import sys
from esh_language import Diagnostic, elaborate

REFERENCE = Path(__file__).resolve().parent
PROJECT = REFERENCE.parent.parent
CORE = PROJECT / ".lake" / "build" / "bin" / "esh-core"
MAX_SOURCE = 262144
PERMISSIONS = {"read", "write", "mkdir", "remove"}
OP_PERMISSION = {"read": "read", "list": "read", "write": "write", "mkdir": "mkdir", "remove": "remove"}


class OperationError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


def require_core() -> None:
    if not CORE.is_file():
        raise OperationError("build-required", "Run 'lake build' first.")


def analyze(source: str, file: Path | None = None, project_root: Path | None = None) -> dict:
    require_core()
    if len(source.encode("utf-8")) > MAX_SOURCE:
        return {"ok": False, "error": "Source exceeds 256 KiB."}
    try:
        bundle = elaborate(source, file, project_root)
        result = subprocess.run([str(CORE), "check-ir"], input=json.dumps(bundle, ensure_ascii=False),
                                capture_output=True, text=True, encoding="utf-8", timeout=20)
        return json.loads(result.stdout)
    except Diagnostic as error:
        return error.report()
    except RecursionError:
        return {"ok": False, "error": "Static analysis exceeded its structural depth limit."}
    except subprocess.TimeoutExpired:
        return {"ok": False, "error": "Static analysis exceeded its time limit."}
    except (ValueError, UnicodeError):
        return {"ok": False, "error": "The checker did not return a valid report."}


def path_parts(value: str) -> list[str]:
    if not isinstance(value, str):
        raise OperationError("path", "Expected a relative path.")
    if value == ".":
        return []
    parts = value.split("/")
    if any(not p or p in (".", "..") or "\\" in p or
           any(ord(c) < 32 or ord(c) == 127 for c in p) for p in parts):
        raise OperationError("path", "Invalid relative path.")
    value.encode("utf-8", errors="strict")
    return parts


class Host:
    """Trusted syscall bridge; roots are opened once and traversed by descriptors.

    This is not an OS sandbox or a verified filesystem. See docs/assurance.md.
    """
    def __init__(self, grants: list[list[str]], max_bytes: int = 8 * 1024 * 1024):
        self.roots: dict[str, tuple[int, set[str]]] = {}
        self.active: dict[str, set[str]] = {}
        self.max_bytes = max_bytes
        if os.name != "posix" or not hasattr(os, "O_NOFOLLOW"):
            raise OperationError("platform", "The host requires POSIX descriptor-relative operations.")
        if max_bytes <= 0:
            raise OperationError("limit", "max-bytes must be positive.")
        try:
            for name, permission_text, directory in grants:
                rights = set(permission_text.split(","))
                if not name or name == "_" or name in self.roots:
                    raise OperationError("grant", "Capability names must be nonempty and unique.")
                if not rights <= PERMISSIONS:
                    raise OperationError("grant", "Unknown permission in grant.")
                # The selected root is trusted configuration, not a script path.
                fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
                self.roots[name] = (fd, rights)
        except BaseException:
            self.close()
            raise

    def close(self) -> None:
        for fd, _ in self.roots.values():
            os.close(fd)
        self.roots.clear()

    def authorize(self, report: dict) -> None:
        active = {}
        for cap in report["capabilities"]:
            name, rights = cap["name"], set(cap["permissions"])
            if name not in self.roots or not rights <= self.roots[name][1]:
                raise OperationError("grant", f"Missing grant for {name}: {', '.join(sorted(rights))}")
            active[name] = rights
        self.active = active

    @contextmanager
    def directory(self, root: int, parts: list[str]):
        fd = os.dup(root)
        try:
            for part in parts:
                child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                dir_fd=fd)
                os.close(fd)
                fd = child
            yield fd
        finally:
            os.close(fd)

    def dispatch(self, request: dict) -> dict:
        try:
            operation, region = request["operation"], request["region"]
            needed = OP_PERMISSION.get(operation)
            if needed is None or needed not in self.active.get(region, set()):
                raise OperationError("permission", "Request is outside the active grants.")
            parts = path_parts(request["path"])
            root = self.roots[region][0]
            if operation == "list":
                with self.directory(root, parts) as fd:
                    names, size = [], 0
                    with os.scandir(fd) as entries:
                        for entry in entries:
                            size += len(entry.name.encode("utf-8"))
                            if size > self.max_bytes:
                                raise OperationError("resource-limit", "Directory listing exceeds max-bytes.")
                            names.append(entry.name)
                    return {"ok": sorted(names)}
            if not parts:
                raise OperationError("path", "This operation requires a non-root path.")
            with self.directory(root, parts[:-1]) as parent:
                name = parts[-1]
                if operation == "read":
                    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK,
                                 dir_fd=parent)
                    with os.fdopen(fd, "rb") as stream:
                        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                            raise OperationError("file-kind", "Expected a regular file.")
                        data = stream.read(self.max_bytes + 1)
                    if len(data) > self.max_bytes:
                        raise OperationError("resource-limit", "File exceeds max-bytes.")
                    return {"ok": data.decode("utf-8", errors="strict")}
                if operation == "write":
                    data = request["contents"].encode("utf-8", errors="strict")
                    if len(data) > self.max_bytes:
                        raise OperationError("resource-limit", "Write exceeds max-bytes.")
                    mode = 0o600
                    try:
                        target = os.stat(name, dir_fd=parent, follow_symlinks=False)
                    except FileNotFoundError:
                        pass
                    else:
                        if not stat.S_ISREG(target.st_mode):
                            raise OperationError("file-kind", "Expected a regular file or a new path.")
                        mode = stat.S_IMODE(target.st_mode) & 0o777
                    temporary = ".esh-" + secrets.token_hex(16)
                    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 0o600, dir_fd=parent)
                    try:
                        with os.fdopen(fd, "wb") as stream:
                            stream.write(data)
                            stream.flush()
                            os.fchmod(stream.fileno(), mode)
                            os.fsync(stream.fileno())
                        os.replace(temporary, name, src_dir_fd=parent, dst_dir_fd=parent)
                    finally:
                        try:
                            os.unlink(temporary, dir_fd=parent)
                        except FileNotFoundError:
                            pass
                    return {"ok": None}
                if operation == "mkdir":
                    os.mkdir(name, 0o700, dir_fd=parent)
                    return {"ok": None}
                if operation == "remove":
                    info = os.stat(name, dir_fd=parent, follow_symlinks=False)
                    if not stat.S_ISREG(info.st_mode):
                        raise OperationError("file-kind", "remove accepts regular files only.")
                    os.unlink(name, dir_fd=parent)
                    return {"ok": None}
            raise OperationError("protocol", "Unsupported request.")
        except OperationError as e:
            return {"error": {"code": e.code, "message": str(e)}}
        except UnicodeError:
            return {"error": {"code": "encoding", "message": "Expected valid UTF-8."}}
        except OSError as e:
            # Return the portable reason; avoid leaking configured absolute roots.
            return {"error": {"code": errno.errorcode.get(e.errno, "io"),
                              "message": e.strerror or "Filesystem operation failed."}}
        except (KeyError, TypeError, AttributeError):
            return {"error": {"code": "protocol", "message": "Malformed host request."}}


def execute(file: Path, grants: list[list[str]], max_bytes: int,
            project_root: Path | None = None) -> tuple[int, dict]:
    require_core()
    try:
        bundle = elaborate(file.read_text(encoding="utf-8"), file, project_root)
    except Diagnostic as error:
        return 2, error.report()
    except RecursionError:
        return 2, {"ok": False, "error": "Static analysis exceeded its structural depth limit."}
    host = Host(grants, max_bytes)
    process = None
    final = None
    ready = False
    try:
        process = subprocess.Popen([str(CORE), "run-ir"], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, text=True, encoding="utf-8")
        process.stdin.write(json.dumps(bundle, ensure_ascii=False) + "\n")
        process.stdin.flush()
        for line in process.stdout:
            message = json.loads(line)
            if message.get("kind") == "ready" and not ready:
                host.authorize(message["analysis"])
                ready = True
            elif message.get("kind") == "request" and ready:
                process.stdin.write(json.dumps(host.dispatch(message), ensure_ascii=False) + "\n")
                process.stdin.flush()
            elif message.get("kind") == "result" and ready:
                final = message
            elif message.get("ok") is False:
                final = message
            else:
                raise OperationError("protocol", "Unexpected kernel message.")
        code = process.wait()
        if final is None:
            raise OperationError("protocol", "Kernel exited without a result.")
        return code, final
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait()
            process.stdin.close()
            process.stdout.close()
        host.close()


def print_analysis(report: dict) -> int:
    if not report.get("ok"):
        diagnostic = report.get("diagnostic", {})
        where = diagnostic.get("span", {})
        print(f"{diagnostic.get('source', report.get('entry', '<buffer>'))}:"
              f"{where.get('line', '?')}:{where.get('column', '?')}: "
              f"{diagnostic.get('code', 'error')}: {diagnostic.get('message', report.get('error'))}")
        return 2
    print("Type: " + report["type"])
    effects = report["effects"]
    print("Effects (upper bound): " + (", ".join(f"{e['operation']} {e['region']}" for e in effects) or "pure"))
    for node in report["nodes"]:
        if not node["kind"].startswith("fs."):
            continue
        d = node["details"]
        notes = []
        if d["inferred"]:
            notes.append("inferred _ -> " + d["region"])
        if d["narrowed"]:
            notes.append("rights {" + ", ".join(d["granted"]) + "} -> {" + ", ".join(d["required"]) + "}")
        print(f"  {node.get('source', report.get('entry', '<buffer>'))}:"
              f"{node['span']['line']}:{node['span']['column']}: {node['kind']} [{d['region']}]"
              + ("; " + "; ".join(notes) if notes else ""))
    return 0


def serve(port: int, file: Path | None = None, project_root: Path | None = None) -> None:
    require_core()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def send(self, code, body, content_type):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'")
            self.end_headers()
            self.wfile.write(body)

        def valid_host(self):
            return self.headers.get("Host") in {f"127.0.0.1:{self.server.server_port}", f"localhost:{self.server.server_port}"}

        def do_GET(self):
            if not self.valid_host():
                return self.send(403, b"Forbidden", "text/plain")
            if self.path == "/":
                return self.send(200, (REFERENCE / "ui" / "index.html").read_bytes(), "text/html; charset=utf-8")
            if self.path == "/example":
                return self.send(200, (file or PROJECT / "examples" / "copy.esh").read_bytes(), "text/plain; charset=utf-8")
            self.send(404, b"Not found", "text/plain")

        def do_POST(self):
            origin = self.headers.get("Origin")
            expected = {f"http://127.0.0.1:{self.server.server_port}", f"http://localhost:{self.server.server_port}"}
            if not self.valid_host() or origin not in expected or self.path != "/analyze":
                return self.send(403, b"Forbidden", "text/plain")
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if length < 0 or length > MAX_SOURCE:
                    return self.send(413, b"Source too large", "text/plain")
                source = self.rfile.read(length).decode("utf-8")
                payload = json.dumps(analyze(source, file, project_root), ensure_ascii=False).encode("utf-8")
                self.send(200, payload, "application/json; charset=utf-8")
            except (ValueError, UnicodeError):
                self.send(400, b"Invalid input", "text/plain")

    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"Review editor: http://127.0.0.1:{server.server_port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


def main() -> int:
    parser = argparse.ArgumentParser(description="esh: typed filesystem scripts with explicit effects")
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check", help="Analyze a script without executing it")
    check.add_argument("file", type=Path)
    check.add_argument("--json", action="store_true")
    check.add_argument("--project-root", type=Path, help="Import boundary; defaults to the entry file's directory")
    run = commands.add_parser("run", help="Run using explicitly granted filesystem roots")
    run.add_argument("file", type=Path)
    run.add_argument("--grant", nargs=3, action="append", default=[], metavar=("NAME", "PERMISSIONS", "DIRECTORY"))
    run.add_argument("--max-bytes", type=int, default=8 * 1024 * 1024)
    run.add_argument("--project-root", type=Path, help="Import boundary; defaults to the entry file's directory")
    review = commands.add_parser("serve", help="Open a local analysis-only editor service")
    review.add_argument("--port", type=int, default=8765)
    review.add_argument("--file", type=Path, help="Entry-file context for reviewing local imports")
    review.add_argument("--project-root", type=Path, help="Import boundary; defaults to the entry file's directory")
    args = parser.parse_args()
    try:
        if args.command == "check":
            report = analyze(args.file.read_text(encoding="utf-8"), args.file, args.project_root)
            if args.json:
                print(json.dumps(report, ensure_ascii=False, indent=2))
                return 0 if report.get("ok") else 2
            return print_analysis(report)
        if args.command == "run":
            code, result = execute(args.file, args.grant, args.max_bytes, args.project_root)
            print(json.dumps(result, ensure_ascii=False, indent=2))
            return code
        serve(args.port, args.file, args.project_root)
        return 0
    except (OperationError, OSError, ValueError) as e:
        print(f"esh: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
