"""Offline PTY regression: live command output, failure, timeout and stop."""
import argparse
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
    parser=argparse.ArgumentParser();parser.add_argument('--live2d',action='store_true');args=parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='aster-command-') as temp:
        project=Path(temp)/'project';project.mkdir();state=Path(temp)/'state'
        argv=['--project',str(project),'--state-dir',str(state),'--demo']
        if not args.live2d:argv+=['--no-live2d']
        child=pexpect.spawn(str(ROOT/'aster'),argv,env=dict(os.environ,TERM='xterm-256color',TERM_PROGRAM='iTerm.app'),encoding='utf-8',codec_errors='replace',dimensions=(42,132))
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
            if len(buffer)>4_000_000:buffer=buffer[-1_000_000:]
            return screen_text(screen)
        def wait(predicate,timeout=15):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                text=pump()
                if predicate(text):return text
            raise AssertionError('Timed out: '+pump())
        def saved():
            files=list(state.glob('*.json'))
            return json.loads(files[0].read_text()) if files else {}
        def panel_has(text, expected):
            return any(line.split('│',1)[1].rsplit('│',1)[0].strip()==expected for line in text.splitlines() if '│' in line)
        def begin(kind):
            child.send('/demo command'+(' '+kind if kind else '')+'\r')
            wait(lambda t:'Allow shell?' in t)
            child.send('y');wait(lambda t:'Allow shell?' not in t)
            child.send('\x1bOS');wait(lambda t:'Command output' in t)
        def close():
            child.send('\x1b');wait(lambda t:'Command output' not in t)
        try:
            wait(lambda t:'aster' in t)
            if args.live2d:wait(lambda _:len(frames)>=3,135)
            before=len(frames)
            begin('')
            wait(lambda t:panel_has(t,'phase one'))
            assert saved()['status']!='done'
            wait(lambda _:saved().get('status')=='done')
            failed_session=saved();failed=failed_session['work']['command']
            assert failed['exit_code']==7 and failed['stderr_tail']=='fixture failure\n'
            assert not failed['running'] and not failed['timed_out']
            wait(lambda t:'Failed' in t and 'fixture failure' in t)
            if args.live2d:assert len(frames)>before+1
            close();begin('timeout')
            wait(lambda _:saved().get('status')=='done' and saved().get('work',{}).get('command',{}).get('timed_out'))
            wait(lambda t:'Timed out' in t)
            assert 'should not finish\n' not in saved()['work']['command']['stdout_tail']
            close();begin('stop')
            wait(lambda t:panel_has(t,'command started'))
            close();child.send('\x1b')
            wait(lambda _:saved().get('status')=='stopped')
            stopped=saved()['work']['command']
            assert stopped['stopped'] and not stopped['timed_out'] and not stopped['running']
            child.send('/output\r');wait(lambda t:'Command output' in t and 'Stopped by you' in t)
            close();child.send('/quit\r')
            end=time.monotonic()+12
            while time.monotonic()<end:
                try:pump()
                except pexpect.EOF:break
            else:
                diagnostic={'screen':screen_text(screen),'alive':child.isalive(),'pid':child.pid,'session':saved()}
                p=state/'diagnostics/live2d.json'
                if p.exists():diagnostic['renderer']=json.loads(p.read_text())
                (ROOT/'.aster/qa/command-quit-failure.json').write_text(json.dumps(diagnostic,ensure_ascii=False,indent=2))
                raise AssertionError('Quit timed out; app alive='+str(diagnostic['alive']))
            child.close();assert child.exitstatus==0
            evidence={'output_visible_before_exit':True,'exit_7_and_stderr_preserved':True,'timeout_distinct_from_stop':True,'interrupted_command_stopped':True,'saved_command_output':True,'companion_frames':len(frames),'exit':0,'api_calls':0}
            preview=ROOT/'.aster/qa/command-preview-state';preview.mkdir(exist_ok=True)
            failed_session['project']=str(ROOT)
            sample=preview/(failed_session['id']+'.json');sample.write_text(json.dumps(failed_session,ensure_ascii=False));sample.chmod(0o600)
            (ROOT/'.aster/qa'/('command-live2d-e2e.json' if args.live2d else 'command-e2e.json')).write_text(json.dumps(evidence,indent=2));print(json.dumps(evidence,indent=2))
        finally:stop_child(child,stream)
if __name__=='__main__':main()
