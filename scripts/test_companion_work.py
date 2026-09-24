"""Offline real-PTY test of the companion's task/decision/review workflow."""
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT=Path(__file__).resolve().parents[1]

def main():
    with tempfile.TemporaryDirectory(prefix='aster-companion-work-') as tmp:
        root=Path(tmp);state=root/'state';project=root/'project';project.mkdir()
        env=dict(os.environ,TERM='xterm-256color',TERM_PROGRAM='iTerm.app')
        child=pexpect.spawn(str(ROOT/'aster'),['--project',str(project),'--state-dir',str(state),'--demo'],env=env,encoding='utf-8',codec_errors='replace',dimensions=(42,132))
        class Screen(pyte.Screen):
            def write_process_input(self,data):child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen);buffer='';frames=set()
        def pump():
            nonlocal buffer
            try:chunk=child.read_nonblocking(524288,timeout=.1)
            except pexpect.TIMEOUT:return screen_text(screen)
            stream.feed(chunk);buffer+=chunk
            while (m:=re.search(r'\x1b\]1337;File=[^:]*:([A-Za-z0-9+/=]+)\x07',buffer)):
                frames.add(hashlib.sha256(m[1].encode()).hexdigest());buffer=buffer[m.end():]
            assert len(buffer)<4000000
            return screen_text(screen)
        def wait(predicate,timeout=20):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                text=pump()
                if predicate(text):return
            diagnostic=json.loads((state/'diagnostics/live2d.json').read_text()) if (state/'diagnostics/live2d.json').exists() else {}
            (ROOT/'.aster/qa/companion-work-failure.json').write_text(json.dumps({'frames':len(frames),'diagnostic':diagnostic,'tail':buffer[-2000:]},ensure_ascii=False,indent=2))
            raise AssertionError(f'Timed out; distinct frames={len(frames)}, renderer={diagnostic.get("status")}\n'+screen_text(screen))
        def text(needle):wait(lambda t:needle in t)
        def close(needle):child.send('\x1b');wait(lambda t:needle not in t)
        def command(value,needle):child.send(value+'\r');text(needle)
        try:
            text('aster');command('/demo work','Which language')
            wait(lambda _:len(frames)>=3,135)
            assert 'Which language' in screen_text(screen),'Question was not visible with animation'
            child.send('2');text('Allow write_file?')
            assert not (project/'companion-demo.json').exists()
            count=len(frames);wait(lambda _:len(frames)>=count+2)
            assert 'Allow write_file?' in screen_text(screen),'Approval disappeared while rendering'
            child.send('y');text('Allow edit_file?')
            assert json.loads((project/'companion-demo.json').read_text())=={'greeting':'你好','ready':False}
            child.send('y')
            def complete(_):
                return any(json.loads(p.read_text()).get('status')=='done' for p in state.glob('*.json'))
            wait(complete,30)
            actual=json.loads((project/'companion-demo.json').read_text())
            assert actual=={'greeting':'你好','ready':True}
            command('/work','Working together')
            text('Recorded checks passed');text('3. Verify the saved result')
            close('Working together');command('/review','Review changes')
            text('+++ b/companion-demo.json');close('Review changes')
            child.setwinsize(24,80);screen.resize(24,80);text('aster')
            child.setwinsize(42,132);screen.resize(42,132);text('aster')
            child.send('/quit\r');end=time.monotonic()+12
            while time.monotonic()<end:
                try:pump()
                except pexpect.EOF:break
            else:raise AssertionError('Quit timed out')
            child.close();assert child.exitstatus==0
            sessions=[json.loads(p.read_text()) for p in state.glob('*.json')]
            work=sessions[0]['work'];assert len(work['steps'])==3 and len(work['diffs'])==2
            assert work['evidence'][0]['passed']
            qa=ROOT/'.aster/qa';qa.mkdir(parents=True,exist_ok=True)
            evidence={'question_answer_used':True,'approval_before_write':True,'exact_edit':True,'plan_steps':3,'review_diffs':2,'actual':actual,'animated_during_decisions':True,'distinct_frames':len(frames),'resize':True,'exit':0,'api_calls':0}
            (qa/'companion-work-e2e.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2))
            preview=qa/'work-preview-state';preview.mkdir(exist_ok=True)
            sessions[0]['project']=str(ROOT)
            saved=preview/(sessions[0]['id']+'.json');saved.write_text(json.dumps(sessions[0],ensure_ascii=False));saved.chmod(0o600)
            print(json.dumps(evidence,ensure_ascii=False,indent=2))
        finally:
            stop_child(child, stream)

if __name__=='__main__':main()
