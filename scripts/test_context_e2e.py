"""PTY context/skill/prompt discovery and persistence. Offline; no API calls."""
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
    with tempfile.TemporaryDirectory(prefix='aster-context-') as temp:
        project=Path(temp)/'project';project.mkdir();state=Path(temp)/'state'
        skill=project/'.agents/skills/fixture';skill.mkdir(parents=True)
        (skill/'SKILL.md').write_text('---\nname: fixture\ndescription: Inspect a fixture precisely\n---\nSKILL-CONTENT-486\nRead the attached file and check the result.\n')
        prompts=project/'.aster/prompts';prompts.mkdir(parents=True)
        (prompts/'review.md').write_text('---\ndescription: Review a selected change\n---\nInspect $ARGUMENTS then summarize the evidence.\n')
        (project/'source.txt').write_text('skip first\nFILE-CONTENT-486\nskip last\n')
        child=pexpect.spawn(str(ROOT/'aster'),['--project',str(project),'--state-dir',str(state),'--demo','--no-live2d','--permissions','allow'],env=dict(os.environ,TERM='xterm-256color'),encoding='utf-8',codec_errors='replace',dimensions=(42,132))
        class Screen(pyte.Screen):
            def write_process_input(self,data):child.send(data)
        screen=Screen(132,42);stream=pyte.Stream(screen)
        def pump():
            try:stream.feed(child.read_nonblocking(65536,timeout=.1))
            except pexpect.TIMEOUT:pass
            return screen_text(screen)
        def wait(predicate,timeout=15):
            end=time.monotonic()+timeout
            while time.monotonic()<end:
                text=pump()
                if predicate(text):return text
            raise AssertionError('Timed out: '+pump())
        def saved():
            paths=list(state.glob('*.json'))
            return json.loads(paths[0].read_text()) if paths else {}
        try:
            wait(lambda t:'aster' in t)
            child.send('/skills\r');wait(lambda t:'Skills beside' in t and 'fixture' in t)
            child.send('fixture\r');wait(lambda t:'/skill fixture' in t and 'Find:' not in t)
            assert not saved().get('messages')
            child.send('inspect @source.txt:2\r')
            wait(lambda _:saved().get('status')=='done')
            first=saved()
            assert first['work']['skills']==['fixture']
            assert first['work']['context_files']==['source.txt:2']
            assert 'SKILL-CONTENT-486' in first['messages'][0]['content']
            assert '2: FILE-CONTENT-486' in first['messages'][0]['content']
            assert 'skip first' not in first['messages'][0]['content']
            assert 'SKILL-CONTENT-486' not in first['entries'][0]['text']
            child.send('/context\r');wait(lambda t:'Context beside' in t and 'Attached files this turn' in t)
            child.send('\x1b');wait(lambda t:'Context beside' not in t)
            child.send('/prompts\r');wait(lambda t:'Reusable prompts' in t and 'review' in t)
            child.send('\r');wait(lambda t:'/prompt review' in t and 'Find:' not in t)
            child.send('this change\r')
            wait(lambda _:saved().get('status')=='done' and any(m['content']=='Inspect this change then summarize the evidence.' for m in saved().get('messages',[]) if m['role']=='user'))
            child.send('/reload\r');wait(lambda t:'Project resources refreshed' in t)
            child.setwinsize(24,80);screen.resize(24,80);wait(lambda t:'Project resources' in t)
            child.send('\x1b');wait(lambda t:'Project resources' not in t)
            child.send('/quit\r')
            end=time.monotonic()+12
            while time.monotonic()<end:
                try:pump()
                except pexpect.EOF:break
            else:raise AssertionError('Quit timed out')
            child.close();assert child.exitstatus==0
            evidence={'skill_picker_prepares_without_api':True,'explicit_skill_loaded':True,'selected_file_lines_attached':True,'readable_user_transcript':True,'context_inspection':True,'prompt_expansion':True,'reload':True,'resize':True,'exit':0,'api_calls':0}
            (ROOT/'.aster/qa/context-e2e.json').write_text(json.dumps(evidence,indent=2));print(json.dumps(evidence,indent=2))
        finally:stop_child(child,stream)
if __name__=='__main__':main()
