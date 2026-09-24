"""Actual animated TUI: custom emotion/motion controls and immutable local assets."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]


def asset_hashes():
    model = Path.home() / 'desktop-pet/assets/弄玉运行档_无水印'
    definition = json.loads((model / '弄玉.model3.json').read_text())
    names = ['弄玉.model3.json'] + [definition['FileReferences'][k] for k in ['Moc', 'Physics', 'DisplayInfo'] if k in definition['FileReferences']] + definition['FileReferences']['Textures']
    return {n: hashlib.sha256((model / n).read_bytes()).hexdigest() for n in names}


def main():
    before = asset_hashes()
    with tempfile.TemporaryDirectory(prefix='aster-companion-api-') as temporary:
        project = Path(temporary).resolve() / 'project'
        project.mkdir()
        state = Path(temporary) / 'sessions'
        child = pexpect.spawn(str(ROOT / 'aster'), ['--project', str(project), '--state-dir', str(state),
            '--companion-profile', str(ROOT / 'examples/companion-custom.json'), '--texture-size', '1024', '--demo'],
            env=dict(os.environ, TERM='xterm-256color', TERM_PROGRAM='iTerm.app'),
            encoding='utf-8', codec_errors='replace', dimensions=(42, 132))
        screen = pyte.Screen(132, 42)
        output = InlineFrames(pyte.Stream(screen))

        def pump():
            try:
                output.feed(child.read_nonblocking(524288, timeout=.1))
            except pexpect.TIMEOUT:
                pass
            return screen_text(screen)

        def wait(predicate, timeout=20):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if predicate(pump()):
                    return
            raise AssertionError('Timed out: ' + screen_text(screen))

        def command(text):
            child.send('\x1b[200~' + text + '\x1b[201~')
            wait(lambda s: text in '\n'.join(s.splitlines()[-6:]))
            child.send('\r')

        def frames(n=3):
            start = len(output.frames)
            wait(lambda _: len(output.frames) >= start + n)

        def close_panel():
            child.send('\x1b')
            frames(2)
            wait(lambda t: '和弄玉说说' in t)

        def inspect_result(needle):
            frames()
            command('/pet info')
            wait(lambda t: 'Companion interface v1' in t)
            child.send('\x1b[F')
            wait(lambda t: needle in t and '"ok": true' in t)
            text = screen_text(screen)
            close_panel()
            return text

        try:
            wait(lambda _: len(output.frames) >= 3, 135)
            command('/emotion')
            wait(lambda t: 'Companion emotions' in t and 'focused' in t)
            close_panel()
            command('/motion')
            wait(lambda t: 'Companion motions' in t and 'acknowledge' in t and 'shake' in t)
            close_panel()
            command('/emotion focused 0.6')
            result = inspect_result('"emotion": "focused"')
            assert '"strength": 0.6' in result
            assert all(n in result for n in ['eye_y', 'smile_l', 'smile_r'])
            command('/motion acknowledge 0.8')
            inspect_result('"motion": "acknowledge"')
            command('/motion missing')
            wait(lambda t: 'Choose a configured name' in t)
            command('/emotion happy 2')
            wait(lambda t: 'strength from 0 to 1' in t)
            command('/pet reset')
            inspect_result('"reset": true')
            # Renderer info is a snapshot. Re-open it after each control.
            command('/pet info')
            wait(lambda t: 'Companion interface v1' in t)
            child.setwinsize(24, 80)
            screen.resize(24, 80)
            screen.reset()
            wait(lambda t: 'Companion interface v1' in t)
            close_panel()
            command('/quit')
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                try:
                    pump()
                except pexpect.EOF:
                    break
            else:
                raise AssertionError('Quit timed out')
            child.close()
            assert child.exitstatus == 0
            session = json.loads(next(state.glob('*.json')).read_text())
            assert not session['messages'] and session['input_tokens'] == session['output_tokens'] == 0
            assert before == asset_hashes()
            evidence = {'custom_emotion_strength_acknowledged': True, 'all_emotion_channels_supported': True,
                'custom_motion_acknowledged': True, 'unknown_name_and_invalid_strength_rejected': True,
                'reset_acknowledged': True, 'source_asset_hashes_unchanged': len(before),
                'distinct_frames': len(output.frames), 'resize': True, 'api_calls': 0, 'exit': 0}
            (ROOT / '.aster/qa/companion-api-e2e.json').write_text(json.dumps(evidence, indent=2))
            print(json.dumps(evidence, indent=2))
        except Exception:
            (ROOT / '.aster/qa/companion-api-failure.json').write_text(json.dumps({'screen': screen_text(screen), 'frames': len(output.frames)}, ensure_ascii=False, indent=2))
            raise
        finally:
            stop_child(child, output)


if __name__ == '__main__':
    main()
