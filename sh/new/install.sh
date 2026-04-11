#!/usr/bin/env bash
set -Eeuo pipefail

RUNNING_SCRIPT_DIR="$(cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="${RUNNING_SCRIPT_DIR}"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
STATE_DIR="${RUNNING_SCRIPT_DIR}/state"
STATE_FILE="${STATE_DIR}/install-state.tsv"

DEFAULT_IMAGE_NAME="codexmanager/codex-lite:latest"
DEFAULT_CONTAINER_PREFIX="codex-"
DEFAULT_BOOTSTRAP_REPO_URL="https://github.com/3582730951/Codex-Manager-Dev.git"
DEFAULT_REPO_URL="$(git -C "${PROJECT_DIR}" remote get-url origin 2>/dev/null || printf '%s' "${DEFAULT_BOOTSTRAP_REPO_URL}")"
DEFAULT_REPO_REF="$(git -C "${PROJECT_DIR}" rev-parse --abbrev-ref HEAD 2>/dev/null || printf 'main')"
DEFAULT_DEPLOY_DIR=""
LEGACY_RUNTIME_ENV="${SCRIPT_DIR}/../.runtime.env"
BOOTSTRAP_CACHE_DIR="${XDG_CACHE_HOME:-${HOME:-/root}/.cache}/codex-manager-install"

log() {
  printf '[install] %s\n' "$*"
}

warn() {
  printf '[install][warn] %s\n' "$*" >&2
}

die() {
  printf '[install][error] %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "缺少命令: $1"
}

ensure_state_dir() {
  mkdir -p "${STATE_DIR}"
  touch "${STATE_FILE}"
}

state_get() {
  local key="$1"
  ensure_state_dir
  awk -F '\t' -v key="${key}" '$1 == key {print substr($0, index($0, FS) + 1); found=1; exit} END {if (!found) exit 1}' "${STATE_FILE}"
}

state_get_or_default() {
  local key="$1" fallback="$2"
  local value
  value="$(state_get "${key}" 2>/dev/null || true)"
  if [[ -n "${value}" ]]; then
    printf '%s' "${value}"
  else
    printf '%s' "${fallback}"
  fi
}

state_set() {
  local key="$1" value="$2" tmp_file
  ensure_state_dir
  tmp_file="$(mktemp "${STATE_DIR}/state.XXXXXX")"
  awk -F '\t' -v key="${key}" '$1 != key {print}' "${STATE_FILE}" > "${tmp_file}" || true
  printf '%s\t%s\n' "${key}" "${value}" >> "${tmp_file}"
  mv "${tmp_file}" "${STATE_FILE}"
}

state_remove() {
  local key="$1" tmp_file
  ensure_state_dir
  tmp_file="$(mktemp "${STATE_DIR}/state.XXXXXX")"
  awk -F '\t' -v key="${key}" '$1 != key {print}' "${STATE_FILE}" > "${tmp_file}" || true
  mv "${tmp_file}" "${STATE_FILE}"
}

state_remove_prefix() {
  local prefix="$1" tmp_file
  ensure_state_dir
  tmp_file="$(mktemp "${STATE_DIR}/state.XXXXXX")"
  awk -F '\t' -v prefix="${prefix}" 'index($1, prefix) != 1 {print}' "${STATE_FILE}" > "${tmp_file}" || true
  mv "${tmp_file}" "${STATE_FILE}"
}

step_key() {
  local action="$1" step="$2"
  printf 'done:%s:%s' "${action}" "${step}"
}

activate_action_context() {
  local action="$1" context="$2"
  local failed_action failed_step saved_context
  failed_action="$(state_get 'FAILED_ACTION' 2>/dev/null || true)"
  failed_step="$(state_get 'FAILED_STEP' 2>/dev/null || true)"
  saved_context="$(state_get "ACTION_CONTEXT:${action}" 2>/dev/null || true)"

  if [[ "${failed_action}" == "${action}" && -n "${failed_step}" && "${saved_context}" == "${context}" ]]; then
    log "检测到上次 ${action} 在步骤 ${failed_step} 失败，本次将从失败点继续。"
  else
    state_remove_prefix "done:${action}:"
    if [[ "${failed_action}" == "${action}" ]]; then
      state_remove 'FAILED_ACTION'
      state_remove 'FAILED_STEP'
    fi
  fi

  state_set "ACTION_CONTEXT:${action}" "${context}"
  state_set 'ACTIVE_ACTION' "${action}"
}

finish_action() {
  local action="$1"
  local failed_action
  failed_action="$(state_get 'FAILED_ACTION' 2>/dev/null || true)"
  if [[ "${failed_action}" == "${action}" ]]; then
    state_remove 'FAILED_ACTION'
    state_remove 'FAILED_STEP'
  fi
  state_set 'LAST_SUCCESS_ACTION' "${action}"
  state_set 'LAST_SUCCESS_AT' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  state_remove 'ACTIVE_ACTION'
}

run_step() {
  local action="$1" step="$2"
  shift 2

  if [[ "$(state_get "$(step_key "${action}" "${step}")" 2>/dev/null || true)" == "1" ]]; then
    log "跳过已完成步骤: ${action}/${step}"
    return 0
  fi

  state_set 'FAILED_ACTION' "${action}"
  state_set 'FAILED_STEP' "${step}"
  log "执行步骤: ${action}/${step}"
  "$@"
  state_set "$(step_key "${action}" "${step}")" "1"
}

prompt_default() {
  local prompt="$1" default_value="$2" input
  read -r -p "${prompt} [${default_value}]: " input
  if [[ -z "${input}" ]]; then
    printf '%s' "${default_value}"
  else
    printf '%s' "${input}"
  fi
}

prompt_visible_default() {
  local prompt="$1" default_value="${2:-}" input
  if [[ -n "${default_value}" ]]; then
    read -r -p "${prompt} [${default_value}]: " input
    if [[ -z "${input}" ]]; then
      printf '%s' "${default_value}"
    else
      printf '%s' "${input}"
    fi
    return 0
  fi

  read -r -p "${prompt}: " input
  printf '%s' "${input}"
}

prompt_yes_no() {
  local prompt="$1" default_answer="${2:-y}" input normalized
  if [[ "${default_answer}" == "y" ]]; then
    read -r -p "${prompt} [Y/n]: " input
    normalized="${input:-y}"
  else
    read -r -p "${prompt} [y/N]: " input
    normalized="${input:-n}"
  fi
  normalized="${normalized,,}"
  [[ "${normalized}" == "y" || "${normalized}" == "yes" ]]
}

prompt_secret_default() {
  local prompt="$1" default_value="${2:-}" input
  if [[ -n "${default_value}" ]]; then
    read -r -s -p "${prompt} [留空保持当前值]: " input
    printf '\n' >&2
    if [[ -z "${input}" ]]; then
      printf '%s' "${default_value}"
    else
      printf '%s' "${input}"
    fi
    return 0
  fi

  read -r -s -p "${prompt}: " input
  printf '\n' >&2
  printf '%s' "${input}"
}

resolve_path() {
  local target="$1"
  python3 - "$target" <<'PY'
import os
import pathlib
import sys

target = pathlib.Path(sys.argv[1]).expanduser()
print(target.resolve())
PY
}

is_windows_drive_path() {
  [[ "$1" =~ ^[A-Za-z]:[\\/].* ]]
}

windows_path_to_host_path() {
  local target="$1"
  python3 - "$target" <<'PY'
import re
import sys

raw = sys.argv[1].strip()
match = re.match(r'^([A-Za-z]):[\\/]*(.*)$', raw)
if not match:
    raise SystemExit(raw)

drive = match.group(1).lower()
rest = match.group(2).replace("\\", "/").strip("/")
if rest:
    print(f"/mnt/{drive}/{rest}")
else:
    print(f"/mnt/{drive}")
PY
}

running_inside_docker() {
  [[ -f "/.dockerenv" ]]
}

