FROM ghcr.io/swiss-ai/vllm_apertus_1.5_release@sha256:d85533f251c8272218ae2e519510836376e5a335b44f2204f193018e37753bc6
USER root
WORKDIR /opt/apertus-cpu
COPY src ./src
COPY runtime-lock.json ./runtime-lock.json
RUN groupadd --gid 10001 apertus && \
    useradd --uid 10001 --gid 10001 --system --no-create-home \
        --home-dir /models/huggingface --shell /usr/sbin/nologin apertus && \
    mkdir -p /models/huggingface/hub && chown -R 10001:10001 /models /opt/apertus-cpu
ENV PYTHONUNBUFFERED=1 PYTHONDONTWRITEBYTECODE=1 \
    HOME=/models/huggingface USER=apertus LOGNAME=apertus
USER 10001:10001
RUN python3 -c "import os, pwd, getpass; assert os.getuid() == 10001 and os.getgid() == 10001; account = pwd.getpwuid(os.getuid()); assert account.pw_name == getpass.getuser() == 'apertus'; assert account.pw_dir == os.environ['HOME']; from transformers import AutoModelForMultimodalLM, AutoProcessor; import torch; assert torch.__version__.startswith('2.11.')"
EXPOSE 8000
ENTRYPOINT ["python3", "-m", "src.api.main"]
CMD []
