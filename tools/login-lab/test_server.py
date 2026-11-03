"""Exercise the proxy boundary without contacting login providers."""
import http.server
import json
import pathlib
import tempfile
import threading
import unittest
import urllib.error
import urllib.request

from server import Handler


class Upstream(http.server.BaseHTTPRequestHandler):
    calls = []

    def log_message(self, *_):
        pass

    def do_POST(self):
        self.calls.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        self.send_response(200)
        self.end_headers()
        self.wfile.write(json.dumps({'ok': True, 'data': {'cookie': 'private-cookie', 'caller_credential': 'twc1_private', 'state': 'confirmed'}, 'meta': {'request_id': 'test-request'}}).encode())


class BoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.upstream = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Upstream)
        self.proxy = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.host = '127.0.0.1:' + str(self.proxy.server_port)
        self.proxy.allowed_hosts = {self.host}
        self.proxy.backend = 'http://127.0.0.1:' + str(self.upstream.server_port)
        self.proxy.events = pathlib.Path(self.temp.name) / 'events.jsonl'
        self.proxy.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        Upstream.calls = []
        for server in [self.upstream, self.proxy]:
            threading.Thread(target=server.serve_forever, daemon=True).start()

    def tearDown(self):
        for server in [self.proxy, self.upstream]:
            server.shutdown()
            server.server_close()
        self.temp.cleanup()

    def request(self, body, headers=None, path='/api/v1/auth/import'):
        request = urllib.request.Request('http://' + self.host + path, json.dumps(body).encode(), headers={'Content-Type': 'application/json', **(headers or {})})
        try:
            response = self.client.open(request)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, response.read().decode()

    def test_import_scrubs_response_and_never_logs_credentials(self):
        status, body = self.request({'account': 'lab-test', 'platform': 'netease', 'credential': {'kind': 'cookie', 'value': 'private-cookie'}})
        self.assertEqual(status, 200)
        self.assertEqual(len(Upstream.calls), 1)
        self.assertNotIn('private-cookie', body)
        self.assertNotIn('twc1_private', body)
        log = self.proxy.events.read_text()
        self.assertNotIn('private-cookie', log)
        self.assertNotIn('credential', log)
        self.assertIn('test-request', log)

    def test_rejects_account_and_caller_mode_before_upstream(self):
        for body in [{'account': 'production'}, {'account': 'lab-test', 'credential_mode': 'caller'}, []]:
            self.assertEqual(self.request(body)[0], 400)
        self.assertEqual(Upstream.calls, [])

    def test_rejects_foreign_origin_host_and_cross_site(self):
        for headers in [{'Origin': 'http://foreign.test'}, {'Host': 'foreign.test'}, {'Sec-Fetch-Site': 'cross-site'}]:
            self.assertEqual(self.request({'account': 'lab-test'}, headers)[0], 403)
        self.assertEqual(Upstream.calls, [])

    def test_rejects_unrelated_routes(self):
        self.assertEqual(self.request({}, path='/api/v1/download')[0], 404)
        self.assertEqual(Upstream.calls, [])


if __name__ == '__main__':
    unittest.main()
