"""Explicit bounded provider test: redirect an unapproved write through Ctrl+G.

Requires --run-live. One task; no provider retry. Only final.json is approved.
"""
import argparse
import json
import os
from pathlib import Path
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {'direction': 'updated', 'proof': 'steer-486'}
DIRECTION = 'Change of plan: do not create original.json. Instead write final.json containing exactly {"direction":"updated","proof":"steer-486"}, then verify final.json with check_file json_equals. No shell commands or other files.'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--run-live', action='store_true')
    args = parser.parse_args()
    if not args.run_live:
        parser.error('--run-live is required; this test consumes provider tokens')
    qa = ROOT / '.aster/qa'
    qa.mkdir(exist_ok=True, parents=True)
    state = qa / 'steering-live-sessions'
    with tempfile.TemporaryDirectory(prefix='aster-steering-live-') as tmp:
        project = Path(tmp).resolve()
        child = pexpect.spawn(str(ROOT/'aster'), ['--project', str(project), '--state-dir', str(state), '--no-live2d'], env=dict(os.environ, TERM='xterm-256color'), encoding='utf-8', codec_errors='replace', dimensions=(42,132))
        class Screen(pyte.Screen):
            def write_process_input(self, data): child.send(data)
        screen = Screen(132,42)
        stream = pyte.Stream(screen)
        def pump(display=True):
            try: stream.feed(child.read_nonblocking(65536, timeout=.1))
            except pexpect.TIMEOUT: pass
            return screen_text(screen) if display else ''
        def wait(predicate, timeout=20):
            end = time.monotonic()+timeout
            while time.monotonic()<end:
                text = pump()
                if predicate(text): return text
            raise AssertionError('Timed out: '+pump())
        result = None
        try:
            wait(lambda t: 'aster' in t)
            child.send('Use write_file to create original.json with {"direction":"original"}, then verify it using check_file json_equals. Use no shell commands and create no other files.\r')
            wait(lambda t: 'Allow write_file?' in t and 'original.json' in t, 90)
            assert not (project/'original.json').exists()
            child.send('\x07')
            wait(lambda t: 'change direction' in t)
            child.send(DIRECTION+'\r')
            approved = False
            deadline = time.monotonic()+180
            while time.monotonic()<deadline:
                text = pump()
                if 'Allow shell?' in text or 'Allow edit_file?' in text:
                    raise AssertionError('Unexpected action')
                if 'Allow write_file?' in text and 'final.json' in text and not approved:
                    assert 'steer-486' in text
                    assert not (project/'original.json').exists()
                    child.send('y')
                    approved = True
                for path in state.glob('*.json'):
                    session = json.loads(path.read_text())
                    if session['project'] == str(project) and session['status'] in ('done','error','stopped'):
                        result = session
                        break
                if result: break
            assert result and result['status']=='done', result['entries'][-1] if result else 'No final session'
            assert approved and not (project/'original.json').exists()
            assert json.loads((project/'final.json').read_text())==EXPECTED
            assert len([e for e in result['entries'] if e['role']=='you' and e['text']==DIRECTION])==1
            assert not result['pending']
            assert any(c['passed'] and c['path']=='final.json' for c in result['checks'])
            child.send('/quit\r')
            deadline = time.monotonic()+12
            while time.monotonic()<deadline:
                try: pump(False)
                except pexpect.EOF: break
            else: raise AssertionError('Terminal did not exit')
            child.close()
            assert child.exitstatus==0
            evidence = {'session':result['id'],'model':result['model'],'model_turns':sum(m['role']=='assistant' for m in result['messages']),'tools':result['tools'],'input_tokens':result['input_tokens'],'output_tokens':result['output_tokens'],'redirected_unapproved_write':True,'original_absent':True,'exact_final_json':True,'check_passed':True,'direction_recorded_once':True,'exit':child.exitstatus}
            (qa/'steering-live.json').write_text(json.dumps(evidence,indent=2))
            print(json.dumps(evidence,indent=2))
        finally:
            if child.isalive():
                child.send('\x03')
                deadline=time.monotonic()+4
                while time.monotonic()<deadline:
                    try: pump(False)
                    except pexpect.EOF: break
                if child.isalive(): child.terminate(force=True)

if __name__=='__main__': main()
