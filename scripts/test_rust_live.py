"""Explicit, bounded MiniMax test through the Rust TUI. Makes real API calls."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {'language':'Rust','companion':'弄玉','proof':'jade-486'}

def main():
    qa=ROOT/'.aster/qa';qa.mkdir(parents=True,exist_ok=True)
    state=qa/'minimax-sessions';state.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='aster-live-project-') as tmp:
        project=Path(tmp).resolve()
        (project/'AGENTS.md').write_text('# Validation project\nWrite only result.json. Do not use shell commands. The proof value must be jade-486.\n')
        binary=ROOT/'target/release/aster'
        env=dict(os.environ,TERM='xterm-256color',COLORTERM='truecolor');env.pop('NO_COLOR',None)
        child=pexpect.spawn(str(binary),['--project',str(project),'--state-dir',str(state),'--no-live2d'],env=env,encoding='utf-8',codec_errors='replace',dimensions=(42,132),timeout=2)
        class Screen(pyte.Screen):
            def write_process_input(self, data): child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen)
        def pump():
            try:stream.feed(child.read_nonblocking(65536,timeout=.1))
            except pexpect.TIMEOUT:pass
            except pexpect.EOF:raise AssertionError('Aster exited unexpectedly')
            return screen_text(screen)
        def wait(needle,timeout=15):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                if needle in pump():return
            raise AssertionError('Missing '+needle+'\n'+screen_text(screen))
        try:
            wait('aster')
            prompt='Read AGENTS.md. Create result.json with exactly '+json.dumps(EXPECTED,ensure_ascii=False)+'. Use write_file and then check_file with json_equals to verify the saved file against that exact object. Do not use shell commands. Reply briefly.'
            child.send(prompt+'\r')
            deadline=time.monotonic()+180
            approved=False;result=None
            while time.monotonic()<deadline:
                text=pump()
                if 'Allow shell?' in text:raise AssertionError('Unexpected shell request; not approved')
                if 'Allow write_file?' in text and not approved:
                    assert 'result.json' in text and 'jade-486' in text,text
                    assert not (project/'result.json').exists()
                    child.send('y');approved=True
                for p in state.glob('*.json'):
                    s=json.loads(p.read_text())
                    if s['project']==str(project) and s['status'] in ('done','error','stopped'):
                        result=s;break
                if result:break
            assert result,'No completed live turn within deadline'
            assert result['status']=='done',result['entries'][-1]
            assert approved,'The write did not pass through the permission prompt'
            actual=json.loads((project/'result.json').read_text())
            assert actual==EXPECTED,(actual,EXPECTED)
            assert result['checks'] and result['checks'][-1]['passed'],result['checks']
            assert result['input_tokens']>0 and result['output_tokens']>0
            child.send('/quit\r')
            end=time.monotonic()+10
            while time.monotonic()<end:
                try:stream.feed(child.read_nonblocking(65536,timeout=.1))
                except pexpect.TIMEOUT:pass
                except pexpect.EOF:break
            else:raise AssertionError('Aster did not exit')
            child.close();assert child.exitstatus==0
            artifact=qa/'minimax-result.json';shutil.copy2(project/'result.json',artifact)
            evidence={'session':result['id'],'provider':'MiniMax','model':result['model'],'status':result['status'],'tool_calls':result['tools'],'model_turns':sum(m['role']=='assistant' for m in result['messages']),'input_tokens':result['input_tokens'],'output_tokens':result['output_tokens'],'expected':EXPECTED,'actual':actual,'checks':result['checks'],'approval_before_write':approved,'terminal_exit':child.exitstatus}
            (qa/'rust-minimax-e2e.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2))
            print(json.dumps(evidence,ensure_ascii=False,indent=2))
        finally:
            if child.isalive():
                child.send('\x03')
                try:child.expect(pexpect.EOF,timeout=4)
                except (pexpect.TIMEOUT,pexpect.EOF):pass
                if child.isalive():child.terminate(force=True)

if __name__=='__main__':main()
