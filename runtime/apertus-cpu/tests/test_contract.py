import logging
import secrets
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch
from src.config import Config, SERVED_MODEL
from src.domain.errors import ServiceError
from src.domain.json_data import strict_json
from src.domain.outputs import parse_generated
from src.domain.requests import parse_request
from src.infrastructure.model import ApertusModel, prepare_prompt, reached_limit
from src.infrastructure.safe_logging import RedactedFormatter, redact_fields
from src.application.completion import CompletionService

TOOLS = [{"type": "function", "function": {"name": "read_source", "description": "Read a test record.",
          "parameters": {"type": "object", "properties": {"source_id": {"type": "string", "enum": ["fixture-1"]}},
                         "required": ["source_id"], "additionalProperties": False}}}]


def body(**changes):
    return {"model": SERVED_MODEL, "messages": [{"role": "user", "content": "Read the test source."}], **changes}


def request(**changes):
    return parse_request(body(**changes), SERVED_MODEL, 256)


class BackendFixture:
    ready, failed = True, False

    def __init__(self, text="This is a synthetic HTTP test response.", truncated=False):
        self.text, self.truncated, self.calls = text, truncated, 0

    def generate(self, _request):
        self.calls += 1
        return {"text": self.text, "prompt_tokens": 10, "completion_tokens": 7, "truncated": self.truncated}


class RequestTests(unittest.TestCase):
    def test_unknown_model_stream_media_and_extra_fields_rejected(self):
        variants = [{"model": "apertus-v1-5-70b"}, {"stream": True}, {"unknown": 1},
                    {"messages": [{"role": "user", "content": [{"type": "image", "url": "untrusted"}]}]}]
        for variant in variants:
            with self.subTest(variant=variant), self.assertRaises(ServiceError):
                request(**variant)

    def test_bounded_output_and_temperature(self):
        self.assertEqual(request(max_tokens=5000).max_tokens, 256)
        for change in ({"max_tokens": True}, {"max_tokens": 0}, {"temperature": float("nan")},
                       {"temperature": 3}, {"tool_choice": {"type": "function"}}):
            with self.subTest(change=change), self.assertRaises(ServiceError):
                request(**change)

    def test_tool_history_requires_matching_results(self):
        call = {"id": "test-call", "type": "function", "function": {"name": "read_source", "arguments": '{"source_id":"fixture-1"}'}}
        history = [{"role": "user", "content": "Read."}, {"role": "assistant", "content": None, "tool_calls": [call]},
                   {"role": "tool", "tool_call_id": "test-call", "content": "Synthetic passage."}]
        parsed = request(messages=history, tools=TOOLS)
        self.assertEqual(parsed.messages[1]["content"], "")
        self.assertIsNone(history[1]["content"])
        with self.assertRaises(ServiceError):
            request(messages=history[:-1])
        history[-1]["tool_call_id"] = "other-call"
        with self.assertRaises(ServiceError):
            request(messages=history)

    def test_schema_rejects_unimplemented_or_duplicate_tools(self):
        for tools in (TOOLS + TOOLS, [{"type": "function", "function": {"name": "read_source", "parameters": {"$ref": "untrusted"}}}]):
            with self.assertRaises(ServiceError):
                request(tools=tools)

    def test_complete_json_and_no_duplicate_members(self):
        for text in ('{"model":1,"model":2}', '{"value":NaN}', '{"value":1} trailing'):
            with self.assertRaises(ValueError):
                strict_json(text)

    def test_malformed_role_and_tool_identity_fail_as_invalid_requests(self):
        for messages in ([{"role": [], "content": "fixture"}],
                         [{"role": "tool", "tool_call_id": [], "content": "fixture"}],
                         [{"role": "user", "tool_call_id": "unused", "content": "fixture"}]):
            with self.assertRaises(ServiceError) as error:
                request(messages=messages)
            self.assertEqual(error.exception.status, 400)


