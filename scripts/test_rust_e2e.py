"""Keyboard-driven Rust workbench smoke test. Uses demo tools only; no API calls."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import InlineFrames, screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--live2d', action='store_true')
    args = parser.parse_args()
    binary = ROOT/'target/release/aster'
    if not binary.exists():
        binary = ROOT/'target/debug/aster'
    with tempfile.TemporaryDirectory(prefix='aster-e2e-') as temp:
        project=Path(temp)/'project';project.mkdir()
        state=Path(temp)/'sessions'
        (project/'AGENTS.md').write_text('# Test project\nUse the jade-check convention.\n')
        env=dict(os.environ, TERM='xterm-256color', COLORTERM='truecolor', TERM_PROGRAM='iTerm.app')
        env.pop('NO_COLOR',None)
        command=[str(binary),'--project',str(project),'--state-dir',str(state),'--demo']
        if not args.live2d: command+=['--no-live2d']
        child=pexpect.spawn(command[0],command[1:],env=env,encoding='utf-8',codec_errors='replace',dimensions=(42,132),timeout=2)
        class Screen(pyte.Screen):
            def write_process_input(self, data):
                child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen)
        output=InlineFrames(stream)
        def pump():
            try:
                chunk=child.read_nonblocking(524288,timeout=.1)
                output.feed(chunk)
            except pexpect.TIMEOUT: pass
            except pexpect.EOF: raise AssertionError('Terminal exited unexpectedly')
            return screen_text(screen)
        def wait(needle,timeout=12):
            deadline=time.monotonic()+timeout
            while time.monotonic()<deadline:
                if needle in pump(): return
            raise AssertionError(f'Missing screen text {needle!r}\n'+ screen_text(screen))
        def send(command,needle):
            def composer():
                return next((line.split('›',1)[1] for line in screen_text(screen).splitlines()[-5:]
                             if line.lstrip().startswith('›')), '')
            child.send('\x1b[200~'+command+'\x1b[201~')
            deadline=time.monotonic()+12
            while time.monotonic()<deadline:
                pump()
                if command in composer():break
            else:raise AssertionError('Composer did not receive '+command)
            child.send('\r');wait(needle)
            while time.monotonic()<deadline:
                pump()
                if command not in composer():break
            else:raise AssertionError('Command was not submitted: '+command)
        def dismiss(needle,timeout=12):
            child.send('\x1b')
            deadline=time.monotonic()+timeout
            while time.monotonic()<deadline:
                if needle not in pump(): return
            raise AssertionError(f'Panel did not close: {needle}')
        try:
            wait('aster')
            send('/agents','jade-check')
            dismiss('Project instructions')
            child.send('/');wait('Start a fresh conversation')
            dismiss('Start a fresh conversation')
            send('/rename Jade terminal test','Jade terminal test')
            send('/demo','Allow write_file?')
            assert not (project/'aster-demo.json').exists()
            child.send('y');wait('check_file',timeout=15)
            wait('已通过独立 JSON 检查',timeout=15)
            wait('·  ready',timeout=15)
            assert json.loads((project/'aster-demo.json').read_text())=={'companion':'弄玉','ready':True}
            send('/fork Jade branch','Jade branch')
            send('/sessions','Your conversations')
            wait('Jade terminal test')
            child.send('Jade terminal test');time.sleep(.2);pump()
            child.send('\r');deadline=time.monotonic()+12
            while time.monotonic()<deadline:
                text=pump()
                if 'Your conversations' not in text and 'Jade terminal test' in text:break
            else:raise AssertionError('Session picker did not resume')
            send('/export','Saved ')
            assert list((state/'exports').glob('*.md'))
            send('/plan','Plan mode')
            send('/build','Build mode')
            send('/mood happy','Build mode')
            if args.live2d:
                deadline=time.monotonic()+65
                while time.monotonic()<deadline:
                    pump()
                    if output.frames:break
                else: raise AssertionError('Live2D never emitted terminal graphics')
            child.setwinsize(24,80);screen.resize(24,80);screen.reset();wait('和弄玉说说')
            child.setwinsize(42,132);screen.resize(42,132);screen.reset();wait('和弄玉说说')
            child.send('/quit\r')
            deadline=time.monotonic()+12
            while time.monotonic()<deadline:
                try:
                    chunk=child.read_nonblocking(524288,timeout=.1);output.feed(chunk)
                except pexpect.TIMEOUT: pass
                except pexpect.EOF: break
            else: raise AssertionError('Aster did not exit\n'+screen_text(screen))
            child.close()
            assert child.exitstatus==0,(child.exitstatus,child.signalstatus,child.before[-3000:])
            sessions=[json.loads(p.read_text()) for p in state.glob('*.json')]
            assert len(sessions)==2
            assert any(s['parent'] for s in sessions)
            assert any(s['checks'] and s['checks'][0]['passed'] for s in sessions)
            assert all(p.stat().st_mode&0o777==0o600 for p in state.glob('*.json'))
            evidence={'sessions':len(sessions),'checked_file':True,'approval_before_write':True,'AGENTS_md':True,'slash_completion':True,'fork':True,'resume':True,'export':True,'resize':[[132,42],[80,24]],'live2d_packets':args.live2d,'exit':child.exitstatus}
            qa=ROOT/'.aster/qa';qa.mkdir(parents=True,exist_ok=True)
            (qa/'rust-terminal-e2e.json').write_text(json.dumps(evidence,indent=2))
            print(json.dumps(evidence,indent=2))
        finally:
            stop_child(child, stream)

if __name__=='__main__':main()
