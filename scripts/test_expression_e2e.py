"""Real PTY: after a reply, the provider chooses 弄玉's expression and the renderer applies it.

A local stand-in provider streams a reply, then answers the structured expression query with
{"emotion": "happy", "strength": 0.7, "motion": "nod"}. The renderer (headless Chromium with the
stub rig from tests/fixtures/live2d-stub.js, not the real model) must confirm the emotion at that
strength and the motion, her card must show it, and the transcript must show only the reply.
No external API is called. Chromium: ASTER_TEST_CHROME, else /opt/pw-browsers.
"""
import glob, json, os, shutil, stat, sys, threading, time, tempfile, pexpect, pyte
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from terminal_helpers import InlineFrames, screen_text, stop_child
ROOT = Path(__file__).resolve().parents[1]


def stub_pet(root, chrome):
    """The same stand-in rig the Rust renderer test uses; no Pixi or Cubism code."""
    pet = root / "pet"
    model = pet / "弄玉运行档_无水印"
    (pet / "vendor").mkdir(parents=True)
    shutil.copy(ROOT / "tests/fixtures/live2d-stub.js", pet / "vendor/pixi.min.js")
    (pet / "vendor/live2dcubismcore.min.js").write_text("/* test double */")
    (pet / "vendor/pixi-live2d-display-cubism4.min.js").write_text("/* test double */")
    (model / "stub.4096").mkdir(parents=True)
    (model / "弄玉.model3.json").write_text(json.dumps({"Version": 3, "FileReferences": {
        "Moc": "stub.moc3", "Textures": ["stub.4096/texture_00.png"], "Physics": "stub.physics3.json"}},
        ensure_ascii=False))
    (model / "stub.moc3").write_text("stub")
    (model / "stub.physics3.json").write_text("{}")
    # A 1x1 PNG is enough for the stub's texture loader.
    (model / "stub.4096/texture_00.png").write_bytes(bytes.fromhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108020000009077"
        "53de0000000c4944415408d763f8cfc0000003010100c9fe92ef0000000049454e44ae426082"))
    wrapper = root / "chrome.sh"
    wrapper.write_text(f"#!/bin/sh\nexec '{chrome}' --no-sandbox \"$@\"\n")
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IEXEC)
    return pet, wrapper


CHROME = os.environ.get("ASTER_TEST_CHROME") or next(iter(sorted(glob.glob("/opt/pw-browsers/*/chrome-linux/chrome"))[::-1]), None)
if not CHROME:
    sys.exit("Set ASTER_TEST_CHROME to a Chromium binary to run this test.")
seen = []


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        seen.append({'key': self.headers.get('x-api-key'), 'body': body})
        if body.get('stream'):
            events = [
                {"type": "message_start", "message": {"usage": {"input_tokens": 50}}},
                {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "All four tests pass now. "}},
                {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Nice work, Aster."}},
                {"type": "content_block_stop", "index": 0},
                {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 12}},
                {"type": "message_stop"}]
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
            for e in events:
                self.wfile.write(f"data: {json.dumps(e)}\n\n".encode()); self.wfile.flush(); time.sleep(.1)
        else:
            answer = {"stop_reason": "end_turn", "usage": {"input_tokens": 320, "output_tokens": 24}, "content": [
                {"type": "text", "text": json.dumps({"emotion": "happy", "strength": 0.7, "motion": "nod"})}]}
            data = json.dumps(answer).encode()
            self.send_response(200); self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)


server = HTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory() as t:
    project = Path(t) / 'p'; project.mkdir(); state = Path(t) / 's'; state.mkdir(mode=0o700)
    (state / 'providers.json').write_text(json.dumps({"providers": [{"id": "stand-in", "name": "Stand-in", "base": f"http://127.0.0.1:{server.server_port}", "auth": "x-api-key", "key": "stand-in-key-7777", "models": [{"name": "stand-in-model"}]}], "default": {"provider": "stand-in", "model": "stand-in-model"}}))
    os.chmod(state / 'providers.json', 0o600)
    env = {k: v for k, v in os.environ.items() if k != 'ANTHROPIC_BASE_URL'}
    pet, wrapper = stub_pet(Path(t), CHROME)
    env.update(TERM='xterm-256color', TERM_PROGRAM='iTerm.app', ASTER_PET_DIR=str(pet), ASTER_CHROME=str(wrapper))
    child = pexpect.spawn(str(ROOT / 'target/release/aster'), ['--project', str(project), '--state-dir', str(state)], env=env, encoding='utf-8', codec_errors='replace', dimensions=(42, 132))
    screen = pyte.Screen(132, 42); out = InlineFrames(pyte.Stream(screen))

    def pump():
        try:
            out.feed(child.read_nonblocking(1 << 20, timeout=.1))
        except pexpect.TIMEOUT:
            pass
        return screen_text(screen)

    def wait(pred, timeout=40, label=''):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            s = pump()
            if pred(s):
                return s
        raise AssertionError(label + '\n' + screen_text(screen))

    def panel(command, title):
        """Open a panel and read all of it, page by page."""
        child.send(command + '\r')
        text = wait(lambda s: title in s, label=command)
        for _ in range(60):
            child.send('\x1b[6~'); time.sleep(.03); text += pump()
        child.send('\x1b'); wait(lambda s: title not in s, label='close ' + command)
        return text

    try:
        wait(lambda s: len(out.frames) >= 3, 120, 'frames')
        wait(lambda s: 'Stand-in · stand-in-model' in s, 5, 'header shows provider')
        child.send('\x1b[200~run the tests\x1b[201~'); time.sleep(.3); child.send('\r')
        wait(lambda s: 'Nice work, Aster.' in s and '● ready' in s, 30, 'reply')
        # Her card shows the emotion the renderer confirmed.
        wait(lambda s: '· happy' in s, 15, 'card shows the confirmed emotion')
        info = panel('/pet info', 'Companion interface v1')
        # The renderer's own state and receipts: happy at the provider's strength, and the nod.
        for needle in ['"emotion": "happy"', '"strength": 0.7', '"motion": "nod"', '"ok": true']:
            assert needle in info, needle
        assert '"ok": false' not in info
        status = panel('/status', 'session status')
        assert 'upstream query on' in status and 'happy 0.70 + nod' in status, status
        child.send('/quit\r')
        end = time.monotonic() + 12
        while time.monotonic() < end:
            try:
                pump()
            except pexpect.EOF:
                break
        child.close()
        assert child.exitstatus == 0, child.exitstatus
        assert len(seen) == 2, [s['body'].get('stream') for s in seen]
        turn, query = seen[0], seen[1]
        assert turn['body']['stream'] is True and '〔' not in turn['body']['system']
        assert query['key'] == 'stand-in-key-7777' and query['body']['model'] == 'stand-in-model'
        assert 'tools' not in query['body'] and 'stream' not in query['body']
        schema = query['body']['output_config']['format']['schema']
        assert schema['properties']['emotion']['enum'] == ['angry', 'happy', 'heart', 'neutral'], schema
        assert 'Nice work, Aster.' in query['body']['messages'][0]['content']
        saved = json.loads(next(p for p in state.glob('*.json') if p.name != 'providers.json').read_text())
        assert (saved['input_tokens'], saved['output_tokens']) == (370, 36), (saved['input_tokens'], saved['output_tokens'])
        assert [e['text'] for e in saved['entries'] if e['role'] == 'nongyu'] == ['All four tests pass now. Nice work, Aster.']
        assert 'stand-in-key' not in json.dumps(saved) and 'output_config' not in json.dumps(saved)
        evidence = {'provider_requests': 2, 'query_structured': True, 'query_without_tools': True,
                    'renderer_emotion': 'happy', 'renderer_strength': 0.7, 'renderer_motion_acknowledged': 'nod',
                    'card_shows_confirmed_emotion': True, 'status_line': True, 'transcript_reply_only': True,
                    'usage_counted': [370, 36], 'frames': len(out.frames), 'external_api_calls': 0, 'exit': 0}
        qa = ROOT / '.aster/qa'; qa.mkdir(parents=True, exist_ok=True)
        (qa / 'expression-e2e.json').write_text(json.dumps(evidence, indent=2))
        print(json.dumps(evidence, indent=2))
    finally:
        stop_child(child)
        server.shutdown()
