# syntax=docker/dockerfile:1.7

FROM docker:28-cli AS docker-cli

FROM node:22-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV HOME=/root
ENV CODEX_HOME=/root/.codex
ENV CODEX_MANAGED_SKILLS_ROOT=/root/.agents/skills
ENV CODEX_LEGACY_SKILLS_ROOT=/root/.codex/skills

SHELL ["/bin/bash", "-lc"]

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    bubblewrap \
    ca-certificates \
    curl \
    docker.io \
    git \
    jq \
    less \
    openssh-client \
    procps \
    python3 \
    python3-venv \
    ripgrep \
    socat \
    tini \
    unzip \
    xz-utils \
  && rm -rf /var/lib/apt/lists/*

RUN ln -sf /usr/bin/bwrap /usr/local/bin/bubblewrap

RUN npm install -g @openai/codex \
  && npm cache clean --force

COPY --from=docker-cli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=docker-cli /usr/local/libexec/docker/cli-plugins/docker-compose /usr/local/libexec/docker/cli-plugins/docker-compose

COPY sh/new/scripts/container-bootstrap.sh /usr/local/bin/container-bootstrap
COPY sh/new/skills /opt/codex-skills

RUN chmod +x /usr/local/bin/container-bootstrap

WORKDIR /workspace

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/container-bootstrap"]
CMD ["tail", "-f", "/dev/null"]
