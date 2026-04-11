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

persist_gateway_remote_settings() {
  if [[ -n "${CODEX_GATEWAY_ADMIN_ORIGIN:-}" ]]; then
    printf '%s\n' "${CODEX_GATEWAY_ADMIN_ORIGIN}" > "${CODEX_HOME}/gateway-admin-origin"
    chmod 600 "${CODEX_HOME}/gateway-admin-origin" >/dev/null 2>&1 || true
  fi
  if [[ -n "${CMGR_CODEX_REMOTE_LOOPBACK_PORT:-}" ]]; then
    printf '%s\n' "${CMGR_CODEX_REMOTE_LOOPBACK_PORT}" > "${CODEX_HOME}/remote-loopback-port"
    chmod 600 "${CODEX_HOME}/remote-loopback-port" >/dev/null 2>&1 || true
  fi
}

install_codex_remote_helper() {
  cat > /usr/local/bin/cmgr-codex-remote <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail

REAL_BIN="${CODEX_REAL_BIN:-/usr/local/bin/codex-real}"
CODEX_HOME="${CODEX_HOME:-/root/.codex}"
ADMIN_ORIGIN_FILE="${CODEX_HOME}/gateway-admin-origin"
LOOPBACK_PORT_FILE="${CODEX_HOME}/remote-loopback-port"
PID_FILE="${CODEX_HOME}/remote-app-server-forwarder.pid"
LOG_FILE="${CODEX_HOME}/remote-app-server-forwarder.log"

warn() {
  printf '[cmgr-codex] %s\n' "$*" >&2
}

read_api_key() {
  if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    printf '%s' "${OPENAI_API_KEY}"
    return 0
  fi

  python3 - "${CODEX_HOME}/auth.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
if not path.exists():
    raise SystemExit(1)
data = json.loads(path.read_text(encoding="utf-8"))
token = (data.get("OPENAI_API_KEY") or "").strip()
if not token:
    raise SystemExit(1)
print(token, end="")
PY
}

read_admin_origin() {
  if [[ -n "${CODEX_GATEWAY_ADMIN_ORIGIN:-}" ]]; then
    printf '%s' "${CODEX_GATEWAY_ADMIN_ORIGIN}"
    return 0
  fi
  [[ -f "${ADMIN_ORIGIN_FILE}" ]] || return 1
  tr -d '\r\n' < "${ADMIN_ORIGIN_FILE}"
}

read_loopback_port() {
  if [[ -n "${CMGR_CODEX_REMOTE_LOOPBACK_PORT:-}" ]]; then
    printf '%s' "${CMGR_CODEX_REMOTE_LOOPBACK_PORT}"
    return 0
  fi
  if [[ -f "${LOOPBACK_PORT_FILE}" ]]; then
    tr -d '\r\n' < "${LOOPBACK_PORT_FILE}"
    return 0
  fi
  printf '19081'
}

has_remote_flag() {
  local arg
  for arg in "$@"; do
    case "${arg}" in
      --remote|--remote=*|--remote-auth-token-env|--remote-auth-token-env=*)
        return 0
        ;;
    esac
  done
  return 1
}

remote_mode_for_args() {
  local arg first_positional=""
  for arg in "$@"; do
    case "${arg}" in
      --)
        break
        ;;
      -*)
        continue
        ;;
      *)
        first_positional="${arg}"
        break
        ;;
    esac
  done

  case "${first_positional}" in
    "")
      printf '%s' "root"
      ;;
    resume|fork)
      printf '%s' "subcommand"
      ;;
    exec|review|login|logout|mcp|mcp-server|app-server|completion|sandbox|debug|execpolicy|apply|cloud|cloud-tasks|responses-api-proxy|stdio-to-uds|exec-server|features)
      printf '%s' "disabled"
      ;;
    *)
      printf '%s' "root"
      ;;
  esac
}

parse_admin_target() {
  python3 - "$1" <<'PY'
import sys
from urllib.parse import urlparse

parsed = urlparse(sys.argv[1])
host = parsed.hostname or ""
if not host:
    raise SystemExit(1)
port = parsed.port
if port is None:
    port = 443 if parsed.scheme == "https" else 80
print(f"{host}\t{port}", end="")
PY
}

