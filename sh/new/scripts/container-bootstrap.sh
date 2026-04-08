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
lines.append(f'openai_base_url = "{os.environ["CODEX_ACTIVE_API_BASE"]}"')
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
