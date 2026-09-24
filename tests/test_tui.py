import asyncio
import json
import os
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from relay.engine import Engine
from relay.providers import MiniMaxProvider
from relay.server import make_server
from relay.tui_backend import LocalBackend, RemoteBackend, make_backend

try:
    from textual.widgets import Input, OptionList, RichLog, Select, Static, TabbedContent
    from relay.tui import NewRun, RelayTUI, clean
    HAS_TUI = True
except ModuleNotFoundError:
    HAS_TUI = False


@unittest.skipUnless(HAS_TUI, "Install requirements-tui.txt to test the terminal UI")
class TerminalTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.engine = Engine(self.root / 'runs')
        self.backend = LocalBackend(self.engine)
        self.env = patch.dict(os.environ, {'TERM':'xterm-256color','COLORTERM':'truecolor'})
        self.env.start()
        os.environ.pop('NO_COLOR',None)

    async def asyncTearDown(self):
        if self.engine.active:
            self.engine.cancel(self.engine.active)
            for _ in range(50):
                if not self.engine.active: break
                await asyncio.sleep(.05)
        self.engine.close()
        self.tmp.cleanup()
        self.env.stop()

    def app(self, **kwargs):
        return RelayTUI(self.backend, self.root/'exports', default_provider='demo', **kwargs)

    async def completed(self, app, pilot, previous_id=None):
        for _ in range(50):
            await pilot.pause(.03)
            await app.refresh_data()
            if app.current and app.current['id'] != previous_id and app.current['status'] in {'passed','failed','cancelled','error','limit','unverified'}:
                return app.current
        self.fail('Run did not complete')

    async def test_keyboard_launch_inspect_and_export(self):
        app=self.app()
        async with app.run_test(size=(140,45)) as pilot:
            await pilot.press('n');await pilot.pause()
            self.assertIsInstance(app.screen,NewRun)
            await pilot.click('#launch')
            run=await self.completed(app,pilot)
            self.assertEqual(run['status'],'passed')
            await pilot.press('3');await pilot.pause()
            self.assertEqual(app.query_one('#pages',TabbedContent).active,'files-tab')
            self.assertEqual(json.loads(run['files']['service.json'])['timeout_seconds'],30)
            self.assertTrue(app.query_one('#file-content',RichLog).lines)
            await pilot.press('2');await pilot.pause()
            self.assertEqual(app.query_one('#pages',TabbedContent).active,'checks-tab')
            self.assertTrue(run['checks'][0]['passed'])
            await pilot.press('e');await pilot.pause()
            self.assertTrue(app.last_export.is_file())
            export=json.loads(app.last_export.read_text())
            self.assertEqual(export['id'],run['id'])
            self.assertEqual(export['files'],run['files'])
            self.assertEqual(app.last_export.stat().st_mode & 0o777,0o600)

    async def test_failed_task_is_visible_as_failed(self):
        app=self.app()
        async with app.run_test(size=(120,40)) as pilot:
            await pilot.press('n');await pilot.pause()
            app.screen.query_one('#task-choice',Select).value='failure-lab'
            await pilot.click('#launch')
            run=await self.completed(app,pilot)
            self.assertEqual(run['status'],'failed')
            await pilot.press('2');await pilot.pause()
            text='\n'.join(line.text for line in app.query_one('#checks',RichLog).lines)
            self.assertIn('2 / 3',text)
            self.assertIn('605',text)

    async def test_checkpoint_branch_from_ui_preserves_parent(self):
        parent_id=self.engine.start({'task_id':'config-repair'},background=False)
        before=self.engine.get(parent_id)
        app=self.app()
        async with app.run_test(size=(120,40)) as pilot:
            await pilot.pause();await pilot.press('b');await pilot.pause()
            app.screen.query_one('#checkpoint',Select).value=2
            await pilot.click('#start-branch')
            run=await self.completed(app,pilot,previous_id=parent_id)
            self.assertEqual(run['status'],'passed')
            self.assertEqual(run['parent_id'],parent_id)
            self.assertEqual(run['parent_checkpoint'],2)
            self.assertEqual(self.engine.get(parent_id)['files'],before['files'])

    async def test_invalid_limit_keeps_form_and_does_not_run(self):
        app=self.app()
        async with app.run_test(size=(120,45)) as pilot:
            await pilot.press('n');await pilot.pause()
            app.screen.query_one('#turn-limit',Input).value='0'
            await pilot.click('#launch');await pilot.pause()
            self.assertIsInstance(app.screen,NewRun)
            self.assertIn('between',str(app.screen.query_one('#form-error',Static).render()))
            self.assertEqual(self.engine.list_runs(),[])

    async def test_compact_layout_and_resize_keep_actions_reachable(self):
        app=self.app()
        async with app.run_test(size=(80,24)) as pilot:
            await pilot.pause()
            self.assertFalse(app.query_one('#sidebar').display)
            await pilot.press('n');await pilot.pause()
            launch=app.screen.query_one('#launch')
            self.assertLess(launch.region.bottom,25)
            await pilot.click('#launch')
            run=await self.completed(app,pilot)
            self.assertEqual(run['status'],'passed')
            await pilot.press('h');await pilot.pause()
            self.assertTrue(app.query_one('#sidebar').display)
            await pilot.press('enter');await pilot.pause()
            self.assertFalse(app.query_one('#sidebar').display)
            await pilot.resize_terminal(140,45);await pilot.pause()
            self.assertTrue(app.query_one('#sidebar').display)
            self.assertTrue(app.query_one('#companion-box').display)
            await pilot.press('c');await pilot.pause()
            self.assertFalse(app.query_one('#companion-box').display)

    async def test_stop_inflight_from_tui_prevents_late_write(self):
        entered,release=threading.Event(),threading.Event()
        def transport(*_):
            entered.set();release.wait(5)
            return {'content':[{'type':'tool_use','id':'late','name':'write_file','input':{'path':'late.txt','content':'late'}}], 'stop_reason':'tool_use'}
        provider=MiniMaxProvider('test-model',transport=transport)
        self.engine.provider_factory=lambda *_:provider
        run_id=self.engine.start({'provider':'minimax'})
        self.assertTrue(await asyncio.to_thread(entered.wait,1))
        app=self.app()
        async with app.run_test(size=(120,40)) as pilot:
            await pilot.pause();await pilot.press('s')
            run=await self.completed(app,pilot)
            self.assertEqual(run['id'],run_id)
            self.assertEqual(run['status'],'cancelled')
            self.assertNotIn('late.txt',run['files'])
        release.set()

    async def test_custom_manifest_can_be_loaded_and_run(self):
        task_path=self.root/'custom.json'
        task_path.write_text(json.dumps({'title':'Custom proof','prompt':'Write ok to result.txt.',
                                        'checks':[{'name':'Exact output','type':'text_equals','path':'result.txt','expected':'ok'}]}))
        responses=iter([
            {'content':[{'type':'tool_use','id':'one','name':'write_file','input':{'path':'result.txt','content':'ok'}}],'stop_reason':'tool_use'},
            {'content':[{'type':'text','text':'Done.'}],'stop_reason':'end_turn'}])
        self.engine.provider_factory=lambda *_:MiniMaxProvider('mock',transport=lambda *_:next(responses))
        original=self.backend.config
        self.backend.config=lambda:{**original(),'minimax_ready':True}
        app=self.app()
        async with app.run_test(size=(140,50)) as pilot:
            await pilot.press('n');await pilot.pause()
            app.screen.query_one('#task-choice',Select).value='custom'
            app.screen.query_one('#provider-choice',Select).value='minimax'
            app.screen.query_one('#manifest-path',Input).value=str(task_path)
            await pilot.pause();await pilot.click('#launch')
            run=await self.completed(app,pilot)
            self.assertEqual(run['status'],'passed')
            self.assertEqual(run['files']['result.txt'],'ok')

    async def test_terminal_control_codes_and_markup_are_not_interpreted(self):
        self.assertEqual(clean('\x1b[31mred\x1b[0m [bold]literal[/bold]\x07'),'red [bold]literal[/bold]')
        self.assertNotIn('\x9b',clean('x\x9by'))


class RemoteTests(unittest.TestCase):
    def test_dashboard_connection_shares_runs_and_enforces_workspace(self):
        with tempfile.TemporaryDirectory() as tmp:
            engine=Engine(Path(tmp)/'runs')
            server=make_server(engine,0)
            thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
            url=f'http://127.0.0.1:{server.server_port}'
            try:
                backend=RemoteBackend(url,expected_workspace=engine.root)
                run_id=backend.start({'provider':'demo','task_id':'config-repair'})
                for _ in range(100):
                    result=backend.get(run_id)
                    if result['status']=='passed':break
                    import time;time.sleep(.01)
                self.assertEqual(result['status'],'passed')
                self.assertEqual(backend.list_runs()[0]['id'],run_id)
                with self.assertRaises(ValueError):RemoteBackend(url,expected_workspace=Path(tmp)/'other')
                with self.assertRaises(ValueError):RemoteBackend('https://example.com')
            finally:
                server.shutdown();server.server_close();thread.join();engine.close()


if __name__=='__main__': unittest.main()
