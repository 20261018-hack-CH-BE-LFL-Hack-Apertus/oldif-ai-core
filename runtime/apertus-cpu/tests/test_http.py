import http.client
import json
import secrets
import threading
import unittest
from src.api.server import PrivateServer
from src.application.completion import CompletionService
from src.config import Config, SERVED_MODEL
from test_contract import BackendFixture, body


class HttpTests(unittest.TestCase):
    def setUp(self):
        self.config = Config(secrets.token_urlsafe(32), "fixture-token", request_bytes=1024)
        self.backend = BackendFixture()
        service = CompletionService(self.config, self.backend)
        self.server = PrivateServer(("127.0.0.1", 0), self.config, service)
        self.worker = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.worker.start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join(timeout=2)

    def call(self, method, path, data=None, auth=True, extra_headers=None):
        connection = http.client.HTTPConnection(*self.server.server_address, timeout=3)
        headers = {"Content-Type": "application/json"}
        if auth:
            headers["Authorization"] = "Bearer " + self.config.api_key
        headers.update(extra_headers or {})
        payload = json.dumps(data) if data is not None else None
        connection.request(method, path, payload, headers)
        response = connection.getresponse()
        value = json.loads(response.read())
        result = response.status, dict(response.getheaders()), value
        connection.close()
        return result

    def test_model_endpoint_requires_auth_and_preserves_identity(self):
        self.assertEqual(self.call("GET", "/v1/models", auth=False)[0], 401)
        status, headers, value = self.call("GET", "/v1/models")
        self.assertEqual(status, 200)
        self.assertEqual(value["data"][0]["id"], SERVED_MODEL)
        self.assertEqual(headers["Cache-Control"], "no-store")

    def test_health_reports_actual_readiness(self):
        self.backend.ready = False
        self.assertEqual(self.call("GET", "/health", auth=False)[0], 503)
        self.backend.ready = True
        self.assertEqual(self.call("GET", "/health", auth=False)[0], 200)

    def test_completion_and_invalid_content_do_not_reach_backend(self):
        self.assertEqual(self.call("POST", "/v1/chat/completions", body(), auth=False)[0], 401)
        self.assertEqual(self.call("POST", "/v1/chat/completions", body(), extra_headers={"Content-Type": "text/plain"})[0], 415)
        self.assertEqual(self.call("POST", "/v1/chat/completions", body(stream=True))[0], 400)
        self.assertEqual(self.backend.calls, 0)
        status, _, result = self.call("POST", "/v1/chat/completions", body())
        self.assertEqual(status, 200)
        self.assertEqual(result["choices"][0]["message"]["content"], self.backend.text)

    def test_request_byte_limit_and_unknown_path(self):
        self.assertEqual(self.call("POST", "/v1/chat/completions", body(messages=[{"role": "user", "content": "x" * 2000}]))[0], 413)
        self.assertEqual(self.call("GET", "/missing")[0], 404)
        self.assertEqual(self.backend.calls, 0)


if __name__ == "__main__":
    unittest.main()
