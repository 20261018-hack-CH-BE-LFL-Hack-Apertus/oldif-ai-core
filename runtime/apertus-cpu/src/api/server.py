import hmac
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import logging
from ..domain.errors import ServiceError, invalid
from ..domain.json_data import strict_json

LOG = logging.getLogger(__name__)


class PrivateServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 8

    def __init__(self, address, config, service):
        self.config, self.service = config, service
        super().__init__(address, Handler)

    def handle_error(self, request, client_address):
        LOG.error("http_handler_failed")


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def setup(self):
        super().setup()
        self.connection.settimeout(30)

    def log_message(self, _format, *_args):
        return

    def version_string(self):
        return "Oldif"

    def send_error(self, code, message=None, explain=None):
        self.respond(code, error_body("http_error", "The HTTP request is not supported."))

    def authorized(self):
        values = self.headers.get_all("Authorization", [])
        expected = "Bearer " + self.server.config.api_key
        return len(values) == 1 and hmac.compare_digest(values[0].encode(), expected.encode())

    def do_GET(self):
        if self.path == "/health":
            backend = self.server.service.backend
            status = "ready" if backend.ready else "failed" if backend.failed else "loading"
            return self.respond(200 if backend.ready else 503, {"status": status})
        if not self.authorized():
            return self.respond(401, error_body("unauthorized", "A valid bearer credential is required."))
        if self.path != "/v1/models":
            return self.respond(404, error_body("not_found", "Route not found."))
        model = self.server.config.served_model
        self.respond(200, {"object": "list", "data": [{"id": model, "object": "model", "owned_by": "swiss-ai"}]})

    def do_POST(self):
        if not self.authorized():
            return self.respond(401, error_body("unauthorized", "A valid bearer credential is required."))
        if self.path != "/v1/chat/completions":
            return self.respond(404, error_body("not_found", "Route not found."))
        try:
            body = self.read_body()
            self.respond(200, self.server.service.complete(body))
        except ServiceError as error:
            self.respond(error.status, error_body(error.code, error.message))
        except Exception:
            LOG.error("request_processing_failed")
            self.respond(500, error_body("internal_error", "The request could not be completed."))

    def read_body(self):
        lengths = self.headers.get_all("Content-Length", [])
        if self.headers.get("Transfer-Encoding") or len(lengths) != 1:
            raise invalid("Send one Content-Length header without Transfer-Encoding.")
        if self.headers.get_content_type() != "application/json":
            raise ServiceError(415, "content_type", "Use application/json.")
        try:
            length = int(lengths[0])
        except ValueError:
            raise invalid() from None
        if not 0 < length <= self.server.config.request_bytes:
            raise ServiceError(413, "request_size", "The request exceeds its byte limit.")
        try:
            raw = self.rfile.read(length)
            if len(raw) != length:
                raise ValueError()
            return strict_json(raw.decode("utf-8"))
        except (ValueError, UnicodeError):
            raise invalid("Provide a complete valid JSON body.") from None

    def respond(self, status, value):
        payload = json.dumps(value, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        self.wfile.write(payload)


def error_body(code, message):
    return {"error": {"code": code, "message": message, "type": "invalid_request_error"}}
