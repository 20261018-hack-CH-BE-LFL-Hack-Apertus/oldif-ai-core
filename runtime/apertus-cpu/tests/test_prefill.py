import copy
import secrets
import unittest
from contextlib import nullcontext
from types import SimpleNamespace
from unittest.mock import Mock
from src.application.completion import CompletionService
from src.config import Config
from src.domain.errors import ServiceError
from src.domain.json_data import strict_json
from src.domain.outputs import parse_generated
from src.infrastructure.model import ApertusModel
from test_contract import TOOLS, body, request


class Inputs(dict):
    def to(self, _device):
        return self


class Output:
    def __init__(self, tokens):
        self.tokens, self.selection = tokens, None

    def __getitem__(self, selection):
        self.selection = selection
        return self.tokens


def backend(suffix, tokens=None):
    config = Config(secrets.token_urlsafe(32), "fixture-token")
    model = ApertusModel(config)
    model.ready = True
    model.processor = Mock()
    model.processor.apply_chat_template.return_value = Inputs(input_ids=SimpleNamespace(shape=(1, 7)))
    model.processor.decode.return_value = suffix
    model.model = SimpleNamespace(device="cpu", generation_config=SimpleNamespace(eos_token_id=[2, 68, 72]),
                                  generate=Mock(return_value=Output(tokens or [11, 12, 68])))
    model.torch = SimpleNamespace(inference_mode=nullcontext)
    return model


def tool_history():
    call = {"id": "read-fixture", "type": "function", "function": {
        "name": "read_source", "arguments": '{"source_id":"fixture-1"}'}}
    return [{"role": "user", "content": "Read the fixture."},
            {"role": "assistant", "content": None, "tool_calls": [call]},
            {"role": "tool", "tool_call_id": "read-fixture", "content": "Fixture evidence."}]


class PrefillTests(unittest.TestCase):
    def assert_template(self, model, prefix):
        call = model.processor.apply_chat_template.call_args
        self.assertEqual(call.args[0][-1], {"role": "assistant", "content": prefix})
        self.assertFalse(call.kwargs["add_generation_prompt"])
        self.assertTrue(call.kwargs["continue_final_message"])
        self.assertFalse(call.kwargs["enable_thinking"])
        self.assertEqual(model.processor.decode.call_args.kwargs, {"skip_special_tokens": False})

    def test_json_prefix_restores_only_structure_with_accurate_usage(self):
        model = backend('"answer":"fixture"}<|assistant_end|>')
        response = CompletionService(model.config, model).complete(body(response_format={"type": "json_object"}))
        self.assert_template(model, "{")
        self.assertEqual(response["choices"][0]["message"]["content"], '{"answer":"fixture"}')
        self.assertEqual(response["usage"], {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10})
        self.assertEqual(model.model.generate.return_value.selection, (0, slice(7, None)))
        self.assertEqual(model.model.generate.call_args.kwargs["max_new_tokens"], 256)

    def test_required_native_prefix_preserves_generated_arguments_and_eos(self):
        suffix = '[{"read_source":{"source_id":"fixture-1"}}]<|tools_suffix|>'
        model = backend(suffix, [11, 12, 72])
        response = CompletionService(model.config, model).complete(body(tools=TOOLS, tool_choice="required", max_tokens=3))
        self.assert_template(model, "<|tools_prefix|>")
        choice = response["choices"][0]
        self.assertEqual(choice["finish_reason"], "tool_calls")
        function = choice["message"]["tool_calls"][0]["function"]
        self.assertEqual(function["name"], "read_source")
        self.assertEqual(strict_json(function["arguments"]), {"source_id": "fixture-1"})
        self.assertEqual(response["usage"]["completion_tokens"], 3)
        model.processor.decode.assert_called_once_with([11, 12, 72], skip_special_tokens=False)

    def test_post_tool_json_prefill_preserves_caller_history(self):
        parsed = request(messages=tool_history(), response_format={"type": "json_object"})
        original = copy.deepcopy(parsed.messages)
        model = backend('"answer":"fixture"}')
        generated = model.generate(parsed)
        self.assert_template(model, "{")
        self.assertEqual(parsed.messages, original)
        templated = model.processor.apply_chat_template.call_args.args[0]
        self.assertEqual(templated[1:-1], original)
        self.assertEqual(parse_generated(generated["text"], parsed)[0]["content"], '{"answer":"fixture"}')

    def test_text_auto_none_and_terminal_assistant_keep_existing_generation(self):
        terminal = [{"role": "user", "content": "Fixture"}, {"role": "assistant", "content": "Existing reply"}]
        variants = [{}, {"tools": TOOLS, "tool_choice": "auto"}, {"tools": TOOLS, "tool_choice": "none"},
                    {"messages": terminal, "response_format": {"type": "json_object"}},
                    {"messages": terminal, "tools": TOOLS, "tool_choice": "required"}]
        for change in variants:
            with self.subTest(change=change):
                parsed = request(**change)
                model = backend("generated fixture")
                self.assertEqual(model.generate(parsed)["text"], "generated fixture")
                call = model.processor.apply_chat_template.call_args
                self.assertEqual(call.args[0][-1], parsed.messages[-1])
                self.assertTrue(call.kwargs["add_generation_prompt"])
                self.assertFalse(call.kwargs.get("continue_final_message", False))

    def test_json_prefill_never_repairs_missing_or_duplicated_delimiters(self):
        for suffix in ('"answer":', '{"answer":"fixture"}', '"answer":"fixture"'):
            with self.subTest(suffix=suffix), self.assertRaises(ServiceError):
                model = backend(suffix)
                CompletionService(model.config, model).complete(body(response_format={"type": "json_object"}))

    def test_native_prefill_does_not_supply_endings_names_or_arguments(self):
        suffixes = ['[{"read_source":{"source_id":"fixture-1"}}]',
                    '[{"unknown":{"source_id":"fixture-1"}}]<|tools_suffix|>',
                    '[{"read_source":{"source_id":"other"}}]<|tools_suffix|>']
        for suffix in suffixes:
            with self.subTest(suffix=suffix), self.assertRaises(ServiceError):
                model = backend(suffix)
                CompletionService(model.config, model).complete(body(tools=TOOLS, tool_choice="required"))

    def test_context_limit_includes_prefill_and_prevents_generation(self):
        model = backend('"answer":"fixture"}')
        model.processor.apply_chat_template.return_value["input_ids"].shape = (1, 8190)
        with self.assertRaises(ServiceError) as error:
            model.generate(request(response_format={"type": "json_object"}, max_tokens=3))
        self.assertEqual(error.exception.code, "context_limit")
        model.model.generate.assert_not_called()


if __name__ == "__main__":
    unittest.main()
