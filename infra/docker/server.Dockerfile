FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app

COPY . .

RUN cargo build --release -p codex-manager-server

FROM debian:bookworm-slim AS runner
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/codex-manager-server /usr/local/bin/codex-manager-server

ENV CMGR_SERVER_BIND_ADDR=0.0.0.0
ENV CMGR_SERVER_DATA_PORT=8080
ENV CMGR_SERVER_ADMIN_PORT=8081

EXPOSE 8080 8081

CMD ["codex-manager-server"]

