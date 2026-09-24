"""Real pseudo-terminal test. Live API usage requires the explicit --live flag."""
import argparse
import json
import os
import sys
import tempfile
import time
from pathlib import Path

import pexpect
import pyte

PROJECT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT))
from relay.tui_backend import RemoteBackend


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--live',action='store_true',help='Launch ONE bounded MiniMax config task through the TUI')
    parser.add_argument('--connect',default='http://127.0.0.1:8787')
    args=parser.parse_args()
    qa=PROJECT/'.relay/qa';qa.mkdir(parents=True,exist_ok=True)
    env=dict(os.environ,TERM='xterm-256color',COLORTERM='truecolor')
    env.pop('NO_COLOR',None)
    with tempfile.TemporaryDirectory(prefix='relay-pty-') as tmp:
        export_dir=qa/('live-exports' if args.live else 'demo-exports')
        command=[str(PROJECT/'.venv/bin/python'),'-m','relay']
        backend=None
        if args.live:
            backend=RemoteBackend(args.connect)
            before={r['id'] for r in backend.list_runs()}
            command+=['tui','--connect',args.connect,'--provider','minimax']
        else:
            command+=['--data-dir',str(Path(tmp)/'runs'),'tui','--provider','demo']
        command+=['--export-dir',str(export_dir)]
        child=pexpect.spawn(command[0],command[1:],cwd=str(PROJECT),env=env,
                            encoding='utf-8',codec_errors='replace',dimensions=(42,132),timeout=5)
        screen=pyte.Screen(132,42);stream=pyte.Stream(screen)
        transcript=[]

        def pump(seconds=.15):
            until=time.monotonic()+seconds
            while time.monotonic()<until:
                try:
                    chunk=child.read_nonblocking(65536,timeout=min(.1,max(.01,until-time.monotonic())))
                    transcript.append(chunk);stream.feed(chunk)
                except pexpect.TIMEOUT:pass
                except pexpect.EOF:break
            return '\n'.join(screen.display)

        def expect_screen(needle,timeout=8):
            until=time.monotonic()+timeout
            while time.monotonic()<until:
                text=pump()
                if needle in text:return text
                if not child.isalive():break
            (qa/'terminal-failure.txt').write_text('\n'.join(screen.display))
            raise AssertionError(f'Terminal did not display {needle!r}; see terminal-failure.txt')

        try:
            expect_screen('TERMINAL WORKBENCH')
            child.send('n');expect_screen('Give your agent a task.')
            # Focus begins at the first select. Shift-Tab reaches the last button.
            child.send('\x1b[Z');pump(.2);child.send('\r')
            run=None
            if args.live:
                deadline=time.monotonic()+140
                while time.monotonic()<deadline:
                    pump(.35)
                    fresh=[r for r in backend.list_runs() if r['id'] not in before]
                    if len(fresh)>1:raise AssertionError('More than one live run was launched')
                    if fresh and fresh[0]['status'] in {'passed','failed','error','cancelled','limit'}:
                        run=backend.get(fresh[0]['id']);break
                if not run:raise AssertionError('Live run did not finish within its deadline')
                if run['status']!='passed':raise AssertionError('Live task did not pass: '+run['status'])
            expect_screen('PASSED',timeout=12)
            child.send('3');expect_screen('timeout_seconds');expect_screen('30')
            child.send('2');expect_screen('CHECKS PASSED')
            child.send('e');expect_screen('Exported')
            exports=sorted(export_dir.glob('*.json'),key=lambda p:p.stat().st_mtime)
            exported=json.loads(exports[-1].read_text())
            assert exported['status']=='passed'
            assert json.loads(exported['files']['service.json'])=={
                'service':'relay-worker','region':'ap-southeast','retries':3,'timeout_seconds':30,'debug':False}
            run=run or exported
            (qa/('terminal-live.txt' if args.live else 'terminal-demo.txt')).write_text('\n'.join(screen.display))
            child.setwinsize(24,80);screen.resize(24,80);pump(.5)
            child.send('1');expect_screen('Timeline')
            child.setwinsize(42,132);screen.resize(42,132);pump(.5)
            child.send('q')
            child.expect(pexpect.EOF,timeout=6);child.close()
            assert child.exitstatus==0,(child.exitstatus,child.signalstatus)
            evidence={'mode':'live' if args.live else 'demo','run_id':run['id'],'status':run['status'],
                      'steps':run['steps'],'tool_calls':run['tool_calls'],'input_tokens':run['input_tokens'],
                      'output_tokens':run['output_tokens'],'export':str(exports[-1]),'terminal_exit':child.exitstatus,
                      'sizes':[[132,42],[80,24]],'actions':['launch via keyboard','verify result','inspect file',
                      'inspect checks','export trace','resize','quit cleanly']}
            (qa/('terminal-live.json' if args.live else 'terminal-demo.json')).write_text(json.dumps(evidence,indent=2))
            print(json.dumps(evidence,indent=2))
        finally:
            if child.isalive():child.terminate(force=True)


if __name__=='__main__':main()
