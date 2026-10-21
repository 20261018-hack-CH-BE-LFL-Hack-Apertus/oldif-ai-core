FROM ghcr.io/swiss-ai/vllm_apertus_1.5_release@sha256:d85533f251c8272218ae2e519510836376e5a335b44f2204f193018e37753bc6
USER root
WORKDIR /opt/apertus-cpu
COPY src ./src
COPY runtime-lock.json ./runtime-lock.json
RUN python3 -c "from transformers import AutoModelForMultimodalLM, AutoProcessor; import torch; assert torch.__version__.startswith('2.11.')" && \
    mkdir -p /models/huggingface/hub && chown -R 10001:10001 /models /opt/apertus-cpu
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1
USER 10001:10001
EXPOSE 8000
ENTRYPOINT ["python3", "-m", "src.api.main"]
CMD []
