"""Single environment boundary for the private Apertus CPU endpoint."""
from dataclasses import dataclass, field
import os
import re

MODEL_ID = "swiss-ai/Apertus-v1.5-8B"
MODEL_REVISION = "ef4bbc54d6fe658bc782280ff9f3b53a10a9758f"
SERVED_MODEL = "apertus-v1-5-8b"


@dataclass(frozen=True)
class Config:
    api_key: str = field(repr=False)
    hf_token: str = field(repr=False)
    model_id: str = MODEL_ID
    revision: str = MODEL_REVISION
    served_model: str = SERVED_MODEL
    host: str = "0.0.0.0"
    port: int = 8000
    threads: int = 14
    context_tokens: int = 8192
    output_tokens: int = 256
    request_bytes: int = 262144
    cache_dir: str = "/models/huggingface/hub"

    @classmethod
    def from_env(cls):
        values = {"api_key": os.environ.get("VLLM_API_KEY", ""),
                  "hf_token": os.environ.get("HF_TOKEN", "")}
        text_fields = {"model_id": "APERTUS_MODEL_ID", "revision": "APERTUS_MODEL_REVISION",
                       "served_model": "APERTUS_SERVED_MODEL", "host": "APERTUS_HOST",
                       "cache_dir": "APERTUS_MODEL_CACHE_DIR"}
        number_fields = {"port": "APERTUS_PORT", "threads": "APERTUS_CPU_THREADS",
                         "context_tokens": "APERTUS_MAX_CONTEXT_TOKENS",
                         "output_tokens": "APERTUS_MAX_NEW_TOKENS",
                         "request_bytes": "APERTUS_MAX_REQUEST_BYTES"}
        for name, variable in text_fields.items():
            values[name] = os.environ.get(variable, getattr(cls, name))
        for name, variable in number_fields.items():
            values[name] = int(os.environ.get(variable, getattr(cls, name)))
        config = cls(**values)
        config.validate()
        return config

    def validate(self):
        if len(self.api_key) < 32 or not self.hf_token:
            raise ValueError("Configured runtime credentials are required.")
        if (self.model_id, self.revision, self.served_model) != (MODEL_ID, MODEL_REVISION, SERVED_MODEL):
            raise ValueError("Only the reviewed Apertus model and immutable revision are supported.")
        if not re.fullmatch(r"[a-f0-9]{40}", self.revision):
            raise ValueError("An immutable model revision is required.")
        if not (1 <= self.port <= 65535 and 1 <= self.threads <= 64):
            raise ValueError("Invalid runtime port or thread count.")
        if not (256 <= self.context_tokens <= 8192 and 1 <= self.output_tokens <= 256):
            raise ValueError("Context or output limit exceeds the reviewed CPU profile.")
        if not (1024 <= self.request_bytes <= 1048576):
            raise ValueError("Invalid request byte limit.")
