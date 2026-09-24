"""Backends for the terminal UI. All HTTP connections stay on loopback."""
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from .engine import DEFAULT_LIMITS, Engine
from .tasks import TASKS
from .tools import TOOL_SCHEMAS


def public_config(engine):
    return {"app": "relay", "protocol": 1, "workspace": str(engine.root),
            "minimax_ready": bool(os.environ.get("ANTHROPIC_AUTH_TOKEN") or os.environ.get("ANTHROPIC_API_KEY")),
            "model": os.environ.get("MINIMAX_MODEL", "MiniMax-M2.7"),
            "limits": DEFAULT_LIMITS, "tools": TOOL_SCHEMAS}


class LocalBackend:
    mode = "local engine"
    owns_engine = True

    def __init__(self, engine):
        self.engine = engine

    def config(self): return public_config(self.engine)
    def tasks(self): return list(TASKS.values())
    def list_runs(self): return self.engine.list_runs()
    def get(self, run_id): return self.engine.get(run_id)
    def start(self, config): return self.engine.start(config)
    def cancel(self, run_id): return self.engine.cancel(run_id)


class RemoteBackend:
    mode = "dashboard connected"
    owns_engine = False

    def __init__(self, url, expected_workspace=None):
        parsed = urllib.parse.urlsplit(url)
        if (parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost"}
                or parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in {"", "/"}):
            raise ValueError("Connect to a local Aster address such as http://127.0.0.1:8787.")
        self.url = url.rstrip("/")
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect)
        config = self.config()
        if config.get("app") != "relay" or config.get("protocol") != 1:
            raise ValueError("This is not a compatible Aster server. Restart the dashboard with the updated code.")
        if expected_workspace and config.get("workspace") != str(Path(expected_workspace).resolve()):
            raise ValueError("The dashboard is using a different workspace; specify --connect explicitly to select it.")

    def request(self, path, body=None):
        data = None if body is None else json.dumps(body, allow_nan=False).encode()
        request = urllib.request.Request(self.url + path, data=data, headers={"Content-Type": "application/json"})
        try:
            with self.opener.open(request, timeout=4) as response:
                return json.load(response)
        except urllib.error.HTTPError as exc:
            try:
                message = json.loads(exc.read(4096)).get("error", f"Aster returned HTTP {exc.code}.")
            except (ValueError, AttributeError):
                message = f"Aster returned HTTP {exc.code}."
            finally:
                exc.close()
            raise ValueError(message) from None
        except (urllib.error.URLError, TimeoutError, OSError):
            raise ConnectionError("Cannot reach the Aster dashboard. Check that it is still running.") from None

    def config(self): return self.request("/api/config")
    def tasks(self): return self.request("/api/tasks")
    def list_runs(self): return self.request("/api/runs")
    def get(self, run_id): return self.request(f"/api/runs/{run_id}")
    def start(self, config): return self.request("/api/runs", config)["id"]
    def cancel(self, run_id): return self.request(f"/api/runs/{run_id}/cancel", {})


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise ValueError("Aster does not follow server redirects.")


def make_backend(data_dir, connect=None):
    if connect:
        return RemoteBackend(connect)
    try:
        return LocalBackend(Engine(data_dir))
    except ValueError as original:
        try:
            return RemoteBackend("http://127.0.0.1:8787", expected_workspace=data_dir)
        except (ValueError, ConnectionError):
            raise ValueError(str(original) + " Or connect explicitly with tui --connect http://127.0.0.1:8787.") from None
