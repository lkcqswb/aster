"""Own-process SIGTERM/SIGHUP regressions. No API calls or user terminal control."""
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
COMMAND = "echo $$ > shell.pid; sleep 30 & echo $! > worker.pid; echo shutdown-fixture-running; wait; echo unexpected > completed"


def processes():
    output = subprocess.check_output(["ps", "-axo", "pid=,ppid=,stat=,command="], text=True)
    result = {}
    for line in output.splitlines():
        parts = line.strip().split(None, 3)
        if len(parts) == 4:
            result[int(parts[0])] = (int(parts[1]), parts[2], parts[3])
    return result


def descendants(pid):
    snapshot = processes()
    owned = {pid}
    while True:
        added = {p for p, (parent, _, _) in snapshot.items() if parent in owned} - owned
        if not added:
            return {p: snapshot[p] for p in owned - {pid} if p in snapshot}
        owned.update(added)


def assert_gone(owned):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        snapshot = processes()
        remaining = {pid: row for pid, row in snapshot.items()
                     if pid in owned and row[2] == owned[pid][2] and not row[1].startswith("Z")}
        if not remaining:
            return
        time.sleep(.05)
    raise AssertionError("Owned processes remained after shutdown: " + repr(remaining))


def terminal_case(root, name, sig, mode, live2d=False):
    project = root / name
    project.mkdir()
    state = project / "sessions"
    argv = ["--project", str(project), "--state-dir", str(state), "--demo"]
    if not live2d:
        argv.append("--no-live2d")
    if mode == "command":
        argv.extend(["--permissions", "allow"])
    child = pexpect.spawn(str(ROOT / "aster"), argv,
                          env=dict(os.environ, TERM="xterm-256color", TERM_PROGRAM="iTerm.app"),
                          encoding="utf-8", codec_errors="replace", dimensions=(42, 132))

    class Screen(pyte.Screen):
        def write_process_input(self, data):
            child.send(data)

    screen = Screen(132, 42)
    stream = pyte.Stream(screen)
    frames = set()
    buffer = ""
    raw = ""

    def pump():
        nonlocal buffer, raw
        try:
            chunk = child.read_nonblocking(524288, timeout=.1)
        except pexpect.TIMEOUT:
            return
        stream.feed(chunk)
        raw = (raw + chunk)[-100000:]
        buffer += chunk
        while (match := re.search(r"\x1b\]1337;File=[^:]*:([A-Za-z0-9+/=]+)\x07", buffer)):
            frames.add(hashlib.sha256(match[1].encode()).hexdigest())
            buffer = buffer[match.end():]
        if len(buffer) > 4_000_000:
            buffer = buffer[-1_000_000:]

    def wait(predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pump()
            if predicate():
                return
        raise AssertionError("Timed out: " + screen_text(screen))

    try:
        wait(lambda: "aster" in screen_text(screen))
        if live2d:
            wait(lambda: len(frames) >= 3, 135)
        if mode in ("command", "approval"):
            child.send("/run " + COMMAND + "\r")
            if mode == "command":
                wait(lambda: (project / "worker.pid").exists())
            else:
                wait(lambda: "Allow shell?" in screen_text(screen))
        owned = descendants(child.pid)
        profiles = set()
        for _, _, command in owned.values():
            match = re.search(r"--user-data-dir=([^ ]+)", command)
            if match:
                profiles.add(Path(match[1]))
        if live2d:
            assert profiles, "Renderer process was not observed"
        if mode == "command":
            for file in ("shell.pid", "worker.pid"):
                pid = int((project / file).read_text())
                assert pid in owned, (file, pid, owned)
        started = time.monotonic()
        os.kill(child.pid, sig)
        deadline = started + 12
        while time.monotonic() < deadline:
            try:
                pump()
            except pexpect.EOF:
                break
        else:
            raise AssertionError("Signal did not terminate Aster")
        child.close()
        assert child.exitstatus == 0, (child.exitstatus, child.signalstatus)
        assert "\x1b[?1049l" in raw, "Alternate screen was not restored"
        assert_gone(owned)
        assert all(not profile.exists() for profile in profiles), profiles
        saved = json.loads(next(state.glob("*.json")).read_text())
        assert saved["work"].get("model_requests", 0) == 0
        assert not (project / "completed").exists()
        if mode in ("command", "approval"):
            assert saved["status"] in ("stopped", "interrupted"), saved["status"]
        if mode == "command":
            assert saved["work"]["command"]["stopped"]
        if mode == "approval":
            assert not (project / "shell.pid").exists()
        return {"case": name, "signal": signal.Signals(sig).name, "status": saved["status"],
                "seconds": round(time.monotonic() - started, 3), "owned_processes_cleaned": len(owned),
                "profiles_removed": len(profiles), "distinct_frames": len(frames), "terminal_restored": True}
    finally:
        stop_child(child, stream)


def headless_case(root):
    project = root / "headless-command"
    project.mkdir()
    state = project / "sessions"
    child = subprocess.Popen([str(ROOT / "aster"), "--project", str(project), "--state-dir", str(state),
                              "--no-live2d", "--demo", "--permissions", "allow", "--prompt", "/run " + COMMAND],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        deadline = time.monotonic() + 10
        while not (project / "worker.pid").exists() and time.monotonic() < deadline:
            assert child.poll() is None
            time.sleep(.05)
        assert (project / "worker.pid").exists()
        owned = descendants(child.pid)
        child.send_signal(signal.SIGTERM)
        stdout, stderr = child.communicate(timeout=10)
        assert child.returncode != 0, (stdout, stderr)
        assert_gone(owned)
        saved = json.loads(next(state.glob("*.json")).read_text())
        assert saved["status"] in ("stopped", "interrupted")
        assert saved["work"]["command"]["stopped"]
        assert saved["work"]["model_requests"] == 0
        assert not (project / "completed").exists()
        return {"case": "headless-command", "signal": "SIGTERM", "exit": child.returncode,
                "status": saved["status"], "owned_processes_cleaned": len(owned)}
    finally:
        if child.poll() is None:
            child.terminate()
            child.communicate(timeout=10)


def main():
    with tempfile.TemporaryDirectory(prefix="aster-shutdown-") as temporary:
        root = Path(temporary)
        cases = [terminal_case(root, "renderer-idle", signal.SIGHUP, "idle", live2d=True),
                 terminal_case(root, "pending-approval", signal.SIGTERM, "approval"),
                 terminal_case(root, "running-command", signal.SIGTERM, "command"), headless_case(root)]
    result = {"cases": cases, "api_calls": 0}
    (ROOT / ".aster/qa/shutdown-e2e.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
