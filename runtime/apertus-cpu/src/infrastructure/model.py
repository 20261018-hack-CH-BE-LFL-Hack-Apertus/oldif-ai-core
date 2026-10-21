"""Publisher-pinned Transformers adapter; model weights and generated text stay real."""
import copy
import logging
import time
from ..domain.errors import ServiceError
from ..domain.outputs import PREFIX

LOG = logging.getLogger(__name__)


class ApertusModel:
    def __init__(self, config):
        self.config, self.ready, self.failed = config, False, False
        self.processor, self.model, self.torch = None, None, None

    def load(self):
        try:
            self.load_publisher_model()
            self.ready = True
            LOG.info("model_ready")
        except Exception as error:
            self.failed = True
            LOG.error({"event": "model_load_failed", "error_type": type(error).__name__})

    def load_publisher_model(self):
        import torch
        from transformers import AutoModelForMultimodalLM, AutoProcessor
        from transformers.utils import logging as transformer_logging
        transformer_logging.set_verbosity_error()
        transformer_logging.disable_progress_bar()
        torch.set_num_threads(self.config.threads)
        torch.set_num_interop_threads(1)
        options = {"revision": self.config.revision, "token": self.config.hf_token,
                   "cache_dir": self.config.cache_dir, "trust_remote_code": False}
        self.processor = AutoProcessor.from_pretrained(self.config.model_id, **options)
        self.model = AutoModelForMultimodalLM.from_pretrained(
            self.config.model_id, dtype=self.config.dtype, **options).eval()
        if self.model.device.type != "cpu":
            raise ValueError("The reviewed runtime requires CPU model placement.")
        self.torch = torch

    def generate(self, request):
        started = time.perf_counter()
        try:
            generated = self.generate_checked(request)
            log_completion(started, generated)
            return generated
        except ServiceError:
            raise
        except Exception as error:
            LOG.error({"event": "model_generation_failed", "error_type": type(error).__name__})
            raise ServiceError(502, "generation_failed", "The model could not complete this request.") from None

    def generate_checked(self, request):
        messages, tools = prepare_prompt(request)
        prefix = generation_prefix(request)
        if prefix:
            messages.append({"role": "assistant", "content": prefix})
        inputs = self.processor.apply_chat_template(
            messages, tools=tools, enable_thinking=False, add_generation_prompt=not bool(prefix),
            continue_final_message=bool(prefix),
            tokenize=True, return_dict=True, return_tensors="pt").to(self.model.device)
        prompt_tokens = inputs["input_ids"].shape[-1]
        if prompt_tokens + request.max_tokens > self.config.context_tokens:
            raise ServiceError(400, "context_limit", "The request exceeds the configured context limit.")
        options = {"max_new_tokens": request.max_tokens, "do_sample": request.temperature > 0,
                   "use_cache": True}
        if request.temperature > 0:
            options["temperature"] = request.temperature
        with self.torch.inference_mode():
            output = self.model.generate(**inputs, **options)
        tokens = output[0, prompt_tokens:]
        text = prefix + self.processor.decode(tokens, skip_special_tokens=False)
        return {"text": text, "prompt_tokens": prompt_tokens, "completion_tokens": len(tokens),
                "truncated": reached_limit(tokens, self.model.generation_config.eos_token_id, request.max_tokens)}


def log_completion(started, generated):
    LOG.info({"event": "model_generation_completed", "elapsed_seconds": round(time.perf_counter() - started, 3),
              "prompt_tokens": generated["prompt_tokens"], "completion_tokens": generated["completion_tokens"],
              "truncated": generated["truncated"],
              "finish_state": "token_limit" if generated["truncated"] else "stopped"})


def generation_prefix(request):
    # Adjacent assistant messages merge in the publisher template; preserve existing histories.
    if request.messages[-1]["role"] == "assistant":
        return ""
    if request.tool_choice == "required":
        return PREFIX
    return "{" if request.json_mode else ""


def reached_limit(tokens, eos_ids, maximum):
    endings = {eos_ids} if isinstance(eos_ids, int) else set(eos_ids or [])
    ended = bool(len(tokens)) and int(tokens[-1]) in endings
    return len(tokens) >= maximum and not ended


def prepare_prompt(request):
    messages, tools = copy.deepcopy(request.messages), copy.deepcopy(request.tools)
    if request.tool_choice == "none":
        tools = []
    for tool in tools:
        tool["function"].setdefault("description", "")
    directive = ""
    if request.tool_choice == "required":
        directive = "Call at least one of the supplied tools before answering. Generate the tool call in the native tool-call format."
    if request.json_mode:
        directive = "Return exactly one complete JSON object, without markdown or explanatory text."
    if directive:
        if messages[0]["role"] == "system":
            messages[0]["content"] += "\n\n" + directive
        else:
            messages.insert(0, {"role": "system", "content": directive})
    return messages, tools
