"""One explicitly invoked, bounded MiniMax coding task through the real terminal.

Run with --run-live. Never retries a provider failure. All file and shell actions
must match the fixture before this driver approves them.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import pexpect
import pyte
from terminal_helpers import screen_text, stop_child

ROOT=Path(__file__).resolve().parents[1]
SOURCE='def total(prices):\n    return sum(prices) + 1\n'
TESTS='''import unittest
from calculate import total
class TotalTests(unittest.TestCase):
    def test_empty(self): self.assertEqual(total([]), 0)
    def test_items(self): self.assertEqual(total([3, 8, 12]), 23)
    def test_negative(self): self.assertEqual(total([-5, 2]), -3)
'''

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--run-live',action='store_true');parser.add_argument('--recover',action='store_true');args=parser.parse_args()
    if not args.run_live:parser.error('--run-live is required; this test consumes provider tokens')
    qa=ROOT/'.aster/qa';qa.mkdir(exist_ok=True,parents=True)
    state=qa/('recover-live-sessions' if args.recover else 'edit-live-sessions')
    with tempfile.TemporaryDirectory(prefix='aster-edit-live-') as tmp:
        project=Path(tmp).resolve()
        (project/'calculate.py').write_text(SOURCE)
        (project/'test_calculate.py').write_text(TESTS)
        (project/'AGENTS.md').write_text('# Fixture\nModify only calculate.py using edit_file. Do not replace the whole file. Run exactly `python3 -m unittest -v` to validate. Do not change tests or create other files.\n')
        child=pexpect.spawn(str(ROOT/'aster'),['--project',str(project),'--state-dir',str(state),'--no-live2d'],env=dict(os.environ,TERM='xterm-256color'),encoding='utf-8',codec_errors='replace',dimensions=(42,132))
        class Screen(pyte.Screen):
            def write_process_input(self,data):child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen)
        def pump():
            try:stream.feed(child.read_nonblocking(65536,timeout=.1))
            except pexpect.TIMEOUT:pass
            return screen_text(screen)
        try:
            deadline=time.monotonic()+10
            while 'aster' not in pump():assert time.monotonic()<deadline
            reproduced_without_model=False
            if args.recover:
                child.send('/run python3 -m unittest -v\r');deadline=time.monotonic()+15
                while 'Allow shell?' not in pump():assert time.monotonic()<deadline
                child.send('y');first=None
                while time.monotonic()<deadline:
                    pump()
                    for path in state.glob('*.json'):
                        candidate=json.loads(path.read_text())
                        if candidate['project']==str(project) and candidate['status']=='done':first=candidate;break
                    if first:break
                assert first and first['work']['command']['exit_code']!=0
                assert first['work']['model_requests']==0 and first['input_tokens']==0
                reproduced_without_model=True
                child.send('/recover\r');deadline=time.monotonic()+10
                while time.monotonic()<deadline:
                    pump()
                    if json.loads((state/(first['id']+'.json')).read_text())['status']=='thinking':break
                else:raise AssertionError('Recovery did not start')
            else:
                child.send('Fix total() in calculate.py: it should return the sum of the input prices, including empty and negative inputs. Read AGENTS.md and the source/tests. Share a concise plan with update_plan, use edit_file for the focused fix, and run the specified unittest command. Keep tests unchanged. Report the actual test result.\r')
            approved_edit=False;approved_shell=False;result=None
            deadline=time.monotonic()+190
            while time.monotonic()<deadline:
                text=pump()
                if 'Allow write_file?' in text:raise AssertionError('Model tried to replace the file')
                if 'Allow edit_file?' in text and 'calculate.py' in text and 'sum(prices)' in text and not approved_edit:
                    assert 'calculate.py' in text and 'sum(prices)' in text,text
                    assert (project/'calculate.py').read_text()==SOURCE
                    child.send('y');approved_edit=True
                if 'Allow shell?' in text and '$ python3 -m unittest -v' in text and not approved_shell:
                    assert '$ python3 -m unittest -v' in text,text
                    child.send('y');approved_shell=True
                for p in state.glob('*.json'):
                    s=json.loads(p.read_text())
                    if s['project']==str(project) and s['status'] in ('done','error','stopped'):result=s;break
                if result:break
            assert result,'No final result within the test deadline'
            assert result['status']=='done',result['entries'][-1]
            assert approved_edit and approved_shell
            assert (project/'test_calculate.py').read_text()==TESTS
            validation=subprocess.run(['python3','-m','unittest','-v'],cwd=project,capture_output=True,text=True,timeout=15)
            assert validation.returncode==0,validation.stderr
            assert result['work']['steps'] and result['work']['changed']==['calculate.py']
            assert any(e['passed'] and json.loads(e['identity'])[0]=='shell' for e in result['work']['evidence'])
            child.send('/quit\r');deadline=time.monotonic()+12
            while time.monotonic()<deadline:
                try:pump()
                except pexpect.EOF:break
            else:raise AssertionError('Terminal did not exit')
            child.close();assert child.exitstatus==0
            evidence={'session':result['id'],'model':result['model'],'model_turns':result['work']['model_requests'],'reproduced_without_model':reproduced_without_model,'tools':result['tools'],'input_tokens':result['input_tokens'],'output_tokens':result['output_tokens'],'plan':result['work']['steps'],'approved_edit':approved_edit,'approved_test_command':approved_shell,'tests_unchanged':True,'independent_test_exit':validation.returncode,'test_output':validation.stderr,'source':(project/'calculate.py').read_text(),'exit':child.exitstatus}
            (qa/('agent-recovery-live.json' if args.recover else 'agent-edit-live.json')).write_text(json.dumps(evidence,ensure_ascii=False,indent=2))
            print(json.dumps(evidence,ensure_ascii=False,indent=2))
        finally:
            if child.isalive():
                child.send('\x03')
                deadline=time.monotonic()+4
                while time.monotonic()<deadline:
                    try:pump()
                    except pexpect.EOF:break
                if child.isalive():child.terminate(force=True)

if __name__=='__main__':main()
