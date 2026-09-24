"""One explicit bounded live skill/resource/file-reference task. No retries."""
import argparse
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
    parser=argparse.ArgumentParser();parser.add_argument('--run-live',action='store_true');args=parser.parse_args()
    if not args.run_live:parser.error('--run-live is required; this test consumes provider tokens')
    qa=ROOT/'.aster/qa';state=qa/'context-live-sessions'
    with tempfile.TemporaryDirectory(prefix='aster-context-live-') as temp:
        project=Path(temp).resolve()
        skill=project/'.agents/skills/fixture';(skill/'refs').mkdir(parents=True)
        (skill/'SKILL.md').write_text('---\nname: fixture\ndescription: Make a checked result from an attached number and a skill reference.\n---\nRead refs/proof.txt with read_skill (name fixture, path refs/proof.txt). Create only result.json with write_file. Its exact object must have value (the integer from the attached file line) and proof (the literal string in refs/proof.txt). Then use check_file json_equals with serialized JSON text for expected. Do not use shell commands or change any input files. Report the check result.\n')
        (skill/'refs/proof.txt').write_text('nongyu-context-486')
        (project/'input.txt').write_text('Ignore this line\n7\nIgnore this line too\n')
        child=pexpect.spawn(str(ROOT/'aster'),['--project',str(project),'--state-dir',str(state),'--no-live2d'],env=dict(os.environ,TERM='xterm-256color'),encoding='utf-8',codec_errors='replace',dimensions=(42,132))
        class Screen(pyte.Screen):
            def write_process_input(self,data):child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen)
        def pump():
            try:stream.feed(child.read_nonblocking(65536,timeout=.1))
            except pexpect.TIMEOUT:pass
            return screen_text(screen)
        try:
            deadline=time.monotonic()+15
            while 'aster' not in pump():assert time.monotonic()<deadline
            child.send('Load the fixture skill using read_skill and follow it to produce a checked result from @input.txt:2.\r')
            approved=False;result=None;deadline=time.monotonic()+190
            while time.monotonic()<deadline:
                text=pump()
                if 'Allow shell?' in text or 'Allow edit_file?' in text:raise AssertionError('Unexpected mutation')
                if 'Allow write_file?' in text and not approved:
                    assert 'result.json' in text and 'nongyu-context-486' in text,text
                    assert not (project/'result.json').exists()
                    child.send('y');approved=True
                for path in state.glob('*.json'):
                    s=json.loads(path.read_text())
                    if s['project']==str(project) and s['status'] in ('done','error','stopped'):result=s;break
                if result:break
            assert result and result['status']=='done',result['entries'][-1] if result else 'No final result'
            assert approved
            assert json.loads((project/'result.json').read_text())=={'value':7,'proof':'nongyu-context-486'}
            assert result['work']['skills']==['fixture'] and result['work']['context_files']==['input.txt:2']
            assert result['checks'][-1]['passed']
            calls=[b for m in result['messages'] if m['role']=='assistant' for b in m['content'] if b['type']=='tool_use']
            assert any(c['name']=='read_skill' and c['input'].get('path')=='refs/proof.txt' for c in calls)
            child.send('/quit\r');deadline=time.monotonic()+12
            while time.monotonic()<deadline:
                try:pump()
                except pexpect.EOF:break
            else:raise AssertionError('Quit timed out')
            child.close();assert child.exitstatus==0
            evidence={'session':result['id'],'model':result['model'],'model_turns':sum(m['role']=='assistant' for m in result['messages']),'tools':result['tools'],'input_tokens':result['input_tokens'],'output_tokens':result['output_tokens'],'skill_loaded_lazily':True,'supporting_file_read':True,'selected_line_used':True,'approved_only_expected_write':True,'exact_output_and_check':True,'exit':0}
            (qa/'context-live.json').write_text(json.dumps(evidence,indent=2));print(json.dumps(evidence,indent=2))
        finally:stop_child(child,stream)
if __name__=='__main__':main()