ensure_forwarder() {
  local admin_origin="$1" loopback_port="$2" target_host target_port

  mkdir -p "${CODEX_HOME}"
  if [[ -f "${PID_FILE}" ]] && kill -0 "$(cat "${PID_FILE}")" >/dev/null 2>&1; then
    return 0
  fi
  rm -f "${PID_FILE}"

  IFS=$'\t' read -r target_host target_port <<< "$(parse_admin_target "${admin_origin}")"
  nohup socat "TCP-LISTEN:${loopback_port},bind=127.0.0.1,reuseaddr,fork" "TCP:${target_host}:${target_port}" > "${LOG_FILE}" 2>&1 &
  echo $! > "${PID_FILE}"
  sleep 0.2
  kill -0 "$(cat "${PID_FILE}")" >/dev/null 2>&1
}

request_remote_auth_token() {
  local admin_origin="$1" api_key="$2" session_payload token
  session_payload="${CODEX_REMOTE_APP_SESSION_PAYLOAD:-{}}"
  token="$(
    curl -fsS \
      -H "Authorization: Bearer ${api_key}" \
      -H "Content-Type: application/json" \
      --data "${session_payload}" \
      "${admin_origin%/}/api/v1/codex/app-session" \
      | jq -r '.remote_app_server_auth_token // .remoteAppServerAuthToken // empty'
  )"
  [[ -n "${token}" ]] || return 1
  printf '%s' "${token}"
}

main() {
  local mode admin_origin api_key loopback_port remote_url remote_token

  [[ -x "${REAL_BIN}" ]] || exec "${REAL_BIN}" "$@"

  mode="$(remote_mode_for_args "$@")"
  if [[ "${mode}" == "disabled" ]] || has_remote_flag "$@"; then
    exec "${REAL_BIN}" "$@"
  fi

  admin_origin="$(read_admin_origin || true)"
  api_key="$(read_api_key || true)"
  if [[ -z "${admin_origin}" || -z "${api_key}" ]]; then
    exec "${REAL_BIN}" "$@"
  fi

  loopback_port="$(read_loopback_port)"
  if ! ensure_forwarder "${admin_origin}" "${loopback_port}"; then
    warn "无法建立本地 app-server 转发，回退到普通 Codex 连接。"
    exec "${REAL_BIN}" "$@"
  fi

  remote_token="$(request_remote_auth_token "${admin_origin}" "${api_key}" || true)"
  if [[ -z "${remote_token}" ]]; then
    warn "无法获取远端 app-server 会话，回退到普通 Codex 连接。"
    exec "${REAL_BIN}" "$@"
  fi

  export CODEX_REMOTE_AUTH_TOKEN="${remote_token}"
  remote_url="ws://127.0.0.1:${loopback_port}/api/v1/codex/app-server/ws"

  if [[ "${mode}" == "subcommand" ]]; then
    exec "${REAL_BIN}" "$1" --remote "${remote_url}" --remote-auth-token-env CODEX_REMOTE_AUTH_TOKEN "${@:2}"
  fi
  exec "${REAL_BIN}" --remote "${remote_url}" --remote-auth-token-env CODEX_REMOTE_AUTH_TOKEN "$@"
}

main "$@"
EOF
  chmod +x /usr/local/bin/cmgr-codex-remote
}

install_codex_wrapper() {
  local codex_bin="/usr/local/bin/codex" real_bin="/usr/local/bin/codex-real"

  if [[ -x "${codex_bin}" && ! -x "${real_bin}" ]]; then
    mv "${codex_bin}" "${real_bin}"
  fi
  [[ -x "${real_bin}" ]] || return 0

  install_codex_remote_helper
  cat > "${codex_bin}" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
exec /usr/local/bin/cmgr-codex-remote "$@"
EOF
  chmod +x "${codex_bin}"
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

persist_gateway_remote_settings
install_codex_wrapper

if [[ -d "${CODEX_DEFAULT_PROJECT}" ]]; then
  mkdir -p "${CODEX_DEFAULT_PROJECT}/.codex"
fi

exec "$@"
