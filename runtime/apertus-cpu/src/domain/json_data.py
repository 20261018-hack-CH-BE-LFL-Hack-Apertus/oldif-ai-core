import json


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("Duplicate object member.")
        result[key] = value
    return result


def reject_constant(_value):
    raise ValueError("Non-finite JSON constant.")


def strict_json(text):
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=reject_constant)
