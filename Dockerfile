FROM rust:1.99.1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY proto ./proto
RUN cargo build --locked --release
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* && useradd -r -u 10001 app
WORKDIR /app
COPY --from=build /app/target/release/oldif-ai-core /usr/local/bin/oldif-ai-core
RUN mkdir -p /app/data/runs && chown -R app:app /app
USER app
EXPOSE 8082 50052
ENTRYPOINT ["oldif-ai-core"]
