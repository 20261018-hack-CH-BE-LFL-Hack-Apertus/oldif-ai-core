from dataclasses import dataclass
import math
from .errors import invalid
from .json_data import strict_json
from .tool_schema import validate_tools


@dataclass(frozen=True)
class CompletionRequest:
    messages: list
    tools: list
    tool_choice: str
    max_tokens: int
    temperature: float
    json_mode: bool


def validate_history(messages):
    if not isinstance(messages, list) or not 1 <= len(messages) <= 64:
        raise invalid("Provide between one and 64 text messages.")
    normalized, pending = [], set()
    for index, message in enumerate(messages):
        if not isinstance(message, dict) or set(message) - {"role", "content", "tool_calls", "tool_call_id", "name"}:
            raise invalid()
        role = message.get("role")
        if not isinstance(role, str) or role not in {"system", "user", "assistant", "tool"} or (role == "system" and index != 0):
            raise invalid("The supplied message role or order is unsupported.")
        item = dict(message)
        if role == "assistant" and item.get("content") is None and item.get("tool_calls"):
            item["content"] = ""
        if not isinstance(item.get("content"), str):
            raise invalid("Only string message content is accepted; media and URLs are not loaded.")
        pending = validate_history_calls(item, pending)
        normalized.append(item)
    if pending:
        raise invalid("Every historical tool call must have a corresponding tool result.")
    return normalized


def validate_history_calls(message, pending):
    calls = message.get("tool_calls", [])
    if message["role"] == "tool":
        identifier = message.get("tool_call_id")
        if calls or not isinstance(identifier, str) or identifier not in pending:
            raise invalid("A tool result must match a preceding assistant tool call.")
        return pending - {identifier}
    if "tool_call_id" in message:
        raise invalid()
    if pending:
        raise invalid("Tool results must immediately follow their assistant calls.")
    if calls and message["role"] != "assistant":
        raise invalid()
    if not isinstance(calls, list) or len(calls) > 16:
        raise invalid()
    identifiers = set()
    for call in calls:
        if not isinstance(call, dict) or set(call) != {"id", "type", "function"} or call["type"] != "function":
            raise invalid()
        if not isinstance(call["id"], str) or not call["id"] or call["id"] in identifiers:
            raise invalid()
        validate_history_function(call["function"])
        identifiers.add(call["id"])
    return identifiers


def validate_history_function(function):
    if not isinstance(function, dict) or set(function) != {"name", "arguments"}:
        raise invalid()
    if not isinstance(function["name"], str) or not isinstance(function["arguments"], str):
        raise invalid()
    try:
        if not isinstance(strict_json(function["arguments"]), dict):
            raise ValueError()
    except ValueError:
        raise invalid("Historical tool arguments must be complete JSON objects.") from None


def parse_request(body, served_model, output_limit):
    fields = {"model", "messages", "tools", "tool_choice", "max_tokens", "temperature", "stream", "response_format"}
    if not isinstance(body, dict) or set(body) - fields or body.get("model") != served_model:
        raise invalid("Use the served model and supported request fields.")
    if body.get("stream", False) is not False:
        raise invalid("Streaming is not supported by this bounded endpoint.")
    tools = body.get("tools", [])
    validate_tools(tools)
    choice = body.get("tool_choice", "auto" if tools else "none")
    if not isinstance(choice, str) or choice not in {"none", "auto", "required"} or (choice == "required" and not tools):
        raise invalid("Invalid tool choice.")
    maximum, temperature = body.get("max_tokens", output_limit), body.get("temperature", 0)
    if type(maximum) is not int or maximum < 1:
        raise invalid("max_tokens must be a positive integer.")
    if type(temperature) not in {int, float} or not math.isfinite(temperature) or not 0 <= temperature <= 2:
        raise invalid("temperature must be between zero and two.")
    response_format = body.get("response_format")
    if response_format not in (None, {"type": "json_object"}) or (response_format and tools):
        raise invalid("Only text or standalone json_object response format is supported.")
    return CompletionRequest(validate_history(body.get("messages")), tools, choice,
                             min(maximum, output_limit), temperature, bool(response_format))
