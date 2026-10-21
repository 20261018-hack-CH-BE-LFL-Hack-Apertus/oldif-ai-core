import unittest
from unittest.mock import patch
from src.domain.errors import ServiceError
from src.infrastructure.safe_logging import RedactedFormatter
from test_contract import request
from test_prefill import backend


LOGGER = "src.infrastructure.model"


class GenerationMetricsTests(unittest.TestCase):
    def test_completed_generation_logs_only_allowlisted_metrics_after_long_duration(self):
        marker = "private-output-marker"
        model = backend(marker)
        parsed = request(messages=[{"role": "user", "content": "private-input-marker"}])
        with patch("time.perf_counter", side_effect=[10.0, 251.123]), self.assertLogs(LOGGER, "INFO") as logs:
            generated = model.generate(parsed)
        self.assertEqual(generated["text"], marker)
        self.assertEqual(len(logs.records), 1)
        self.assertEqual(logs.records[0].msg, {
            "event": "model_generation_completed", "elapsed_seconds": 241.123,
            "prompt_tokens": 7, "completion_tokens": 3,
            "truncated": False, "finish_state": "stopped"})
        rendered = RedactedFormatter((model.config.api_key, model.config.hf_token)).format(logs.records[0])
        for forbidden in (marker, "private-input-marker", model.config.api_key, model.config.hf_token,
                          "Authorization", "arguments", "Traceback"):
            self.assertNotIn(forbidden, rendered)

    def test_token_limit_is_distinct_from_a_normal_stop(self):
        model = backend("generated fixture", [11, 12])
        with patch("time.perf_counter", side_effect=[1.0, 2.0]), self.assertLogs(LOGGER, "INFO") as logs:
            generated = model.generate(request(max_tokens=2))
        self.assertTrue(generated["truncated"])
        self.assertEqual(logs.records[0].msg["finish_state"], "token_limit")
        self.assertEqual(logs.records[0].msg["completion_tokens"], 2)

    def test_failed_generation_cannot_emit_completed_metrics_or_exception_content(self):
        model = backend("unused")
        model.model.generate.side_effect = RuntimeError("private-exception-marker")
        with self.assertLogs(LOGGER, "ERROR") as logs, self.assertRaises(ServiceError):
            model.generate(request())
        self.assertEqual(len(logs.records), 1)
        self.assertEqual(logs.records[0].msg, {"event": "model_generation_failed", "error_type": "RuntimeError"})
        self.assertNotIn("private-exception-marker", str(logs.output))
        self.assertIsNone(logs.records[0].exc_info)


if __name__ == "__main__":
    unittest.main()
