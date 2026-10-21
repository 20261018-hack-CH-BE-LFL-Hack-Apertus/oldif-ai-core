import re
from .errors import invalid


def validate_schema(schema):
    if not isinstance(schema, dict) or schema.get("type") != "object":
        raise invalid("Tool parameters must use an object schema.")
    allowed = {"type", "properties", "required", "additionalProperties", "description"}
    if set(schema) - allowed or schema.get("additionalProperties", False) is not False:
        raise invalid("Unsupported tool schema keywords.")
    properties, required = schema.get("properties", {}), schema.get("required", [])
    if not isinstance(properties, dict) or not isinstance(required, list):
        raise invalid()
    if not all(isinstance(key, str) and key in properties for key in required):
        raise invalid()
    for specification in properties.values():
        if not isinstance(specification, dict) or specification.get("type") != "string":
            raise invalid("This bounded endpoint supports string tool arguments only.")
        if set(specification) - {"type", "description", "enum"}:
            raise invalid("Unsupported tool property keywords.")
        choices = specification.get("enum")
        if choices is not None and (not isinstance(choices, list) or not choices or
                                    not all(isinstance(value, str) for value in choices)):
            raise invalid()


def validate_tools(tools):
    if not isinstance(tools, list) or len(tools) > 16:
        raise invalid()
    names = set()
    for tool in tools:
        if not isinstance(tool, dict) or set(tool) != {"type", "function"} or tool["type"] != "function":
            raise invalid()
        function = tool["function"]
        if not isinstance(function, dict) or set(function) - {"name", "description", "parameters", "strict"}:
            raise invalid()
        name = function.get("name", "")
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,63}", name) or name in names:
            raise invalid("Tool names must be unique valid identifiers.")
        if not isinstance(function.get("description", ""), str):
            raise invalid()
        validate_schema(function.get("parameters"))
        names.add(name)


def valid_arguments(arguments, schema):
    if not isinstance(arguments, dict):
        return False
    properties = schema.get("properties", {})
    if set(arguments) - set(properties) or not set(schema.get("required", [])).issubset(arguments):
        return False
    return all(isinstance(value, str) and
               ("enum" not in properties[key] or value in properties[key]["enum"])
               for key, value in arguments.items())
