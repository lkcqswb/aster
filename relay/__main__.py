import argparse
import json
import os
import sys
from pathlib import Path

from .engine import Engine
from .providers import load_env
from .server import make_server


def main():
    os.umask(0o077)
    project = Path(__file__).resolve().parent.parent
    load_env(project / ".env")
    parser = argparse.ArgumentParser(prog="aster", description="Aster — an observable local agent harness")
    parser.add_argument("--data-dir", type=Path, default=project / ".relay" / "runs")
    sub = parser.add_subparsers(dest="command", required=True)
    serve = sub.add_parser("serve", help="Open the local dashboard")
    serve.add_argument("--port", type=int, default=8787)
    tui = sub.add_parser("tui", prog="aster", help="Open the Aster terminal workbench")
    tui.add_argument("--connect", help="Attach to a running local dashboard, e.g. http://127.0.0.1:8787")
    tui.add_argument("--no-companion", action="store_true", help="Hide the terminal portrait")
    tui.add_argument("--companion", type=Path, help="Use a local portrait image")
    tui.add_argument("--provider", choices=["demo", "minimax"], help="Default agent in the new-run form")
    tui.add_argument("--export-dir", type=Path, default=project / ".relay" / "exports")
    run = sub.add_parser("run", help="Run a task once")
    run.add_argument("--task", default="revenue-audit")
    run.add_argument("--task-file", type=Path)
    run.add_argument("--provider", choices=["demo", "minimax"], default="demo")
    run.add_argument("--model")
    run.add_argument("--max-steps", type=int, default=12)
    run.add_argument("--max-output-tokens", type=int, default=2048)
    run.add_argument("--total-output-tokens", type=int, default=12000)
    run.add_argument("--max-seconds", type=int, default=180)
    args = parser.parse_args()
    if args.command == "tui":
        if not sys.stdin.isatty() or not sys.stdout.isatty():
            parser.error("The TUI needs an interactive terminal. Launch ./aster in a terminal window.")
        try:
            from .tui import run_terminal
        except ModuleNotFoundError:
            parser.error("Install terminal dependencies with .venv/bin/python -m pip install -r requirements-tui.txt, then use ./aster.")
        from .tui_backend import make_backend
        try:
            backend = make_backend(args.data_dir, args.connect)
        except (ValueError, ConnectionError) as exc:
            parser.error(str(exc))
        run_terminal(backend, args.export_dir, companion_path=args.companion,
                     show_companion=not args.no_companion, default_provider=args.provider)
        return
    try:
        engine = Engine(args.data_dir)
    except ValueError as exc:
        parser.error(str(exc))
    if args.command == "serve":
        server = make_server(engine, args.port)
        print(f"Aster is ready at http://127.0.0.1:{server.server_port}", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            if engine.active:
                engine.cancel(engine.active)
        finally:
            server.server_close()
    else:
        config = {"task_id": args.task, "provider": args.provider, "model": args.model,
                  "limits": {"max_steps": args.max_steps, "max_seconds": args.max_seconds,
                             "max_output_tokens": args.max_output_tokens,
                             "total_output_tokens": args.total_output_tokens}}
        if args.task_file:
            config["task"] = json.loads(args.task_file.read_text())
        try:
            run_id = engine.start(config, background=False)
        except ValueError as exc:
            parser.error(str(exc))
        result = engine.get(run_id)
        print(json.dumps({k: result[k] for k in ("id", "status", "steps", "tool_calls", "input_tokens", "output_tokens", "checks")}, indent=2))
        if result.get("error"):
            print(result["error"])
        raise SystemExit(0 if result["status"] in {"passed", "unverified"} else 1)


if __name__ == "__main__":
    main()