class GenerationTests(unittest.TestCase):
    def test_cpu_profile_requires_float32(self):
        config = Config(secrets.token_urlsafe(32), "fixture-token")
        self.assertEqual(config.dtype, "float32")
        for dtype in ("auto", "bfloat16", "float16"):
            with self.subTest(dtype=dtype), self.assertRaises(ValueError):
                Config(config.api_key, config.hf_token, dtype=dtype).validate()

    def test_loader_uses_float32_without_changing_pinned_source(self):
        config = Config(secrets.token_urlsafe(32), "fixture-token")
        publisher = SimpleNamespace(AutoProcessor=Mock(), AutoModelForMultimodalLM=Mock())
        publisher.AutoModelForMultimodalLM.from_pretrained.return_value.eval.return_value.device.type = "cpu"
        modules = {"torch": Mock(), "transformers": publisher,
                   "transformers.utils": SimpleNamespace(logging=Mock())}
        with patch.dict("sys.modules", modules):
            ApertusModel(config).load_publisher_model()
        publisher.AutoModelForMultimodalLM.from_pretrained.assert_called_once_with(
            config.model_id, dtype="float32", revision=config.revision,
            token=config.hf_token, cache_dir=config.cache_dir, trust_remote_code=False)

    def test_loader_rejects_non_cpu_result(self):
        config = Config(secrets.token_urlsafe(32), "fixture-token")
        publisher = SimpleNamespace(AutoProcessor=Mock(), AutoModelForMultimodalLM=Mock())
        modules = {"torch": Mock(), "transformers": publisher,
                   "transformers.utils": SimpleNamespace(logging=Mock())}
        for device in ("cuda", "meta"):
            publisher.AutoModelForMultimodalLM.from_pretrained.return_value.eval.return_value.device.type = device
            with self.subTest(device=device), patch.dict("sys.modules", modules), self.assertRaises(ValueError):
                ApertusModel(config).load_publisher_model()

    def test_real_native_tool_shape_is_preserved(self):
        output = '<|tools_prefix|>[{"read_source":{"source_id":"fixture-1"}}]<|tools_suffix|>'
        message, reason = parse_generated(output, request(tools=TOOLS, tool_choice="required"))
        self.assertEqual(reason, "tool_calls")
        self.assertEqual(message["tool_calls"][0]["function"]["name"], "read_source")
        self.assertEqual(strict_json(message["tool_calls"][0]["function"]["arguments"]), {"source_id": "fixture-1"})

    def test_missing_truncated_unknown_and_invalid_tool_outputs_fail(self):
        outputs = ['No tool was generated.', '<|tools_prefix|>[{"read_source":{"source_id":"fixture-1"}}]',
                   '<|tools_prefix|>[{"other":{"source_id":"fixture-1"}}]<|tools_suffix|>',
                   '<|tools_prefix|>[{"read_source":{"source_id":"other"}}]<|tools_suffix|>',
                   '<|tools_prefix|>[{"read_source":{"source_id":1}}]<|tools_suffix|>',
                   '<|tools_prefix|>[{"read_source":{"source_id":"fixture-1","extra":"x"}}]<|tools_suffix|>',
                   '<|tools_prefix|>[{"read_source":{"source_id":"fixture-1","source_id":"fixture-1"}}]<|tools_suffix|>',
                   '<|tools_suffix|><|tools_prefix|>[]']
        for output in outputs:
            with self.subTest(output=output), self.assertRaises(ServiceError):
                parse_generated(output, request(tools=TOOLS, tool_choice="required"))

    def test_json_mode_and_hidden_reasoning_fail_closed(self):
        parsed = request(response_format={"type": "json_object"})
        self.assertEqual(parse_generated('{"answer":"fixture"}', parsed)[1], "stop")
        for output in ('```json\n{"answer":"fixture"}\n```', '{"answer":', '[]', '<|inner_prefix|>thought<|inner_suffix|>{}'):
            with self.assertRaises(ServiceError):
                parse_generated(output, parsed)

    def test_prompt_keeps_source_and_disables_tools_when_requested(self):
        original = request(tools=TOOLS, tool_choice="required")
        messages, _tools = prepare_prompt(original)
        self.assertEqual(messages[0]["role"], "system")
        self.assertEqual(messages[-1], original.messages[-1])
        self.assertEqual(prepare_prompt(request(tools=TOOLS, tool_choice="none"))[1], [])

    def test_actual_eos_at_limit_is_not_truncation(self):
        self.assertFalse(reached_limit([11, 72], [2, 68, 72], 2))
        self.assertTrue(reached_limit([11, 12], [2, 68, 72], 2))
        self.assertFalse(reached_limit([11], [2, 68, 72], 2))

    def test_service_usage_model_and_single_generation(self):
        config = Config(secrets.token_urlsafe(32), "fixture-token")
        backend = BackendFixture(truncated=True)
        service = CompletionService(config, backend)
        answer = service.complete(body())
        self.assertEqual(answer["model"], SERVED_MODEL)
        self.assertEqual(answer["usage"]["total_tokens"], 17)
        self.assertEqual(answer["choices"][0]["finish_reason"], "length")
        service.generation_lock.acquire()
        try:
            with self.assertRaises(ServiceError) as error:
                service.complete(body())
            self.assertEqual(error.exception.status, 429)
        finally:
            service.generation_lock.release()
        self.assertEqual(backend.calls, 1)

    def test_log_redaction_and_config_repr(self):
        key = secrets.token_urlsafe(32)
        config = Config(key, "fixture-token")
        self.assertNotIn(key, repr(config))
        record = logging.LogRecord("test", 20, "", 0, "credential %s Bearer unrelated", (key,), None)
        rendered = RedactedFormatter((key,)).format(record)
        self.assertNotIn(key, rendered)
        self.assertNotIn("unrelated", rendered)
        self.assertEqual(redact_fields({"nested": {"password": "private"}}), {"nested": {"password": "[REDACTED]"}})

    def test_load_failure_logs_class_without_exception_content(self):
        marker = secrets.token_urlsafe(32)
        backend = ApertusModel(Config(marker, "fixture-token"))
        backend.load_publisher_model = Mock(side_effect=RuntimeError(marker))
        with self.assertLogs("src.infrastructure.model", level="ERROR") as logs:
            backend.load()
        self.assertFalse(backend.ready)
        self.assertTrue(backend.failed)
        self.assertIn("RuntimeError", str(logs.output))
        self.assertNotIn(marker, str(logs.output))


if __name__ == "__main__":
    unittest.main()
