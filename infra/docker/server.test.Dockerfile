FROM rust:1.94-slim-bookworm
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY . .

CMD ["cargo", "test", "-p", "codex-manager-server", "--", "--nocapture"]
