"""Loopback-only local UI and JSON API. No credentials are served to the browser."""
import json
import mimetypes
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from .engine import DEFAULT_LIMITS
from .tasks import TASKS
from .tools import TOOL_SCHEMAS
from .tui_backend import public_config

STATIC = Path(__file__).parent / "static"


def make_server(engine, port=8787):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def respond(self, status, data, content_type="application/json; charset=utf-8"):
            body = json.dumps(data, ensure_ascii=False).encode() if isinstance(data, (dict, list)) else data
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Security-Policy", "default-src 'self'; style-src 'self'; script-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def valid_host(self):
            expected = {f"localhost:{self.server.server_port}", f"127.0.0.1:{self.server.server_port}"}
            return self.headers.get("Host") in expected

        def do_GET(self):
            if not self.valid_host():
                return self.respond(403, {"error": "Only local requests are allowed."})
            path = urlsplit(self.path).path
            try:
                if path == "/api/config":
                    return self.respond(200, public_config(engine))
                if path == "/api/tasks":
                    return self.respond(200, list(TASKS.values()))
                if path == "/api/runs":
                    return self.respond(200, engine.list_runs())
                parts = path.strip("/").split("/")
                if len(parts) in (3, 4) and parts[:2] == ["api", "runs"]:
                    if len(parts) == 4 and parts[3] != "export":
                        raise FileNotFoundError()
                    return self.respond(200, engine.get(parts[2]))
                files = {"/": "index.html", "/app.js": "app.js", "/style.css": "style.css", "/favicon.svg": "favicon.svg"}
                if path in files:
                    file = STATIC / files[path]
                    return self.respond(200, file.read_bytes(), mimetypes.guess_type(file)[0] or "text/plain")
                raise FileNotFoundError()
            except FileNotFoundError:
                self.respond(404, {"error": "Not found."})
            except ValueError as exc:
                self.respond(400, {"error": str(exc)})

        def do_POST(self):
            if not self.valid_host():
                return self.respond(403, {"error": "Only local requests are allowed."})
            origin = self.headers.get("Origin")
            if origin and origin not in {f"http://localhost:{self.server.server_port}", f"http://127.0.0.1:{self.server.server_port}"}:
                return self.respond(403, {"error": "Cross-origin requests are not allowed."})
            if self.headers.get("Sec-Fetch-Site") == "cross-site":
                return self.respond(403, {"error": "Cross-site requests are not allowed."})
            if self.headers.get_content_type() != "application/json":
                return self.respond(415, {"error": "Use application/json."})
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if length <= 0 or length > 1_200_000:
                    raise ValueError("Invalid request size.")
                body = json.loads(self.rfile.read(length), parse_constant=lambda x: (_ for _ in ()).throw(ValueError("Invalid JSON number.")))
                path = urlsplit(self.path).path
                if path == "/api/runs":
                    run_id = engine.start(body)
                    return self.respond(201, {"id": run_id})
                parts = path.strip("/").split("/")
                if len(parts) == 4 and parts[:2] == ["api", "runs"] and parts[3] == "cancel":
                    engine.cancel(parts[2])
                    return self.respond(200, {"ok": True})
                raise FileNotFoundError()
            except FileNotFoundError:
                self.respond(404, {"error": "Not found."})
            except (ValueError, TypeError) as exc:
                self.respond(400, {"error": str(exc)})

    return ThreadingHTTPServer(("127.0.0.1", port), Handler)