linux_path_to_host_path() {
  local container_path="$1" abs_path container_id best_source="" best_destination=""
  abs_path="$(resolve_path "${container_path}")"

  if ! running_inside_docker; then
    printf '%s' "${abs_path}"
    return 0
  fi

  container_id="${HOSTNAME:-}"
  if [[ -z "${container_id}" ]]; then
    printf '%s' "${abs_path}"
    return 0
  fi

  while IFS=$'\t' read -r source destination; do
    [[ -n "${source}" && -n "${destination}" ]] || continue
    if [[ "${abs_path}" == "${destination}" || "${abs_path}" == "${destination}/"* ]]; then
      if (( ${#destination} > ${#best_destination} )); then
        best_source="${source}"
        best_destination="${destination}"
      fi
    fi
  done < <(docker inspect -f '{{range .Mounts}}{{printf "%s\t%s\n" .Source .Destination}}{{end}}' "${container_id}" 2>/dev/null || true)

  if [[ -n "${best_source}" ]]; then
    printf '%s%s' "${best_source}" "${abs_path#${best_destination}}"
  else
    printf '%s' "${abs_path}"
  fi
}

workspace_path_to_host_path() {
  local workspace_path="$1"
  if is_windows_drive_path "${workspace_path}"; then
    windows_path_to_host_path "${workspace_path}"
  else
    linux_path_to_host_path "${workspace_path}"
  fi
}

docker_bind_source_exists() {
  local host_path="$1"
  docker run --rm \
    --mount "type=bind,src=${host_path},dst=/cmgr-probe,readonly" \
    alpine:3.20 \
    test -d /cmgr-probe >/dev/null 2>&1
}

verify_workspace_path() {
  local workspace_path="$1" host_workspace_path="$2"

  if [[ -d "${host_workspace_path}" ]]; then
    return 0
  fi

  docker_bind_source_exists "${host_workspace_path}"
}

load_legacy_runtime_env() {
  local file="${LEGACY_RUNTIME_ENV}" line key value
  [[ -f "${file}" ]] || return 0

  while IFS= read -r line; do
    [[ -n "${line}" && "${line}" != \#* && "${line}" == *=* ]] || continue
    key="${line%%=*}"
    value="${line#*=}"
    case "${key}" in
      OPENAI_API_KEY|OPENAI_API_BASE|CODEX_API_BASE|CODEX_GATEWAY_ADMIN_ORIGIN|CMGR_CODEX_REMOTE_LOOPBACK_PORT|HTTP_PROXY|HTTPS_PROXY|NO_PROXY)
        if [[ -z "${!key:-}" ]]; then
          export "${key}=${value}"
        fi
        ;;
    esac
  done < "${file}"
}

cache_key_for_value() {
  local value="$1"
  python3 - "$value" <<'PY'
import hashlib
import sys

print(hashlib.sha256(sys.argv[1].encode("utf-8")).hexdigest()[:16])
PY
}

runtime_layout_ready() {
  [[ -f "${SCRIPT_DIR}/Dockerfile" ]] || return 1
  [[ -f "${SCRIPT_DIR}/scripts/container-bootstrap.sh" ]] || return 1
  [[ -d "${SCRIPT_DIR}/skills" ]] || return 1
  [[ -d "${PROJECT_DIR}/sh/new" ]] || return 1
}

validate_skill_frontmatter() {
  local skill_file="$1"
  python3 - "${skill_file}" <<'PY'
from pathlib import Path
import sys

content = Path(sys.argv[1]).read_text(encoding="utf-8")
lines = content.splitlines()
if len(lines) < 3:
    raise SystemExit(1)
if lines[0].strip() != "---":
    raise SystemExit(1)
try:
    closing_index = next(index for index, line in enumerate(lines[1:], start=1) if line.strip() == "---")
except StopIteration:
    raise SystemExit(1)
raise SystemExit(0 if closing_index >= 2 else 1)
PY
}

validate_skill_bundle() {
  local skills_root="$1" skill_file found=0
  while IFS= read -r -d '' skill_file; do
    found=1
    validate_skill_frontmatter "${skill_file}" || {
      printf '[install][error] 无效技能文件: %s\n' "${skill_file}" >&2
      return 1
    }
  done < <(find "${skills_root}" -mindepth 2 -maxdepth 2 -type f -name 'SKILL.md' -print0 | sort -z)

  (( found )) || {
    printf '[install][error] 未在 %s 下找到任何 SKILL.md\n' "${skills_root}" >&2
    return 1
  }
}

write_embedded_runtime_layout() {
  local bootstrap_root="$1"
  local runtime_root="${bootstrap_root}/sh/new"

  mkdir -p \
    "${runtime_root}/scripts" \
    "${runtime_root}/skills/coding-core" \
    "${runtime_root}/skills/ui-ux-pro-max/scripts"

  cat > "${runtime_root}/Dockerfile" <<'EOF'
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
EOF

  cat > "${runtime_root}/scripts/container-bootstrap.sh" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail

export HOME="${HOME:-/root}"
export CODEX_HOME="${CODEX_HOME:-${HOME}/.codex}"
export CODEX_MANAGED_SKILLS_ROOT="${CODEX_MANAGED_SKILLS_ROOT:-${HOME}/.agents/skills}"
export CODEX_LEGACY_SKILLS_ROOT="${CODEX_LEGACY_SKILLS_ROOT:-${CODEX_HOME}/skills}"
export CODEX_DEFAULT_PROJECT="${CODEX_DEFAULT_PROJECT:-/workspace}"
export CMGR_DOCKER_MODE="${CMGR_DOCKER_MODE:-none}"
export CMGR_INNER_DOCKER_DATA_ROOT="${CMGR_INNER_DOCKER_DATA_ROOT:-/var/lib/docker}"
export CMGR_INNER_DOCKER_PIDFILE="${CMGR_INNER_DOCKER_PIDFILE:-/var/run/dockerd.pid}"
export CMGR_INNER_DOCKER_LOG="${CMGR_INNER_DOCKER_LOG:-/var/log/dockerd.log}"

mkdir -p "${HOME}" "${CODEX_HOME}" "${HOME}/.agents" "${CODEX_MANAGED_SKILLS_ROOT}" "${CODEX_LEGACY_SKILLS_ROOT}"

sync_seeded_skills() {
  local seed_root="/opt/codex-skills" target_dir skill_dir skill_name
  [[ -d "${seed_root}" ]] || return 0

  for target_dir in "${CODEX_MANAGED_SKILLS_ROOT}" "${CODEX_LEGACY_SKILLS_ROOT}"; do
    mkdir -p "${target_dir}"
    for skill_dir in "${seed_root}"/*; do
      [[ -d "${skill_dir}" ]] || continue
      skill_name="$(basename "${skill_dir}")"
      rm -rf -- "${target_dir:?}/${skill_name}"
      cp -a "${skill_dir}" "${target_dir}/"
    done
  done
}

sync_seeded_skills

wait_for_inner_docker() {
  local attempts="${1:-30}" attempt
  for ((attempt=1; attempt<=attempts; attempt+=1)); do
    if docker version >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

cleanup_inner_docker_runtime() {
  pkill -TERM dockerd >/dev/null 2>&1 || true
  pkill -TERM containerd >/dev/null 2>&1 || true
  sleep 1
  pkill -KILL dockerd >/dev/null 2>&1 || true
  pkill -KILL containerd >/dev/null 2>&1 || true
  rm -rf /var/run/docker /var/run/containerd
  rm -f "${CMGR_INNER_DOCKER_PIDFILE}" /var/run/docker.sock
}

start_inner_docker_with_driver() {
  local storage_driver="$1"

  cleanup_inner_docker_runtime
  nohup dockerd \
    --host=unix:///var/run/docker.sock \
    --data-root="${CMGR_INNER_DOCKER_DATA_ROOT}" \
    --pidfile="${CMGR_INNER_DOCKER_PIDFILE}" \
    --storage-driver="${storage_driver}" \
    >"${CMGR_INNER_DOCKER_LOG}" 2>&1 &

  if wait_for_inner_docker 30; then
    return 0
  fi

  if [[ -f "${CMGR_INNER_DOCKER_PIDFILE}" ]]; then
    kill "$(cat "${CMGR_INNER_DOCKER_PIDFILE}")" >/dev/null 2>&1 || true
  fi
  rm -f "${CMGR_INNER_DOCKER_PIDFILE}" /var/run/docker.sock
  return 1
}

start_inner_docker() {
  [[ "${CMGR_DOCKER_MODE}" == "isolated" ]] || return 0

  command -v dockerd >/dev/null 2>&1 || {
    printf '[bootstrap][error] dockerd 不存在，无法启动容器内独立 Docker。\n' >&2
    return 1
  }

  mkdir -p "${CMGR_INNER_DOCKER_DATA_ROOT}" "$(dirname "${CMGR_INNER_DOCKER_PIDFILE}")" "$(dirname "${CMGR_INNER_DOCKER_LOG}")"
  chmod 711 "$(dirname "${CMGR_INNER_DOCKER_LOG}")" >/dev/null 2>&1 || true

  if docker version >/dev/null 2>&1; then
    return 0
  fi

  start_inner_docker_with_driver overlay2 || start_inner_docker_with_driver vfs
}

start_inner_docker

CODEX_ACTIVE_API_BASE="${OPENAI_API_BASE:-${CODEX_API_BASE:-}}"
if [[ -n "${CODEX_ACTIVE_API_BASE}" ]]; then
  export CODEX_ACTIVE_API_BASE
  python3 - <<'PY'
import os
from pathlib import Path

config_path = Path(os.environ.get("CODEX_HOME", "/root/.codex")) / "config.toml"
config_path.parent.mkdir(parents=True, exist_ok=True)
existing = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
lines = [
    line for line in existing.splitlines()
    if not line.lstrip().startswith("openai_base_url")
]
entry = f'openai_base_url = "{os.environ["CODEX_ACTIVE_API_BASE"]}"'
insert_at = next(
    (index for index, line in enumerate(lines) if line.lstrip().startswith("[")),
    len(lines),
)
lines.insert(insert_at, entry)
config_path.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
PY
fi

if [[ -n "${OPENAI_API_KEY:-}" ]]; then
  python3 - <<'PY'
import json
import os
from pathlib import Path

codex_home = Path(os.environ.get("CODEX_HOME", "/root/.codex"))
codex_home.mkdir(parents=True, exist_ok=True)
(codex_home / "auth.json").write_text(
    json.dumps(
        {
            "auth_mode": "apikey",
            "OPENAI_API_KEY": os.environ["OPENAI_API_KEY"],
        },
        ensure_ascii=False,
        indent=2,
    )
    + "\n",
    encoding="utf-8",
)
PY
  chmod 600 "${CODEX_HOME}/auth.json"
elif [[ -f "${CODEX_HOME}/auth.json" ]]; then
  chmod 600 "${CODEX_HOME}/auth.json"
fi

if [[ -d "${CODEX_DEFAULT_PROJECT}" ]]; then
  mkdir -p "${CODEX_DEFAULT_PROJECT}/.codex"
fi

exec "$@"
EOF
  chmod +x "${runtime_root}/scripts/container-bootstrap.sh"

  cat > "${runtime_root}/skills/coding-core/SKILL.md" <<'EOF'
---
name: coding-core
description: Minimal embedded coding workflow used by standalone install.sh bootstraps.
---

# Coding Core

Standalone `install.sh` bootstrap installed the minimal `coding-core` skill pack.

This placeholder keeps the container layout compatible with the Codex runtime.
EOF

  cat > "${runtime_root}/skills/ui-ux-pro-max/SKILL.md" <<'EOF'
---
name: ui-ux-pro-max
description: Minimal embedded UI/UX workflow used by standalone install.sh bootstraps.
---

# UI UX Pro Max

Standalone `install.sh` bootstrap installed the minimal `ui-ux-pro-max` skill pack.

This lightweight bundle preserves the expected directory layout and ships a minimal
`search.py` implementation so bootstrap validation can run without the full dataset.
EOF

  cat > "${runtime_root}/skills/ui-ux-pro-max/scripts/search.py" <<'EOF'
#!/usr/bin/env python3
import argparse


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("query", nargs="?", default="")
    parser.add_argument("--domain", default="general")
    args = parser.parse_args()

    catalog = {
        "style": [
            "bold information hierarchy",
            "clean dashboard spacing",
            "high-contrast action emphasis",
        ],
        "general": [
            "consistent component rhythm",
            "clear typography scale",
            "usable mobile-first layout",
        ],
    }

    for item in catalog.get(args.domain, catalog["general"]):
        print(f"{item}: {args.query}".strip())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
EOF
  chmod +x "${runtime_root}/skills/ui-ux-pro-max/scripts/search.py"
}

ensure_runtime_layout() {
  local repo_url repo_ref cache_key bootstrap_root
  local clone_ready=0

  if runtime_layout_ready; then
    return 0
  fi

  repo_url="${CMGR_BOOTSTRAP_REPO_URL:-${DEFAULT_REPO_URL:-${DEFAULT_BOOTSTRAP_REPO_URL}}}"
  repo_ref="${CMGR_BOOTSTRAP_REPO_REF:-${DEFAULT_REPO_REF:-main}}"
  [[ -n "${repo_url}" ]] || repo_url="${DEFAULT_BOOTSTRAP_REPO_URL}"
  [[ -n "${repo_ref}" ]] || repo_ref="main"

  mkdir -p "${BOOTSTRAP_CACHE_DIR}"
  cache_key="$(cache_key_for_value "${repo_url}|${repo_ref}")"
  bootstrap_root="${BOOTSTRAP_CACHE_DIR}/${cache_key}"

  if [[ -d "${bootstrap_root}/.git" ]]; then
    git config --global --add safe.directory "${bootstrap_root}"
    git -C "${bootstrap_root}" remote set-url origin "${repo_url}" >/dev/null 2>&1 || true
    git -C "${bootstrap_root}" fetch --prune --tags origin >/dev/null 2>&1 || true
    clone_ready=1
  else
    rm -rf "${bootstrap_root}"
    if ! git clone --depth 1 --branch "${repo_ref}" "${repo_url}" "${bootstrap_root}" >/dev/null 2>&1; then
      rm -rf "${bootstrap_root}"
      if git clone "${repo_url}" "${bootstrap_root}" >/dev/null 2>&1; then
        git config --global --add safe.directory "${bootstrap_root}"
        git -C "${bootstrap_root}" fetch --prune --tags origin >/dev/null 2>&1 || true
        clone_ready=1
      fi
    else
      git config --global --add safe.directory "${bootstrap_root}"
      clone_ready=1
    fi
  fi

  if (( clone_ready )); then
    if git -C "${bootstrap_root}" show-ref --verify --quiet "refs/remotes/origin/${repo_ref}"; then
      git -C "${bootstrap_root}" checkout -B "${repo_ref}" "origin/${repo_ref}" >/dev/null 2>&1 || true
    else
      git -C "${bootstrap_root}" checkout --detach "${repo_ref}" >/dev/null 2>&1 || true
    fi
    git -C "${bootstrap_root}" reset --hard >/dev/null 2>&1 || true
    git -C "${bootstrap_root}" clean -fdx >/dev/null 2>&1 || true
  fi

  if [[ ! -f "${bootstrap_root}/sh/new/Dockerfile" || \
        ! -f "${bootstrap_root}/sh/new/scripts/container-bootstrap.sh" || \
        ! -d "${bootstrap_root}/sh/new/skills" ]]; then
    warn "自动拉取到的仓库缺少 sh/new 运行资源，将回退到 install.sh 内置的最小运行布局。"
    rm -rf "${bootstrap_root}"
    mkdir -p "${bootstrap_root}"
    write_embedded_runtime_layout "${bootstrap_root}"
  fi

  SCRIPT_DIR="${bootstrap_root}/sh/new"
  PROJECT_DIR="${bootstrap_root}"
  LEGACY_RUNTIME_ENV="${SCRIPT_DIR}/../.runtime.env"
  log "检测到当前只有 install.sh，已自动同步运行资源到: ${bootstrap_root}"
}

latest_container_name() {
  local latest_name=""
  while IFS= read -r latest_name; do
    :
  done < <(docker ps -a --format '{{.Names}}' | grep -E "^${DEFAULT_CONTAINER_PREFIX}[0-9]+$" | sort -V)

  if [[ -n "${latest_name}" ]]; then
    printf '%s' "${latest_name}"
  else
    printf '%s1' "${DEFAULT_CONTAINER_PREFIX}"
  fi
}

next_container_name() {
  local max_id=0 name current_id
  while IFS= read -r name; do
    [[ "${name}" =~ ^${DEFAULT_CONTAINER_PREFIX}([0-9]+)$ ]] || continue
    current_id="${BASH_REMATCH[1]}"
    if (( current_id > max_id )); then
      max_id="${current_id}"
    fi
  done < <(docker ps -a --format '{{.Names}}')
  printf '%s%d' "${DEFAULT_CONTAINER_PREFIX}" "$((max_id + 1))"
}

container_network_name() {
  printf '%s-net' "$1"
}

container_volume_name() {
  printf '%s-home' "$1"
}

container_agents_volume_name() {
  printf '%s-agents' "$1"
}

container_docker_volume_name() {
  printf '%s-docker' "$1"
}

ensure_network_exists() {
  local network_name="$1"
  docker network inspect "${network_name}" >/dev/null 2>&1 || docker network create "${network_name}" >/dev/null
}

ensure_volume_exists() {
  local volume_name="$1"
  docker volume inspect "${volume_name}" >/dev/null 2>&1 || docker volume create "${volume_name}" >/dev/null
}

running_gateway_server_containers() {
  local preferred_stack preferred_container
  preferred_stack="$(state_get 'LAST_DEPLOY_STACK_NAME' 2>/dev/null || true)"
  preferred_container=""
  {
    if [[ -n "${preferred_stack}" ]]; then
      preferred_container="${preferred_stack}-server"
      if docker inspect "${preferred_container}" >/dev/null 2>&1; then
        printf '%s\n' "${preferred_container}"
      fi
    fi
    docker ps --filter 'label=com.docker.compose.service=server' --format '{{.Names}}'
    docker ps --format '{{.Names}}' | grep -E '.+-server$' || true
  } | awk 'NF && !seen[$0]++'
}

gateway_server_networks() {
  local container_name="$1"
  docker inspect -f '{{range $name, $cfg := .NetworkSettings.Networks}}{{printf "%s\n" $name}}{{end}}' "${container_name}" 2>/dev/null || true
}

probe_gateway_api_key_on_network() {
  local network_name="$1" server_container="$2" api_key="$3" status_code
  [[ -n "${network_name}" && -n "${server_container}" && -n "${api_key}" ]] || return 1

  status_code="$(
    docker run --rm --network "${network_name}" curlimages/curl:8.12.1 \
      -s -o /dev/null -w '%{http_code}' \
      -H "Authorization: Bearer ${api_key}" \
      -H "x-codex-cli-affinity-id: install-gateway-probe" \
      "http://${server_container}:8080/v1/models" 2>/dev/null || true
  )"
  [[ "${status_code}" == "200" ]]
}

detect_gateway_for_api_key() {
  local api_key="$1" server_container network_name
  [[ -n "${api_key}" ]] || return 1

  while IFS= read -r server_container; do
    [[ -n "${server_container}" ]] || continue
    while IFS= read -r network_name; do
      [[ -n "${network_name}" ]] || continue
      if probe_gateway_api_key_on_network "${network_name}" "${server_container}" "${api_key}"; then
        printf '%s\t%s\t%s\n' "${server_container}" "${network_name}" "http://${server_container}:8080/v1"
        return 0
      fi
    done < <(gateway_server_networks "${server_container}")
  done < <(running_gateway_server_containers)

  return 1
}

container_host_port() {
  local container_name="$1" container_port="$2" mapping port
  mapping="$(docker port "${container_name}" "${container_port}/tcp" 2>/dev/null | awk 'NR==1 {print $NF}')" || true
  port="${mapping##*:}"
  [[ "${port}" =~ ^[0-9]+$ ]] || return 1
  printf '%s' "${port}"
}

gateway_admin_origin_for_container() {
  local container_name="$1" host_port
  host_port="$(container_host_port "${container_name}" 8081)" || return 1
  printf 'http://host.docker.internal:%s' "${host_port}"
}

connect_container_to_network_if_needed() {
  local container_name="$1" network_name="$2" current_networks
  [[ -n "${network_name}" ]] || return 0

  current_networks="$(docker inspect -f '{{range $name, $cfg := .NetworkSettings.Networks}}{{printf "%s\n" $name}}{{end}}' "${container_name}" 2>/dev/null || true)"
  grep -Fxq "${network_name}" <<<"${current_networks}" || docker network connect "${network_name}" "${container_name}" >/dev/null
}

verify_gateway_models_access() {
  local container_name="$1" api_base="$2" api_key="$3"
  [[ -n "${container_name}" && -n "${api_base}" && -n "${api_key}" ]] || return 0

  docker exec "${container_name}" bash -lc \
    "curl -fsS -o /dev/null -H \"Authorization: Bearer ${api_key}\" -H \"x-codex-cli-affinity-id: install-verify\" \"${api_base%/}/models\""
}

verify_gateway_admin_access() {
  local container_name="$1" admin_origin="$2"
  [[ -n "${container_name}" && -n "${admin_origin}" ]] || return 0

  docker exec "${container_name}" bash -lc \
    "curl -fsS -o /dev/null \"${admin_origin%/}/health\""
}

create_container_if_missing() {
  local container_name="$1"
  shift
  docker inspect "${container_name}" >/dev/null 2>&1 || docker "$@"
}

ensure_container_matches_spec() {
  local container_name="$1" image_name="$2" host_workspace_path="$3" network_name="$4" volume_name="$5" agents_volume_name="$6" docker_mode="$7" docker_volume_name="$8"
  local current_image current_workdir current_networks workspace_source codex_volume agents_volume docker_sock_bind docker_data_volume privileged current_docker_mode

  docker inspect "${container_name}" >/dev/null 2>&1 || return 0

  current_image="$(docker inspect -f '{{.Config.Image}}' "${container_name}")"
  current_workdir="$(docker inspect -f '{{.Config.WorkingDir}}' "${container_name}")"
  current_networks="$(docker inspect -f '{{range $name, $cfg := .NetworkSettings.Networks}}{{printf "%s\n" $name}}{{end}}' "${container_name}")"
  workspace_source="$(
    docker inspect -f '{{range .Mounts}}{{if eq .Destination "/workspace"}}{{if eq .Type "bind"}}{{.Source}}{{end}}{{end}}{{end}}' "${container_name}"
  )"
  codex_volume="$(
    docker inspect -f '{{range .Mounts}}{{if eq .Destination "/root/.codex"}}{{if eq .Type "volume"}}{{.Name}}{{end}}{{end}}{{end}}' "${container_name}"
  )"
  agents_volume="$(
    docker inspect -f '{{range .Mounts}}{{if eq .Destination "/root/.agents"}}{{if eq .Type "volume"}}{{.Name}}{{end}}{{end}}{{end}}' "${container_name}"
  )"
  docker_sock_bind="$(
    docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/run/docker.sock"}}{{if eq .Type "bind"}}{{.Source}}{{end}}{{end}}{{end}}' "${container_name}"
  )"
  docker_data_volume="$(
    docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/docker"}}{{if eq .Type "volume"}}{{.Name}}{{end}}{{end}}{{end}}' "${container_name}"
  )"
  privileged="$(docker inspect -f '{{.HostConfig.Privileged}}' "${container_name}")"
  current_docker_mode="$(
    docker inspect -f '{{range .Config.Env}}{{println .}}{{end}}' "${container_name}" | awk -F= '$1 == "CMGR_DOCKER_MODE" {print $2; exit}'
  )"

  if [[ "${current_image}" != "${image_name}" ]] || \
     [[ "${current_workdir}" != "/workspace" ]] || \
     [[ "${workspace_source}" != "${host_workspace_path}" ]] || \
     [[ "${codex_volume}" != "${volume_name}" ]] || \
     [[ "${agents_volume}" != "${agents_volume_name}" ]] || \
     ! grep -Fxq "${network_name}" <<<"${current_networks}"; then
    log "检测到容器 ${container_name} 的镜像、工作区或卷配置已变化，将重建容器。"
    docker rm -f "${container_name}" >/dev/null
    return 0
  fi

  case "${docker_mode}" in
    isolated)
      if [[ "${current_docker_mode}" != "isolated" ]] || \
         [[ "${docker_data_volume}" != "${docker_volume_name}" ]] || \
         [[ "${privileged}" != "true" ]] || \
         [[ -n "${docker_sock_bind}" ]]; then
        log "检测到容器 ${container_name} 的内置 Docker 模式配置已变化，将重建容器。"
        docker rm -f "${container_name}" >/dev/null
      fi
      ;;
    host)
      if [[ "${current_docker_mode}" != "host" ]] || \
         [[ "${docker_sock_bind}" != "/var/run/docker.sock" ]] || \
         [[ -n "${docker_data_volume}" ]]; then
        log "检测到容器 ${container_name} 的宿主 Docker 代理配置已变化，将重建容器。"
        docker rm -f "${container_name}" >/dev/null
      fi
      ;;
    none)
      if [[ "${current_docker_mode}" != "none" ]] || \
         [[ -n "${docker_sock_bind}" ]] || \
         [[ -n "${docker_data_volume}" ]]; then
        log "检测到容器 ${container_name} 的 Docker 模式配置已变化，将重建容器。"
        docker rm -f "${container_name}" >/dev/null
      fi
      ;;
    *)
      die "未知 Docker 模式: ${docker_mode}"
      ;;
  esac
}

normalize_docker_mode() {
  local input="${1:-}"
  case "${input}" in
    0|none|NONE|None)
      printf '%s' "none"
      ;;
    1|isolated|ISOLATED|Isolated)
      printf '%s' "isolated"
      ;;
    2|host|HOST|Host|host-socket|HOST-SOCKET|Host-socket)
      printf '%s' "host"
      ;;
    *)
      return 1
      ;;
  esac
}

docker_mode_prompt_default() {
  local saved_mode
  saved_mode="$(state_get 'LAST_DOCKER_MODE' 2>/dev/null || true)"
  if [[ -n "${saved_mode}" ]]; then
    case "${saved_mode}" in
      none) printf '%s' "0" ;;
      isolated) printf '%s' "1" ;;
      host) printf '%s' "2" ;;
      *) printf '%s' "1" ;;
    esac
  else
    printf '%s' "1"
  fi
}

default_workspace_path() {
  local last_deploy_dir last_workspace_path last_host_workspace_path

  last_deploy_dir="$(state_get 'LAST_DEPLOY_DIR' 2>/dev/null || true)"
  if [[ -n "${last_deploy_dir}" && -d "${last_deploy_dir}" ]]; then
    printf '%s' "${last_deploy_dir}"
    return 0
  fi

  last_workspace_path="$(state_get 'LAST_WORKSPACE_PATH' 2>/dev/null || true)"
  last_host_workspace_path="$(state_get 'LAST_HOST_WORKSPACE_PATH' 2>/dev/null || true)"
  if [[ -n "${last_workspace_path}" && ( -d "${last_workspace_path}" || -d "${last_host_workspace_path}" ) ]]; then
    printf '%s' "${last_workspace_path}"
    return 0
  fi

  printf '%s' "${PROJECT_DIR}"
}

refresh_container_bootstrap() {
  local container_name="$1" api_key="$2" api_base="$3" default_project="$4" gateway_admin_origin="$5"
  local -a exec_args

  exec_args=(
    exec
    -e "CODEX_DEFAULT_PROJECT=${default_project}"
  )

  if [[ -n "${api_key}" ]]; then
    exec_args+=(-e "OPENAI_API_KEY=${api_key}")
  fi
  if [[ -n "${api_base}" ]]; then
    exec_args+=(-e "OPENAI_API_BASE=${api_base}" -e "CODEX_API_BASE=${api_base}")
  fi
  if [[ -n "${gateway_admin_origin}" ]]; then
    exec_args+=(
      -e "CODEX_GATEWAY_ADMIN_ORIGIN=${gateway_admin_origin}"
      -e "CMGR_CODEX_REMOTE_LOOPBACK_PORT=19081"
    )
  fi

  exec_args+=(
    "${container_name}"
    bash
    -lc
    "/usr/local/bin/container-bootstrap true"
  )

  docker "${exec_args[@]}"
}

start_container_if_needed() {
  local container_name="$1"
  if [[ "$(docker inspect -f '{{.State.Running}}' "${container_name}" 2>/dev/null || true)" == "true" ]]; then
    return 0
  fi
  docker start "${container_name}" >/dev/null
}

remove_container_if_exists() {
  local container_name="$1"
  docker inspect "${container_name}" >/dev/null 2>&1 && docker rm -f "${container_name}" >/dev/null || true
}

remove_image_if_exists() {
  local image_name="$1"
  docker image inspect "${image_name}" >/dev/null 2>&1 && docker rmi -f "${image_name}" >/dev/null || true
}

remove_network_if_exists() {
  local network_name="$1"
  docker network inspect "${network_name}" >/dev/null 2>&1 && docker network rm "${network_name}" >/dev/null || true
}

remove_volume_if_exists() {
  local volume_name="$1"
  docker volume inspect "${volume_name}" >/dev/null 2>&1 && docker volume rm -f "${volume_name}" >/dev/null || true
}

select_base_compose_file() {
  if [[ -f "${1}/compose.current.test.yml" ]]; then
    printf '%s' "compose.current.test.yml"
  else
    printf '%s' "compose.yml"
  fi
}

is_current_project_repo() {
  local repo_dir="$1"
  [[ -f "${repo_dir}/infra/docker/browser-assist.Dockerfile" ]] || return 1
  [[ -f "${repo_dir}/infra/docker/server.Dockerfile" ]] || return 1
  [[ -f "${repo_dir}/infra/docker/web.Dockerfile" ]] || return 1
  [[ -f "${repo_dir}/compose.yml" || -f "${repo_dir}/compose.current.test.yml" ]] || return 1
}

default_deploy_dir() {
  if is_current_project_repo "${PROJECT_DIR}"; then
    printf '%s' "${PROJECT_DIR}/.docker-deploy/codex-manager"
  else
    resolve_path "${HOME:-/root}/.docker-deploy/codex-manager"
  fi
}

write_current_project_deploy_compose() {
  local compose_file="$1" stack_name="$2" web_port="$3" data_port="$4" admin_port="$5" browser_port="$6" postgres_port="$7" redis_port="$8"
  cat > "${compose_file}" <<EOF
services:
  postgres:
    container_name: ${stack_name}-postgres
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: codex_manager
      POSTGRES_USER: codex_manager
      POSTGRES_PASSWORD: codex_manager
    ports:
      - "${postgres_port}:5432"
    volumes:
      - ${stack_name}-postgres-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U codex_manager -d codex_manager"]
      interval: 10s
      timeout: 5s
      retries: 5

  redis:
    container_name: ${stack_name}-redis
    image: redis:7-alpine
    ports:
      - "${redis_port}:6379"
    command: ["redis-server", "--save", "", "--appendonly", "no"]
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5

  browser-assist:
    container_name: ${stack_name}-browser-assist
    build:
      context: .
      dockerfile: infra/docker/browser-assist.Dockerfile
    environment:
      PORT: 8090
      CMGR_BROWSER_ASSIST_IDLE_TTL_MS: 180000
      CMGR_BROWSER_ASSIST_CHROMIUM_PATH: /usr/bin/chromium-browser
      CMGR_DIRECT_PROXY_URL: \${CMGR_DIRECT_PROXY_URL:-}
      CMGR_WARP_PROXY_URL: \${CMGR_WARP_PROXY_URL:-}
      CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL: \${CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL:-}
      CMGR_BROWSER_ASSIST_WARP_PROXY_URL: \${CMGR_BROWSER_ASSIST_WARP_PROXY_URL:-}
    ports:
      - "${browser_port}:8090"
    depends_on:
      redis:
        condition: service_started

  server:
    container_name: ${stack_name}-server
    build:
      context: .
      dockerfile: infra/docker/server.Dockerfile
    environment:
      CMGR_SERVER_BIND_ADDR: 0.0.0.0
      CMGR_SERVER_DATA_PORT: 8080
      CMGR_SERVER_ADMIN_PORT: 8081
      CMGR_POSTGRES_URL: postgres://codex_manager:codex_manager@postgres:5432/codex_manager
      CMGR_REDIS_URL: redis://redis:6379
      CMGR_REDIS_CHANNEL: cmgr:control-events
      CMGR_BROWSER_ASSIST_URL: http://browser-assist:8090
      CMGR_GATEWAY_HEARTBEAT_SECONDS: 5
      CMGR_ENABLE_DEMO_SEED: "false"
      CMGR_ACCOUNT_ENCRYPTION_KEY: \${CMGR_ACCOUNT_ENCRYPTION_KEY:-}
      CMGR_DIRECT_PROXY_URL: \${CMGR_DIRECT_PROXY_URL:-}
      CMGR_WARP_PROXY_URL: \${CMGR_WARP_PROXY_URL:-}
      CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL: \${CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL:-}
      CMGR_BROWSER_ASSIST_WARP_PROXY_URL: \${CMGR_BROWSER_ASSIST_WARP_PROXY_URL:-}
    ports:
      - "${data_port}:8080"
      - "${admin_port}:8081"
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
      browser-assist:
        condition: service_started

  web:
    container_name: ${stack_name}-web
    build:
      context: .
      dockerfile: infra/docker/web.Dockerfile
    environment:
      PORT: 3000
      SERVER_ADMIN_ORIGIN: http://server:8081
    ports:
      - "${web_port}:3000"
    depends_on:
      server:
        condition: service_started

volumes:
  ${stack_name}-postgres-data:
EOF
}

wait_for_container_running() {
  local container_name="$1" attempts="${2:-20}" attempt running
  for ((attempt=1; attempt<=attempts; attempt+=1)); do
    running="$(docker inspect -f '{{.State.Running}}' "${container_name}" 2>/dev/null || true)"
    if [[ "${running}" == "true" ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

wait_for_http_ready() {
  local label="$1" url="$2" attempts="${3:-60}" expected_statuses_raw="${4:-200 204 301 302 307 308}" attempt status_code
  local -a expected_statuses
  read -r -a expected_statuses <<< "${expected_statuses_raw}"
  for ((attempt=1; attempt<=attempts; attempt+=1)); do
    status_code="$(curl -s -o /dev/null -w '%{http_code}' "${url}" || true)"
    if printf '%s\n' "${expected_statuses[@]}" | grep -Fxq "${status_code}"; then
      log "${label} 已就绪: ${url} (HTTP ${status_code})"
      return 0
    fi
    sleep 2
  done
  return 1
}

wait_for_container_http_ready() {
  local label="$1" container_name="$2" url="$3" attempts="${4:-60}" expected_statuses_raw="${5:-200 204 301 302 307 308}" attempt status_code
  local -a expected_statuses
  read -r -a expected_statuses <<< "${expected_statuses_raw}"
  for ((attempt=1; attempt<=attempts; attempt+=1)); do
    status_code="$(
      docker run --rm --network "container:${container_name}" curlimages/curl:8.12.1 \
        -s -o /dev/null -w '%{http_code}' "${url}" 2>/dev/null || true
    )"
    if printf '%s\n' "${expected_statuses[@]}" | grep -Fxq "${status_code}"; then
      log "${label} 已就绪: ${container_name} ${url} (HTTP ${status_code})"
      return 0
    fi
    sleep 2
  done
  return 1
}

docker_port_in_use() {
  local port="$1"
  docker ps --format '{{.Ports}}' | tr ',' '\n' | grep -Eq "0\\.0\\.0\\.0:${port}->|\\[::\\]:${port}->|:::${port}->"
}

host_port_in_use() {
  local port="$1"
  python3 - "${port}" <<'PY'
import socket
import sys

port = int(sys.argv[1])


def port_busy(family, address):
    sock = socket.socket(family, socket.SOCK_STREAM)
    try:
        if family == socket.AF_INET6:
            try:
                sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
            except OSError:
                pass
        sock.bind((address, port))
    except OSError:
        return True
    finally:
        sock.close()
    return False


busy = port_busy(socket.AF_INET, "0.0.0.0")
try:
    busy = busy or port_busy(socket.AF_INET6, "::")
except OSError:
    pass

raise SystemExit(0 if busy else 1)
PY
}

pick_free_port() {
  local start_port="$1" end_port="$2" port
  for ((port=start_port; port<=end_port; port+=1)); do
    if ! docker_port_in_use "${port}" && ! host_port_in_use "${port}"; then
      printf '%s' "${port}"
      return 0
    fi
  done
  return 1
}

clone_or_update_repo() {
  local repo_url="$1" repo_ref="$2" deploy_dir="$3"

  if [[ -d "${deploy_dir}/.git" ]]; then
    git config --global --add safe.directory "${deploy_dir}"
    git -C "${deploy_dir}" remote set-url origin "${repo_url}"
    git -C "${deploy_dir}" fetch --prune --tags origin
  elif [[ ! -e "${deploy_dir}" || -z "$(ls -A "${deploy_dir}" 2>/dev/null)" ]]; then
    rm -rf "${deploy_dir}"
    git clone "${repo_url}" "${deploy_dir}"
    git config --global --add safe.directory "${deploy_dir}"
  else
    echo "部署目录已存在且不是 Git 仓库: ${deploy_dir}" >&2
    return 1
  fi

  git config --global --add safe.directory "${deploy_dir}"
  git -C "${deploy_dir}" fetch --prune --tags origin
  if git -C "${deploy_dir}" show-ref --verify --quiet "refs/remotes/origin/${repo_ref}"; then
    git -C "${deploy_dir}" checkout -B "${repo_ref}" "origin/${repo_ref}" >/dev/null
  else
    git -C "${deploy_dir}" checkout --detach "${repo_ref}" >/dev/null
  fi
  git -C "${deploy_dir}" reset --hard >/dev/null
  git -C "${deploy_dir}" clean -fdx >/dev/null
}

compose_up_deploy() {
  local deploy_dir="$1" compose_file="$2"
  (
    cd "${deploy_dir}"
    docker compose -f "${compose_file}" up -d --build
  )
}

build_image_action() {
  local action="build_image"
  local image_name

  image_name="$(prompt_default '请输入镜像名' "$(state_get_or_default 'LAST_IMAGE_NAME' "${DEFAULT_IMAGE_NAME}")")"
  activate_action_context "${action}" "image=${image_name}"
  state_set 'LAST_IMAGE_NAME' "${image_name}"

  run_step "${action}" "prepare_runtime" ensure_runtime_layout
  run_step "${action}" "verify_inputs" test -f "${SCRIPT_DIR}/Dockerfile"
  run_step "${action}" "verify_skills" validate_skill_bundle "${SCRIPT_DIR}/skills"
  run_step "${action}" "docker_build" docker build -f "${SCRIPT_DIR}/Dockerfile" -t "${image_name}" "${PROJECT_DIR}"
  finish_action "${action}"
  log "镜像构建完成: ${image_name}"
}

create_container_action() {
  local action="create_container"
  local image_name default_container_name container_name workspace_path host_workspace_path
  local docker_mode docker_mode_input network_name gateway_network_name volume_name agents_volume_name docker_volume_name
  local api_key api_base default_api_base gateway_admin_origin default_admin_origin
  local detected_gateway detected_gateway_server detected_api_base detected_gateway_admin_origin
  local -a docker_args

  load_legacy_runtime_env

  image_name="$(prompt_default '请输入容器使用的镜像名' "$(state_get_or_default 'LAST_IMAGE_NAME' "${DEFAULT_IMAGE_NAME}")")"
  default_container_name="$(state_get_or_default 'LAST_CONTAINER_NAME' "$(next_container_name)")"
  container_name="$(prompt_default '请输入容器名' "${default_container_name}")"
  workspace_path="$(prompt_default '请输入要挂载到容器的工作区路径' "$(default_workspace_path)")"
  api_key="$(prompt_visible_default '请输入 OPENAI_API_KEY（明文显示，便于核对；留空表示沿用当前值或不预置）' "${OPENAI_API_KEY:-}")"
  detected_gateway="$(detect_gateway_for_api_key "${api_key}" 2>/dev/null || true)"
  detected_gateway_server=""
  gateway_network_name=""
  detected_api_base=""
  detected_gateway_admin_origin=""
  if [[ -n "${detected_gateway}" ]]; then
    IFS=$'\t' read -r detected_gateway_server gateway_network_name detected_api_base <<< "${detected_gateway}"
    if [[ -n "${gateway_network_name}" ]]; then
      detected_gateway_admin_origin="http://${detected_gateway_server}:8081"
    else
      detected_gateway_admin_origin="$(gateway_admin_origin_for_container "${detected_gateway_server}" 2>/dev/null || true)"
    fi
    log "已根据 API Key 自动定位网关: ${detected_gateway_server} (${gateway_network_name}) -> ${detected_api_base}"
  fi
  default_api_base="${detected_api_base:-${OPENAI_API_BASE:-${CODEX_API_BASE:-$(state_get_or_default 'LAST_API_BASE' '')}}}"
  api_base="$(prompt_default '请输入网关基础 URL（会同时写入 OPENAI_API_BASE / CODEX_API_BASE，可留空）' "${default_api_base}")"
  default_admin_origin="${detected_gateway_admin_origin:-${CODEX_GATEWAY_ADMIN_ORIGIN:-$(state_get_or_default 'LAST_GATEWAY_ADMIN_ORIGIN' '')}}"
  gateway_admin_origin="$(prompt_default '请输入网关 Admin 地址（用于 Codex /status 远端 app-server，可留空）' "${default_admin_origin}")"
  docker_mode_input="$(prompt_default 'Docker 模式: 0=禁用 1=容器内独立 Docker(推荐) 2=共享宿主 docker.sock' "$(docker_mode_prompt_default)")"
  docker_mode="$(normalize_docker_mode "${docker_mode_input}")" || die "未知 Docker 模式: ${docker_mode_input}"

  host_workspace_path="$(workspace_path_to_host_path "${workspace_path}")"
  if [[ "${host_workspace_path}" != "${workspace_path}" ]]; then
    log "检测到工作区输入需要转换，已映射为宿主路径: ${host_workspace_path}"
  fi
  network_name="$(container_network_name "${container_name}")"
  volume_name="$(container_volume_name "${container_name}")"
  agents_volume_name="$(container_agents_volume_name "${container_name}")"
  docker_volume_name="$(container_docker_volume_name "${container_name}")"

  state_set 'LAST_IMAGE_NAME' "${image_name}"
  state_set 'LAST_CONTAINER_NAME' "${container_name}"
  state_set 'LAST_WORKSPACE_PATH' "${workspace_path}"
  state_set 'LAST_HOST_WORKSPACE_PATH' "${host_workspace_path}"
  state_set 'LAST_DOCKER_MODE' "${docker_mode}"
  if [[ "${docker_mode}" == "none" ]]; then
    state_set 'LAST_DOCKER_ACCESS_ENABLED' "0"
  else
    state_set 'LAST_DOCKER_ACCESS_ENABLED' "1"
  fi
  state_set 'LAST_API_BASE' "${api_base}"
  state_set 'LAST_GATEWAY_ADMIN_ORIGIN' "${gateway_admin_origin}"
  if [[ -n "${gateway_network_name}" ]]; then
    state_set 'LAST_GATEWAY_NETWORK' "${gateway_network_name}"
  fi
  if [[ -n "${detected_gateway_server}" ]]; then
    state_set 'LAST_GATEWAY_SERVER' "${detected_gateway_server}"
  fi
  activate_action_context "${action}" "image=${image_name}|container=${container_name}|workspace=${workspace_path}|api-base=${api_base}|admin=${gateway_admin_origin}|gateway=${gateway_network_name}|docker=${docker_mode}"

  run_step "${action}" "verify_image" docker image inspect "${image_name}" >/dev/null
  run_step "${action}" "verify_workspace" verify_workspace_path "${workspace_path}" "${host_workspace_path}"
  run_step "${action}" "ensure_network" ensure_network_exists "${network_name}"
  if [[ -n "${gateway_network_name}" && "${gateway_network_name}" != "${network_name}" ]]; then
    run_step "${action}" "ensure_gateway_network" ensure_network_exists "${gateway_network_name}"
  fi
  run_step "${action}" "ensure_volume" ensure_volume_exists "${volume_name}"
  run_step "${action}" "ensure_agents_volume" ensure_volume_exists "${agents_volume_name}"
  if [[ "${docker_mode}" == "isolated" ]]; then
    run_step "${action}" "ensure_docker_volume" ensure_volume_exists "${docker_volume_name}"
  fi
  run_step "${action}" "reconcile_container" ensure_container_matches_spec "${container_name}" "${image_name}" "${host_workspace_path}" "${network_name}" "${volume_name}" "${agents_volume_name}" "${docker_mode}" "${docker_volume_name}"

  docker_args=(
    create
    --name "${container_name}"
    --hostname "${container_name}"
    --restart unless-stopped
    --network "${network_name}"
    --workdir /workspace
    --add-host host.docker.internal:host-gateway
    -e CMGR_DOCKER_MODE="${docker_mode}"
    -e HOME=/root
    -e CODEX_HOME=/root/.codex
    -e CODEX_MANAGED_SKILLS_ROOT=/root/.agents/skills
    -e CODEX_LEGACY_SKILLS_ROOT=/root/.codex/skills
    -e CODEX_DEFAULT_PROJECT=/workspace
    -v "${host_workspace_path}:/workspace"
    -v "${volume_name}:/root/.codex"
    -v "${agents_volume_name}:/root/.agents"
  )

  if [[ -n "${api_key}" ]]; then
    docker_args+=(-e "OPENAI_API_KEY=${api_key}")
  fi
  if [[ -n "${api_base}" ]]; then
    docker_args+=(-e "OPENAI_API_BASE=${api_base}" -e "CODEX_API_BASE=${api_base}")
  fi
  if [[ -n "${gateway_admin_origin}" ]]; then
    docker_args+=(
      -e "CODEX_GATEWAY_ADMIN_ORIGIN=${gateway_admin_origin}"
      -e "CMGR_CODEX_REMOTE_LOOPBACK_PORT=19081"
    )
  fi
  if [[ -n "${HTTP_PROXY:-}" ]]; then
    docker_args+=(-e "HTTP_PROXY=${HTTP_PROXY}")
  fi
  if [[ -n "${HTTPS_PROXY:-}" ]]; then
    docker_args+=(-e "HTTPS_PROXY=${HTTPS_PROXY}")
  fi
  if [[ -n "${NO_PROXY:-}" ]]; then
    docker_args+=(-e "NO_PROXY=${NO_PROXY}")
  fi

  case "${docker_mode}" in
    isolated)
      docker_args+=(
        --privileged
        -e DOCKER_HOST=unix:///var/run/docker.sock
        -v "${docker_volume_name}:/var/lib/docker"
      )
      ;;
    host)
      [[ -S /var/run/docker.sock ]] || die "未检测到 /var/run/docker.sock，无法启用共享宿主 Docker。"
      docker_args+=(
        -e DOCKER_HOST=unix:///var/run/docker.sock
        -v /var/run/docker.sock:/var/run/docker.sock
      )
      ;;
    none)
      ;;
    *)
      die "未知 Docker 模式: ${docker_mode}"
      ;;
  esac

  docker_args+=("${image_name}")

  run_step "${action}" "create_container" create_container_if_missing "${container_name}" "${docker_args[@]}"
  run_step "${action}" "start_container" start_container_if_needed "${container_name}"
  run_step "${action}" "verify_running" wait_for_container_running "${container_name}" 20
  if [[ -n "${gateway_network_name}" && "${gateway_network_name}" != "${network_name}" ]]; then
    run_step "${action}" "connect_gateway_network" connect_container_to_network_if_needed "${container_name}" "${gateway_network_name}"
  fi
  run_step "${action}" "refresh_bootstrap" refresh_container_bootstrap "${container_name}" "${api_key}" "${api_base}" "/workspace" "${gateway_admin_origin}"
  if [[ -n "${api_key}" && -n "${api_base}" ]]; then
    run_step "${action}" "verify_gateway_access" verify_gateway_models_access "${container_name}" "${api_base}" "${api_key}"
  fi
  if [[ -n "${gateway_admin_origin}" ]]; then
    run_step "${action}" "verify_gateway_admin" verify_gateway_admin_access "${container_name}" "${gateway_admin_origin}"
  fi
  run_step "${action}" "verify_exec" docker exec "${container_name}" bash -lc "command -v codex >/dev/null && test -d /root/.agents/skills/coding-core && test -d /root/.agents/skills/ui-ux-pro-max && test -d /root/.codex/skills/coding-core && test -d /root/.codex/skills/ui-ux-pro-max && test \"\$(pwd)\" = \"/workspace\""
  run_step "${action}" "verify_skill_frontmatter" docker exec "${container_name}" bash -lc "python3 - <<'PY'
from pathlib import Path

roots = [
    Path('/root/.agents/skills'),
    Path('/root/.codex/skills'),
]
for root in roots:
    for skill_file in sorted(root.rglob('SKILL.md')):
        lines = skill_file.read_text(encoding='utf-8').splitlines()
        if len(lines) < 3 or lines[0].strip() != '---':
            raise SystemExit(f'invalid skill frontmatter: {skill_file}')
        try:
            closing_index = next(index for index, line in enumerate(lines[1:], start=1) if line.strip() == '---')
        except StopIteration as exc:
            raise SystemExit(f'missing frontmatter terminator: {skill_file}') from exc
        if closing_index < 2:
            raise SystemExit(f'invalid frontmatter block: {skill_file}')
PY"
  run_step "${action}" "verify_ui_skill" docker exec "${container_name}" bash -lc "python3 /root/.agents/skills/ui-ux-pro-max/scripts/search.py 'saas dashboard' --domain style >/dev/null"

  case "${docker_mode}" in
    isolated)
      run_step "${action}" "verify_nested_docker" docker exec "${container_name}" bash -lc "docker version >/dev/null 2>&1 && docker info >/dev/null 2>&1"
      ;;
    host)
      run_step "${action}" "verify_nested_docker" docker exec "${container_name}" bash -lc "docker version >/dev/null 2>&1"
      ;;
  esac

  finish_action "${action}"
  log "容器已创建并启动: ${container_name}"
  log "进入容器: docker exec -it ${container_name} bash"
  if [[ -n "${gateway_admin_origin}" ]]; then
    log "已为容器预置网关远端 app-server: ${gateway_admin_origin}"
  fi
}

deploy_project_action() {
  local action="deploy_project"
  local repo_url repo_ref deploy_dir stack_name compose_file
  local web_port data_port admin_port browser_port postgres_port redis_port
  local web_url admin_url web_probe_url admin_probe_url

  repo_url="$(prompt_default '请输入远端仓库 URL' "$(state_get_or_default 'LAST_DEPLOY_REPO_URL' "${DEFAULT_REPO_URL}")")"
  repo_ref="$(prompt_default '请输入远端分支或引用' "$(state_get_or_default 'LAST_DEPLOY_REPO_REF' "${DEFAULT_REPO_REF}")")"
  deploy_dir="$(prompt_default '请输入部署目录' "$(state_get_or_default 'LAST_DEPLOY_DIR' "$(default_deploy_dir)")")"
  stack_name="$(prompt_default '请输入部署栈名称' "$(state_get_or_default 'LAST_DEPLOY_STACK_NAME' 'cmgrd')")"
  activate_action_context "${action}" "repo=${repo_url}|ref=${repo_ref}|dir=${deploy_dir}|stack=${stack_name}"

  state_set 'LAST_DEPLOY_REPO_URL' "${repo_url}"
  state_set 'LAST_DEPLOY_REPO_REF' "${repo_ref}"
  state_set 'LAST_DEPLOY_DIR' "${deploy_dir}"
  state_set 'LAST_DEPLOY_STACK_NAME' "${stack_name}"

  run_step "${action}" "prepare_directory" mkdir -p "${deploy_dir}"
  run_step "${action}" "clone_or_update_repo" clone_or_update_repo "${repo_url}" "${repo_ref}" "${deploy_dir}"
  is_current_project_repo "${deploy_dir}" || die "当前远端仓库不是当前项目结构，install.sh 分支 3 只支持当前项目拓扑。"
  compose_file="${deploy_dir}/.install.compose.yml"

  web_port="$(pick_free_port 13100 13199)"
  data_port="$(pick_free_port 18200 18299)"
  admin_port="$(pick_free_port 18300 18399)"
  browser_port="$(pick_free_port 18400 18499)"
  postgres_port="$(pick_free_port 15440 15499)"
  redis_port="$(pick_free_port 16380 16439)"

  web_url="http://127.0.0.1:${web_port}"
  admin_url="http://127.0.0.1:${admin_port}/health"
  web_probe_url="http://127.0.0.1:3000/"
  admin_probe_url="http://127.0.0.1:8081/health"
  run_step "${action}" "write_compose" write_current_project_deploy_compose "${compose_file}" "${stack_name}" "${web_port}" "${data_port}" "${admin_port}" "${browser_port}" "${postgres_port}" "${redis_port}"
  run_step "${action}" "compose_up" compose_up_deploy "${deploy_dir}" "${compose_file}"
  run_step "${action}" "verify_web" wait_for_container_http_ready "CodexManager Web" "${stack_name}-web" "${web_probe_url}" 120 "200 204 301 302 307 308"
  run_step "${action}" "verify_admin" wait_for_container_http_ready "CodexManager Admin" "${stack_name}-server" "${admin_probe_url}" 120 "200"
  finish_action "${action}"

  log "远端仓库部署完成: ${repo_url}@${repo_ref}"
  log "Web: ${web_url}"
  log "Admin Health: ${admin_url}"
}

delete_action() {
  local action="delete_resources"
  local container_name image_name mode network_name volume_name agents_volume_name docker_volume_name

  container_name="$(prompt_default '请输入目标容器名' "$(state_get_or_default 'LAST_CONTAINER_NAME' "$(latest_container_name)")")"
  image_name="$(state_get_or_default 'LAST_IMAGE_NAME' "${DEFAULT_IMAGE_NAME}")"
  mode="$(prompt_default '删除模式: 1=仅容器 2=仅镜像 3=容器+镜像+网络+卷' "1")"
  activate_action_context "${action}" "container=${container_name}|image=${image_name}|mode=${mode}"

  network_name="$(container_network_name "${container_name}")"
  volume_name="$(container_volume_name "${container_name}")"
  agents_volume_name="$(container_agents_volume_name "${container_name}")"
  docker_volume_name="$(container_docker_volume_name "${container_name}")"

  case "${mode}" in
    1)
      run_step "${action}" "remove_container" remove_container_if_exists "${container_name}"
      ;;
    2)
      run_step "${action}" "remove_image" remove_image_if_exists "${image_name}"
      ;;
    3)
      run_step "${action}" "remove_container" remove_container_if_exists "${container_name}"
      run_step "${action}" "remove_network" remove_network_if_exists "${network_name}"
      run_step "${action}" "remove_volume" remove_volume_if_exists "${volume_name}"
      run_step "${action}" "remove_agents_volume" remove_volume_if_exists "${agents_volume_name}"
      run_step "${action}" "remove_docker_volume" remove_volume_if_exists "${docker_volume_name}"
      run_step "${action}" "remove_image" remove_image_if_exists "${image_name}"
      ;;
    *)
      die "未知删除模式: ${mode}"
      ;;
  esac

  finish_action "${action}"
  log "删除操作完成。"
}

show_menu() {
  cat <<'EOF'

==============================
 Codex Lite 安装脚本
==============================
1) 制作轻量镜像
2) 创建并启动容器
3) 拉取远端仓库并部署项目
4) 删除资源
0) 退出

EOF
}

main() {
  need_cmd docker
  need_cmd git
  need_cmd curl
  need_cmd python3
  ensure_state_dir

  while true; do
    show_menu
    read -r -p "请输入选项: " choice
    case "${choice}" in
      1) build_image_action ;;
      2) create_container_action ;;
      3) deploy_project_action ;;
      4) delete_action ;;
      0) exit 0 ;;
      *) warn "无效选项: ${choice}" ;;
    esac
  done
}

main "$@"
