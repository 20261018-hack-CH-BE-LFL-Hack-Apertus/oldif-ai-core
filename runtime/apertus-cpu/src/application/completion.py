import threading
import time
import uuid
from ..domain.errors import ServiceError
from ..domain.outputs import parse_generated
from ..domain.requests import parse_request


class CompletionService:
    def __init__(self, config, backend):
        self.config, self.backend = config, backend
        self.generation_lock = threading.BoundedSemaphore(1)

    def complete(self, body):
        request = parse_request(body, self.config.served_model, self.config.output_tokens)
        if not self.backend.ready:
            raise ServiceError(503, "model_unavailable", "The model is not ready.")
        if not self.generation_lock.acquire(blocking=False):
            raise ServiceError(429, "model_busy", "The model is processing another request.")
        try:
            generated = self.backend.generate(request)
            message, reason = parse_generated(generated["text"], request)
            if generated["truncated"]:
                reason = "length" if reason == "stop" else reason
            return self.response(message, reason, generated)
        finally:
            self.generation_lock.release()

    def response(self, message, reason, generated):
        prompt, completion = generated["prompt_tokens"], generated["completion_tokens"]
        return {"id": "chatcmpl-" + uuid.uuid4().hex, "object": "chat.completion",
                "created": int(time.time()), "model": self.config.served_model,
                "choices": [{"index": 0, "message": message, "finish_reason": reason}],
                "usage": {"prompt_tokens": prompt, "completion_tokens": completion,
                          "total_tokens": prompt + completion}}
