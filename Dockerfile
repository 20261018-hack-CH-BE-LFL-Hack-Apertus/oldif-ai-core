FROM debian:bookworm-slim AS diarization
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
      amd64) ARCH=x86_64; SHA=f396057150f1b774935c7414fd32ebc04195a795a34120d78d8c9ecefa1b7507 ;; \
      arm64) ARCH=aarch64; SHA=ac50694d4729f9eae390a9851d38f51269d9af9829b535325539090a3b95c787 ;; \
      *) exit 1 ;; esac && \
    curl --fail --location --retry 2 --max-time 120 "https://github.com/NVIDIA/NeMo-Speech.cpp/releases/download/v0.2.0/nemo-speech-0.2.0-linux-${ARCH}-cpu.tar.gz" -o /tmp/runtime.tar.gz && \
    printf '%s  %s\n' "$SHA" /tmp/runtime.tar.gz | sha256sum --check --strict && \
    mkdir -p /opt/nemo && tar -xzf /tmp/runtime.tar.gz --strip-components=1 -C /opt/nemo && rm /tmp/runtime.tar.gz
RUN curl --fail --location --retry 2 --max-time 180 "https://huggingface.co/nvidia/Nemotron-3-Diarization/resolve/f667ed73aee57d40cc39428eb768b4fd87a0a29e/Nemotron-3-Diarization.q8_0.gguf" -o /opt/nemo/model.gguf && \
    printf '%s  %s\n' 08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1 /opt/nemo/model.gguf | sha256sum --check --strict
FROM rust:1.99.1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY proto ./proto
RUN cargo build --locked --release
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 app
WORKDIR /app
COPY --from=diarization /opt/nemo /opt/nemo
COPY evidence/diarization/model-license.txt /opt/nemo/share/licenses/model/OpenMDW-1.1.txt
ENV AI_DIARIZATION_BIN=/opt/nemo/bin/nemo-speech AI_DIARIZATION_MODEL=/opt/nemo/model.gguf
COPY --from=build /app/target/release/oldif-ai-core /usr/local/bin/oldif-ai-core
RUN mkdir -p /app/data/runs && chown -R app:app /app
USER app
EXPOSE 8082 50052
ENTRYPOINT ["oldif-ai-core"]
