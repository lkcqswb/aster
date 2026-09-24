"""Keyboard regression for steering, follow-ups, stop, save and resume. No API calls."""
import json
import os
from pathlib import Path
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT=Path(__file__).resolve().parents[1]

def main():
    with tempfile.TemporaryDirectory(prefix='aster-queue-') as tmp:
        project=Path(tmp)/'project';project.mkdir();state=Path(tmp)/'state'
        child=None
        def start(resume=False):
            nonlocal child
            args=['--project',str(project),'--state-dir',str(state),'--demo','--no-live2d','--permissions','allow']
            if resume:args+=['--continue']
            child=pexpect.spawn(str(ROOT/'aster'),args,env=dict(os.environ,TERM='xterm-256color'),encoding='utf-8',codec_errors='replace',dimensions=(42,132))
            class Screen(pyte.Screen):
                def write_process_input(self,data):child.send(data)
            screen=Screen(132,42);stream=pyte.Stream(screen)
            def pump(display=True):
                try:stream.feed(child.read_nonblocking(65536,timeout=.1))
                except pexpect.TIMEOUT:pass
                # Ratatui can leave a wide-character continuation at the start of
                # a dirty row; pyte.display indexes that empty cell and crashes.
                return screen_text(screen) if display else ''
            return pump
        def saved():
            files=list(state.glob('*.json'))
            return json.loads(files[0].read_text()) if files else {}
        def wait(predicate,timeout=15):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                text=pump()
                if predicate(text):return
            raise AssertionError('Timed out:\n'+pump())
        def quit():
            child.send('/quit\r');end=time.monotonic()+12
            while time.monotonic()<end:
                try:pump(display=False)
                except pexpect.EOF:break
            else:raise AssertionError('Quit timed out')
            child.close();assert child.exitstatus==0
        try:
            pump=start();wait(lambda t:'aster' in t)
            child.send('steering demo\r')
            child.send('Use steer-proof-486 instead.\r')
            wait(lambda _:saved().get('status')=='done' and any(e['text']=='Use steer-proof-486 instead.' for e in saved().get('entries',[])))
            assert not (project/'stale.json').exists()
            assert json.loads((project/'steered.json').read_text())=={'updated':True}
            assert not saved()['pending']
            child.send('first queue test\r');child.send('follow-up after normal finish\x1b\r')
            wait(lambda _:saved().get('status')=='done' and any(e['text']=='follow-up after normal finish' for e in saved().get('entries',[])))
            assert not saved()['pending']
            child.send('task to interrupt\r');child.send('/follow preserved after stop\r')
            wait(lambda _:len(saved().get('pending',[]))==1)
            child.send('\x1b')
            wait(lambda _:saved().get('status')=='stopped')
            assert saved()['pending'][0]['text']=='preserved after stop'
            quit();pump=start(True);wait(lambda t:'aster' in t)
            assert len(saved()['pending'])==1
            child.send('/queue\r');wait(lambda t:'preserved after stop' in t)
            child.send('\x1b');wait(lambda t:'Messages waiting' not in t)
            child.send('/next\r')
            wait(lambda _:saved().get('status')=='done' and not saved().get('pending'))
            assert sum(e['role']=='you' and e['text']=='preserved after stop' for e in saved()['entries'])==1
            quit()
            evidence={'enter_steers':True,'stale_action_skipped':True,'alt_enter_follow_up':True,'stopped_queue_saved':True,'resume_does_not_autorun':True,'next_runs_once':True,'exit':0,'api_calls':0}
            (ROOT/'.aster/qa/queue-e2e.json').write_text(json.dumps(evidence,indent=2));print(json.dumps(evidence,indent=2))
        finally:
            if child is not None and child.isalive():child.terminate(force=True)

if __name__=='__main__':main()
