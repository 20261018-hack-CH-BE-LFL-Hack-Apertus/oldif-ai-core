import json
import uuid
from .errors import ServiceError
from .json_data import strict_json
from .tool_schema import valid_arguments

PREFIX, SUFFIX = "<|tools_prefix|>", "<|tools_suffix|>"


def malformed():
    return ServiceError(502, "invalid_model_output", "The model did not generate a valid requested response.")


def parse_generated(text, request):
    text = strip_termination(text)
    if "<|inner_prefix|>" in text or "<|inner_suffix|>" in text:
        raise malformed()
    if PREFIX not in text and SUFFIX not in text:
        if request.tool_choice == "required" or not text.strip():
            raise malformed()
        if request.json_mode:
            validate_json_output(text)
        return {"role": "assistant", "content": text}, "stop"
    if request.tool_choice == "none" or text.count(PREFIX) != 1 or text.count(SUFFIX) != 1:
        raise malformed()
    if text.index(PREFIX) > text.index(SUFFIX):
        raise malformed()
    before, block = text.split(PREFIX, 1)
    payload, after = block.split(SUFFIX, 1)
    calls = parse_calls(payload, request.tools)
    content = "\n".join(part.strip() for part in (before, after) if part.strip())
    return {"role": "assistant", "content": content or None, "tool_calls": calls}, "tool_calls"


def strip_termination(text):
    text = text.strip()
    endings = ("</s>", "<|assistant_end|>")
    while any(text.endswith(ending) for ending in endings):
        text = next(text[:-len(ending)].rstrip() for ending in endings if text.endswith(ending))
    return text


def validate_json_output(text):
    try:
        if not isinstance(strict_json(text), dict):
            raise ValueError()
    except ValueError:
        raise malformed() from None


def parse_calls(payload, tools):
    try:
        data = strict_json(payload)
    except ValueError:
        raise malformed() from None
    if not isinstance(data, list) or not 1 <= len(data) <= 16:
        raise malformed()
    schemas = {tool["function"]["name"]: tool["function"]["parameters"] for tool in tools}
    result = []
    for item in data:
        if not isinstance(item, dict) or len(item) != 1:
            raise malformed()
        name, arguments = next(iter(item.items()))
        if name not in schemas or not valid_arguments(arguments, schemas[name]):
            raise malformed()
        result.append({"id": "call_" + uuid.uuid4().hex, "type": "function",
                       "function": {"name": name, "arguments": json.dumps(arguments, ensure_ascii=False)}})
    return result
