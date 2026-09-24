import json
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

from relay.engine import Engine
from relay.providers import DemoProvider, MiniMaxProvider, ProviderError
from relay.server import make_server
from relay.tasks import validate_task
from relay.tools import calculate, execute, safe_path


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.engine = Engine(self.root / 'runs')

    def tearDown(self):
        self.engine.close()
        self.tmp.cleanup()

    def test_data_directory_cannot_be_opened_by_two_engines(self):
        with self.assertRaises(ValueError): Engine(self.root / 'runs')

    def test_source_preservation_check_rejects_appended_data(self):
        from relay.tools import verify
        file=self.root/'source.txt';file.write_text('original\nextra\n')
        results=verify(self.root,[{'type':'text_equals','path':'source.txt','expected':'original\n'}])
        self.assertFalse(results[0]['passed'])

    def run_task(self, **config):
        return self.engine.get(self.engine.start(config, background=False))

    def test_success_means_artifact_checks_pass(self):
        run = self.run_task(task_id='revenue-audit')
        self.assertEqual(run['status'], 'passed')
        self.assertEqual(json.loads(run['files']['report.json']),
                         {'paid_orders': 4, 'revenue': 525, 'top_product': 'Monitor'})
        self.assertEqual(run['tool_calls'], 4)
        self.assertTrue(all(c['passed'] for c in run['checks']))
        self.assertEqual([e['seq'] for e in run['events']], list(range(1, len(run['events'])+1)))

    def test_completion_message_does_not_override_failed_checks(self):
        run = self.run_task(task_id='failure-lab')
        self.assertTrue(run['final'])
        self.assertEqual(run['status'], 'failed')
        self.assertFalse(run['checks'][1]['passed'])
        self.assertEqual(run['checks'][1]['actual']['revenue'], 605)

    def test_branch_restores_checkpoint_and_preserves_parent(self):
        parent = self.run_task(task_id='config-repair')
        # Deliberately change the parent's current workspace. The checkpoint is authoritative.
        file = self.root / 'runs' / parent['id'] / 'workspace' / 'service.json'
        file.write_text('{}')
        branch = self.run_task(parent_id=parent['id'], checkpoint=2)
        self.assertEqual(branch['status'], 'passed')
        self.assertEqual(branch['steps'], 2)
        self.assertEqual(branch['parent_checkpoint'], 2)
        self.assertEqual(file.read_text(), '{}')

    def test_step_limit_stops_before_another_call(self):
        run = self.run_task(limits={'max_steps': 1})
        self.assertEqual(run['status'], 'limit')
        self.assertEqual(run['steps'], 1)
        self.assertNotIn('report.json', run['files'])

    def test_tool_limit_stops_further_writes(self):
        run = self.run_task(limits={'max_tools': 2})
        self.assertEqual(run['status'], 'limit')
        self.assertEqual(run['tool_calls'], 2)
        self.assertNotIn('report.json', run['files'])

    def test_path_traversal_and_symlink_escape_rejected(self):
        workspace = self.root / 'workspace'; workspace.mkdir()
        outside = self.root / 'secret'; outside.write_text('private')
        (workspace / 'link').symlink_to(outside)
        for path in ('../secret', str(outside), 'link', '.', 'a/../../secret'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                safe_path(workspace, path)

    def test_tool_registry_and_argument_validation(self):
        with self.assertRaises(ValueError): execute(self.root, 'shell', {'command':'ls'})
        with self.assertRaises(ValueError): execute(self.root, 'read_file', {'path':'x', 'other':'y'})
        with self.assertRaises(ValueError): execute(self.root, 'read_file', {'path':42})

    def test_calculator_does_not_execute_code(self):
        self.assertEqual(calculate('(2 * 80) + 75 + 240 + 50')['result'], 525)
        for expression in ('__import__("os").getcwd()', '2**1000000', '[1][0]', 'True', '1e100', '1/0'):
            with self.subTest(expression=expression), self.assertRaises((ValueError, ArithmeticError)):
                calculate(expression)

    def test_invalid_custom_task_fails_before_creation(self):
        with self.assertRaises(ValueError): validate_task({'title':'x', 'prompt':'x', 'files':{'../x':'bad'}})
        with self.assertRaises(ValueError): validate_task({'title':'x', 'prompt':'x', 'checks':[{'type':'shell','path':'x'}]})
        with self.assertRaises(ValueError): self.run_task(limits={'max_steps':True})
        self.assertEqual(self.engine.list_runs(), [])

    def fake_engine(self, transport):
        provider = MiniMaxProvider(model='test-model', transport=transport)
        self.engine.provider_factory = lambda *_: provider
        return provider

    def test_full_model_content_and_tool_results_sent_back(self):
        seen = []
        opaque = {'type':'thinking','thinking':'private fixture reasoning','signature':'opaque-signature'}
        def transport(payload, timeout):
            seen.append(payload)
            if len(seen) == 1:
                return {'content':[opaque, {'type':'tool_use','id':'one','name':'write_file','input':{'path':'result.txt','content':'hello'}}],
                        'stop_reason':'tool_use', 'usage':{'input_tokens':50,'output_tokens':100,'cache_read_input_tokens':9}}
            return {'content':[{'type':'text','text':'Done.'}], 'stop_reason':'end_turn',
                    'usage':{'input_tokens':70,'output_tokens':20}}
        self.fake_engine(transport)
        task = {'title':'Test','prompt':'Write hello to result.txt.','checks':[{'type':'contains','path':'result.txt','expected':'hello'}]}
        run = self.run_task(provider='minimax', task=task)
        self.assertEqual(run['status'], 'passed')
        self.assertEqual(seen[1]['messages'][1]['content'][0], opaque)
        self.assertEqual(seen[1]['messages'][2]['content'][0]['tool_use_id'], 'one')
        self.assertEqual(run['output_tokens'], 120)
        self.assertEqual(run['cache_read_tokens'], 9)
        self.assertNotIn('private fixture reasoning', json.dumps(run))

    def test_truncated_response_never_executes_partial_tools_or_retries(self):
        calls = []
        def transport(*args):
            calls.append(1)
            return {'content':[{'type':'tool_use','id':'a','name':'write_file','input':{'path':'oops','content':'partial'}}],
                    'stop_reason':'max_tokens','usage':{'output_tokens':64}}
        self.fake_engine(transport)
        run = self.run_task(provider='minimax')
        self.assertEqual(run['status'], 'error')
        self.assertEqual(len(calls), 1)
        self.assertNotIn('oops', run['files'])

    def test_output_allowance_is_reduced_before_request(self):
        allowances = []
        def transport(payload, timeout):
            allowances.append(payload['max_tokens'])
            return {'content':[{'type':'tool_use','id':str(len(allowances)),'name':'list_files','input':{}}],
                    'stop_reason':'tool_use','usage':{'output_tokens':80}}
        self.fake_engine(transport)
        run = self.run_task(provider='minimax', limits={'max_output_tokens':100,'total_output_tokens':170})
        self.assertEqual(allowances, [100, 90])
        self.assertEqual(run['status'], 'limit')

    def test_no_checks_produces_unverified_status(self):
        self.fake_engine(lambda *_: {'content':[{'type':'text','text':'Done.'}],'stop_reason':'end_turn'})
        run = self.run_task(provider='minimax', task={'title':'Hello','prompt':'Say hello'})
        self.assertEqual(run['status'], 'unverified')

    def test_cancel_inflight_prevents_tool_side_effects(self):
        entered, release = threading.Event(), threading.Event()
        def transport(*_):
            entered.set(); release.wait(3)
            return {'content':[{'type':'tool_use','id':'late','name':'write_file','input':{'path':'late.txt','content':'late'}}],
                    'stop_reason':'tool_use'}
        self.fake_engine(transport)
        run_id = self.engine.start({'provider':'minimax'})
        self.assertTrue(entered.wait(1))
        self.engine.cancel(run_id)
        deadline=time.monotonic()+2
        while self.engine.active and time.monotonic()<deadline: time.sleep(.02)
        release.set()
        run=self.engine.get(run_id)
        self.assertEqual(run['status'],'cancelled')
        self.assertNotIn('late.txt',run['files'])

    def test_server_rejects_cross_origin_and_does_not_serve_env(self):
        server=make_server(self.engine,0)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        base=f'http://127.0.0.1:{server.server_port}'
        local=urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with local.open(base+'/api/config') as response: config=json.load(response)
            self.assertNotIn('ANTHROPIC_AUTH_TOKEN', json.dumps(config))
            for path in ('/.env','/../.env','/relay/providers.py'):
                with self.assertRaises(urllib.error.HTTPError) as error: local.open(base+path)
                self.assertEqual(error.exception.code,404)
                error.exception.close()
            request=urllib.request.Request(base+'/api/runs',data=b'{}',headers={'Content-Type':'application/json','Origin':'https://example.com'})
            with self.assertRaises(urllib.error.HTTPError) as error: local.open(request)
            self.assertEqual(error.exception.code,403)
            error.exception.close()
        finally: server.shutdown();server.server_close();thread.join()


if __name__=='__main__': unittest.main()
