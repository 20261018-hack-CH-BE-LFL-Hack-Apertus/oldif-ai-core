import json
import logging
import re

SECRET_FIELDS = {"password", "token", "authorization", "api_key", "secret", "private_key",
                 "mnemonic", "seed", "cookie", "set-cookie", "x-api-key", "proxy-authorization"}


def redact_fields(value):
    if isinstance(value, dict):
        return {key: "[REDACTED]" if str(key).lower() in SECRET_FIELDS else redact_fields(item)
                for key, item in value.items()}
    if isinstance(value, list):
        return [redact_fields(item) for item in value]
    return value


class RedactedFormatter(logging.Formatter):
    def __init__(self, secrets):
        super().__init__()
        self.secrets = tuple(value for value in secrets if value)

    def format(self, record):
        message = json.dumps(redact_fields(record.msg)) if isinstance(record.msg, dict) else record.getMessage()
        for value in self.secrets:
            message = message.replace(value, "[REDACTED]")
        message = re.sub(r"(?i)(bearer\s+)[^\s,;]+", r"\1[REDACTED]", message)
        return json.dumps({"level": record.levelname, "event": message})


def configure_logging(config):
    handler = logging.StreamHandler()
    handler.setFormatter(RedactedFormatter((config.api_key, config.hf_token)))
    logging.basicConfig(level=logging.INFO, handlers=[handler], force=True)
