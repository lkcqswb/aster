"""Real PTY: a provider reply with hidden emotion cues moves 弄玉 through the real renderer.

A local stand-in provider streams "〔happy〕 … [[wave]] …". The transcript must show only the words,
her card the feeling, and the renderer (headless Chromium with the stub rig from
tests/fixtures/live2d-stub.js, not the real model) must apply the emotion and play the gesture.
No external API is called. Requires ASTER_TEST_CHROME=/path/to/chromium.
"""
import json, os, re, shutil, stat, sys, threading, time, tempfile, pexpect, pyte
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
    (model / "motions").mkdir(parents=True)
    (model / "stub.4096").mkdir()
    (model / "弄玉.model3.json").write_text(json.dumps({"Version": 3, "FileReferences": {
        "Moc": "stub.moc3", "Textures": ["stub.4096/texture_00.png"], "Physics": "stub.physics3.json",
        "Motions": {"Idle": [{"File": "motions/idle.motion3.json"}], "Wave": [{"File": "motions/wave.motion3.json"}]}}},
        ensure_ascii=False))
    (model / "stub.moc3").write_text("stub")
    (model / "stub.physics3.json").write_text("{}")
    (model / "motions/idle.motion3.json").write_text('{"Version":3,"Meta":{"Duration":1.5},"StubAmplitude":6}')
    (model / "motions/wave.motion3.json").write_text('{"Version":3,"Meta":{"Duration":1.2},"StubAmplitude":8}')
    # A 1x1 PNG is enough for the stub's texture loader.
    (model / "stub.4096/texture_00.png").write_bytes(bytes.fromhex(
        "89504e470d0a1a0a0000000d49484452000000010000000108020000009077"
        "53de0000000c4944415408d763f8cfc0000003010100c9fe92ef0000000049454e44ae426082"))
    wrapper = root / "chrome.sh"
    wrapper.write_text(f"#!/bin/sh\nexec '{chrome}' --no-sandbox \"$@\"\n")
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IEXEC)
    return pet, wrapper


CHROME = os.environ.get("ASTER_TEST_CHROME")
if not CHROME:
    sys.exit("Set ASTER_TEST_CHROME to a Chromium binary to run this test.")
seen = []
class Provider(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        seen.append({'key': self.headers.get('x-api-key'), 'model': body['model'], 'system': body['system']})
        events = [
            {"type": "message_start", "message": {"usage": {"input_tokens": 50}}},
            {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
        ] + [{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": t}}
             for t in ["〔hap", "py〕 Hello from the stand-in ", "provider. [[wa", "ve]]Nice to meet you."]] + [
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 12}},
            {"type": "message_stop"}]
        self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
        for e in events:
            self.wfile.write(f"data: {json.dumps(e)}\n\n".encode()); self.wfile.flush(); time.sleep(.15)
server = HTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory() as t:
    project = Path(t) / 'p'; project.mkdir(); state = Path(t) / 's'; state.mkdir(mode=0o700)
    (state / 'providers.json').write_text(json.dumps({"providers": [{"id": "stand-in", "name": "Stand-in", "base": f"http://127.0.0.1:{server.server_port}", "auth": "x-api-key", "key": "stand-in-key-7777", "models": [{"name": "stand-in-model"}]}], "default": {"provider": "stand-in", "model": "stand-in-model"}}))
    env = {k: v for k, v in os.environ.items() if k != 'ANTHROPIC_BASE_URL'}
    pet, wrapper = stub_pet(Path(t), CHROME)
    env.update(TERM='xterm-256color', TERM_PROGRAM='iTerm.app', ASTER_PET_DIR=str(pet), ASTER_CHROME=str(wrapper))
    child = pexpect.spawn(str(ROOT / 'target/release/aster'), ['--project', str(project), '--state-dir', str(state)], env=env, encoding='utf-8', codec_errors='replace', dimensions=(42, 132))
    screen = pyte.Screen(132, 42); out = InlineFrames(pyte.Stream(screen))
    def pump():
        try: out.feed(child.read_nonblocking(1 << 20, timeout=.1))
        except pexpect.TIMEOUT: pass
        return screen_text(screen)
    def wait(pred, timeout=40, label=''):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            s = pump()
            if pred(s): return s
        raise AssertionError(label + '\n' + screen_text(screen))
    def status():
        child.send('/status\r')
        s = wait(lambda s: 'session status' in s, label='status')
        text = ''
        for _ in range(40):
            text += s
            child.send('\x1b[6~'); time.sleep(.05); s = pump()
        child.send('\x1b'); wait(lambda s: 'session status' not in s, label='close status')
        return text
    try:
        wait(lambda s: len(out.frames) >= 3, 120, 'frames')
        wait(lambda s: 'Stand-in · stand-in-model' in s, 5, 'header shows provider')
        child.send('\x1b[200~hello\x1b[201~'); time.sleep(.3); child.send('\r')
        shown = wait(lambda s: 'Nice to meet you.' in s and '● ready' in s, 30, 'reply')
        assert '〔' not in shown and '[[wave' not in shown, 'cue leaked'
        assert 'Hello from the stand-in provider.' in shown
        card = wait(lambda s: '· happy' in s, 5, 'card shows feeling')
        # Gestures play one at a time; the wave follows the nod that greeted the message.
        waved = False
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline and not waved:
            info = status()
            assert '"emotion": "happy"' in info, 'renderer did not apply happy'
            lines = info.splitlines()
            waved = any('"name": "wave"' in l or '"name": "Wave"' in l for l in lines)
        assert waved, 'renderer never played the wave cue'
        child.send('/act bow\r'); time.sleep(1.5); pump()
        info2 = status()
        assert 'bow' in info2, 'bow not played'
        assert seen and seen[0]['key'] == 'stand-in-key-7777' and seen[0]['model'] == 'stand-in-model'
        assert '〔〕' in seen[0]['system'] or '〔' in seen[0]['system'], 'persona lacks cue instructions'
        saved = json.loads(next(p for p in state.glob('*.json') if p.name != 'providers.json').read_text())
        reply = [e['text'] for e in saved['entries'] if e['role'] == 'nongyu'][-1]
        assert reply == 'Hello from the stand-in provider. Nice to meet you.', reply
        assistant = [m for m in saved['messages'] if m['role'] == 'assistant'][-1]
        assert '〔happy〕' in json.dumps(assistant, ensure_ascii=False), 'provider blocks lost the cue'
        assert 'stand-in-key' not in json.dumps(saved)
        evidence = {'reply_without_cues': True, 'card_feeling': 'happy', 'renderer_emotion': 'happy',
                    'renderer_gestures': ['nod', 'wave', 'bow'], 'frames': len(out.frames),
                    'provider_key_header_only': True, 'cue_kept_in_provider_blocks': True, 'external_api_calls': 0}
        qa = ROOT / '.aster/qa'; qa.mkdir(parents=True, exist_ok=True)
        (qa / 'emotion-e2e.json').write_text(json.dumps(evidence, indent=2))
        print(json.dumps(evidence, indent=2))
    finally:
        stop_child(child)
        server.shutdown()
