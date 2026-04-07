#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_BIN="$ROOT_DIR/target/debug/codex-manager-server"
TMP_DIR="$(mktemp -d)"
SERVER_PID=""
UPSTREAM_PID=""

cleanup() {
  local exit_code=$?
  for pid in "$SERVER_PID" "$UPSTREAM_PID"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ $exit_code -ne 0 ]]; then
    echo "edge smoke failed" >&2
    for log in "$TMP_DIR"/*.log; do
      [[ -f "$log" ]] || continue
      echo "--- $(basename "$log") ---" >&2
      tail -n 160 "$log" >&2 || true
    done
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

wait_http() {
  local url="$1"
  for _ in $(seq 1 80); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  echo "timed out waiting for $url" >&2
  return 1
}

pick_port() {
  python3 <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

json_field() {
  python3 -c 'import json,sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$1" "$2"
}

require_cmd cargo
require_cmd curl
require_cmd python3
require_cmd pg_isready
require_cmd psql
require_cmd redis-cli

PGPASSWORD="${PGPASSWORD:-codex_manager}"
export PGPASSWORD

UPSTREAM_PORT="${UPSTREAM_PORT:-$(pick_port)}"
DATA_PORT="${DATA_PORT:-$(pick_port)}"
ADMIN_PORT="${ADMIN_PORT:-$(pick_port)}"

pg_isready -h 127.0.0.1 -p 5432 >/dev/null
redis-cli ping >/dev/null

psql -h 127.0.0.1 -U codex_manager -d codex_manager <<'SQL' >/dev/null
DROP TABLE IF EXISTS cli_leases CASCADE;
DROP TABLE IF EXISTS account_route_states CASCADE;
DROP TABLE IF EXISTS cf_incidents CASCADE;
DROP TABLE IF EXISTS upstream_credentials CASCADE;
DROP TABLE IF EXISTS upstream_accounts CASCADE;
DROP TABLE IF EXISTS gateway_api_keys CASCADE;
DROP TABLE IF EXISTS tenants CASCADE;
DROP TABLE IF EXISTS conversation_contexts CASCADE;
DROP TABLE IF EXISTS cache_metrics CASCADE;
SQL

(cd "$ROOT_DIR" && cargo build -p codex-manager-server >/dev/null)

cat >"$TMP_DIR/upstream_stub.py" <<'PY'
import json
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT = int(os.environ["UPSTREAM_PORT"])

def json_response(handler, status, payload, extra_headers=None):
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    if extra_headers:
        for name, value in extra_headers.items():
            handler.send_header(name, value)
    handler.end_headers()
    handler.wfile.write(json.dumps(payload).encode("utf-8"))

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/v1/responses":
            self.send_response(404)
            self.end_headers()
            return

        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8") if length else "{}"
        payload = json.loads(body or "{}")
        stream = bool(payload.get("stream"))
        auth = self.headers.get("Authorization", "")
        token = auth.replace("Bearer ", "", 1)
        wants_tools = bool(payload.get("tools"))

        if token == "quota-low-secret":
            if stream:
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                frames = [
                    "event: response.created\n",
                    'data: {"type":"response.created","response":{"id":"resp_quota_fail","model":"gpt-5.4"}}\n\n',
                    "event: response.failed\n",
                    'data: {"type":"response.failed","response":{"id":"resp_quota_fail","status":"failed","error":{"code":"insufficient_quota","message":"You exceeded your current quota."}}}\n\n',
                ]
                for frame in frames:
                    self.wfile.write(frame.encode("utf-8"))
                return
            json_response(
                self,
                200,
                {
                    "type": "response.failed",
                    "response": {
                        "id": "resp_quota_fail",
                        "status": "failed",
                        "error": {
                            "code": "insufficient_quota",
                            "message": "You exceeded your current quota."
                        }
                    }
                },
            )
            return

        if token == "drift-secret":
            extra_headers = {"openai-model": "gpt-4.1-mini"}
            if stream:
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("openai-model", "gpt-4.1-mini")
                self.end_headers()
                frames = [
                    "event: response.created\n",
                    'data: {"type":"response.created","response":{"id":"resp_drift","model":"gpt-4.1-mini"}}\n\n',
                    "event: response.output_text.delta\n",
                    'data: {"type":"response.output_text.delta","delta":"wrong model"}\n\n',
                    "event: response.completed\n",
                    'data: {"type":"response.completed","response":{"id":"resp_drift","status":"completed","model":"gpt-4.1-mini"}}\n\n',
                ]
                for frame in frames:
                    self.wfile.write(frame.encode("utf-8"))
                return
            json_response(
                self,
                200,
                {
                    "id": "resp_drift",
                    "object": "response",
                    "status": "completed",
                    "model": "gpt-4.1-mini",
                    "output": [{
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "wrong model"}]
                    }]
                },
                extra_headers,
            )
            return

        content_text = "healthy unary ok"
        stream_text = "healthy stream ok"
        if wants_tools:
            if stream:
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                frames = [
                    "event: response.created\n",
                    'data: {"type":"response.created","response":{"id":"resp_tool_ok","model":"gpt-5.4"}}\n\n',
                    "event: response.output_item.added\n",
                    'data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_shell_1","name":"shell"}}\n\n',
                    "event: response.function_call_arguments.done\n",
                    'data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\\\"command\\\":\\\"echo ok\\\"}"}\n\n',
                    "event: response.completed\n",
                    'data: {"type":"response.completed","response":{"id":"resp_tool_ok","status":"completed","model":"gpt-5.4"}}\n\n',
                ]
                for frame in frames:
                    self.wfile.write(frame.encode("utf-8"))
                return
            json_response(
                self,
                200,
                {
                    "id": "resp_tool_ok",
                    "object": "response",
                    "status": "completed",
                    "model": payload.get("model"),
                    "output": [{
                        "type": "function_call",
                        "call_id": "call_shell_1",
                        "name": "shell",
                        "arguments": "{\"command\":\"echo ok\"}"
                    }]
                },
            )
            return

        if stream:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            frames = [
                "event: response.created\n",
                'data: {"type":"response.created","response":{"id":"resp_ok","model":"gpt-5.4"}}\n\n',
                "event: response.output_text.delta\n",
                f'data: {json.dumps({"type":"response.output_text.delta","delta":stream_text})}\n\n',
                "event: response.completed\n",
                'data: {"type":"response.completed","response":{"id":"resp_ok","status":"completed","model":"gpt-5.4"}}\n\n',
            ]
            for frame in frames:
                self.wfile.write(frame.encode("utf-8"))
            return

        json_response(
            self,
            200,
            {
                "id": "resp_ok",
                "object": "response",
                "status": "completed",
                "model": payload.get("model"),
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": content_text}]
                }]
            },
        )

    def log_message(self, *args):
        return

HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
PY

UPSTREAM_PORT="$UPSTREAM_PORT" python3 "$TMP_DIR/upstream_stub.py" >"$TMP_DIR/upstream.log" 2>&1 &
UPSTREAM_PID=$!

CMGR_INSTANCE_ID=cmgr-edge-smoke \
CMGR_SERVER_DATA_PORT="$DATA_PORT" \
CMGR_SERVER_ADMIN_PORT="$ADMIN_PORT" \
"$SERVER_BIN" >"$TMP_DIR/server.log" 2>&1 &
SERVER_PID=$!

wait_http "http://127.0.0.1:$ADMIN_PORT/health"

create_tenant() {
  curl -fsS -X POST "http://127.0.0.1:$ADMIN_PORT/api/v1/tenants" \
    -H 'Content-Type: application/json' \
    --data "{\"slug\":\"$1\",\"name\":\"$2\"}"
}

create_api_key() {
  curl -fsS -X POST "http://127.0.0.1:$ADMIN_PORT/api/v1/gateway/api-keys" \
    -H 'Content-Type: application/json' \
    --data "{\"tenantId\":\"$1\",\"name\":\"$2\"}"
}

import_account() {
  curl -fsS -X POST "http://127.0.0.1:$ADMIN_PORT/api/v1/accounts/import" \
    -H 'Content-Type: application/json' \
    --data "$1"
}

quota_tenant_json="$(create_tenant quota-edge 'Quota Edge')"
quota_tenant_id="$(json_field "$quota_tenant_json" id)"
quota_key_json="$(create_api_key "$quota_tenant_id" 'Quota Key')"
quota_key_token="$(json_field "$quota_key_json" token)"

quota_fail_json="$(import_account "{\"tenantId\":\"$quota_tenant_id\",\"label\":\"A-Quota-Low\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.80,\"quotaHeadroom5h\":0.20,\"quotaHeadroom7d\":0.85,\"healthScore\":0.99,\"egressStability\":0.99,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"quota-low-secret\"}")"
quota_fail_id="$(json_field "$quota_fail_json" id)"
quota_backup_json="$(import_account "{\"tenantId\":\"$quota_tenant_id\",\"label\":\"B-Healthy-Backup\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.95,\"quotaHeadroom5h\":0.95,\"quotaHeadroom7d\":0.95,\"healthScore\":0.10,\"egressStability\":0.10,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"healthy-secret\"}")"
quota_backup_id="$(json_field "$quota_backup_json" id)"

quota_resp="$(curl -fsS -X POST "http://127.0.0.1:$DATA_PORT/v1/responses" \
  -H "Authorization: Bearer $quota_key_token" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: quota-cli' \
  --data '{"model":"gpt-5.4","input":"hello quota","stream":false}')"

quota_leases="$(curl -fsS "http://127.0.0.1:$ADMIN_PORT/api/v1/leases")"
quota_incidents="$(curl -fsS "http://127.0.0.1:$ADMIN_PORT/api/v1/cf-incidents")"
quota_accounts="$(curl -fsS "http://127.0.0.1:$ADMIN_PORT/api/v1/accounts")"

chat_tenant_json="$(create_tenant chat-edge 'Chat Edge')"
chat_tenant_id="$(json_field "$chat_tenant_json" id)"
chat_key_json="$(create_api_key "$chat_tenant_id" 'Chat Key')"
chat_key_token="$(json_field "$chat_key_json" token)"
chat_fail_json="$(import_account "{\"tenantId\":\"$chat_tenant_id\",\"label\":\"A-Chat-Quota-Low\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.80,\"quotaHeadroom5h\":0.20,\"quotaHeadroom7d\":0.85,\"healthScore\":0.99,\"egressStability\":0.99,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"quota-low-secret\"}")"
chat_fail_id="$(json_field "$chat_fail_json" id)"
chat_backup_json="$(import_account "{\"tenantId\":\"$chat_tenant_id\",\"label\":\"B-Chat-Backup\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.95,\"quotaHeadroom5h\":0.95,\"quotaHeadroom7d\":0.95,\"healthScore\":0.10,\"egressStability\":0.10,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"healthy-secret\"}")"
chat_backup_id="$(json_field "$chat_backup_json" id)"

chat_resp="$(curl -fsS -X POST "http://127.0.0.1:$DATA_PORT/v1/chat/completions" \
  -H "Authorization: Bearer $chat_key_token" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: chat-cli' \
  --data '{"model":"gpt-5.4","messages":[{"role":"user","content":"hello quota chat"}],"stream":false}')"

chat_leases="$(curl -fsS "http://127.0.0.1:$ADMIN_PORT/api/v1/leases")"

stream_tenant_json="$(create_tenant stream-edge 'Stream Edge')"
stream_tenant_id="$(json_field "$stream_tenant_json" id)"
stream_key_json="$(create_api_key "$stream_tenant_id" 'Stream Key')"
stream_key_token="$(json_field "$stream_key_json" token)"
stream_fail_json="$(import_account "{\"tenantId\":\"$stream_tenant_id\",\"label\":\"A-Stream-Quota-Low\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.80,\"quotaHeadroom5h\":0.20,\"quotaHeadroom7d\":0.85,\"healthScore\":0.99,\"egressStability\":0.99,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"quota-low-secret\"}")"
stream_fail_id="$(json_field "$stream_fail_json" id)"
stream_backup_json="$(import_account "{\"tenantId\":\"$stream_tenant_id\",\"label\":\"B-Stream-Backup\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.95,\"quotaHeadroom5h\":0.95,\"quotaHeadroom7d\":0.95,\"healthScore\":0.10,\"egressStability\":0.10,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"healthy-secret\"}")"
stream_backup_id="$(json_field "$stream_backup_json" id)"

stream_resp="$(curl -fsS -N -X POST "http://127.0.0.1:$DATA_PORT/v1/responses" \
  -H "Authorization: Bearer $stream_key_token" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: stream-cli' \
  --data '{"model":"gpt-5.4","input":"hello quota stream","stream":true}')"
stream_leases="$(curl -fsS "http://127.0.0.1:$ADMIN_PORT/api/v1/leases")"

wait_tenant_json="$(create_tenant wait-edge 'Wait Edge')"
wait_tenant_id="$(json_field "$wait_tenant_json" id)"
wait_key_json="$(create_api_key "$wait_tenant_id" 'Wait Key')"
wait_key_token="$(json_field "$wait_key_json" token)"
wait_fail_json="$(import_account "{\"tenantId\":\"$wait_tenant_id\",\"label\":\"A-Only-Quota\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.82,\"quotaHeadroom5h\":0.21,\"quotaHeadroom7d\":0.82,\"healthScore\":0.99,\"egressStability\":0.99,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"quota-low-secret\"}")"
wait_fail_id="$(json_field "$wait_fail_json" id)"

curl -sS -o "$TMP_DIR/wait_response.json" -w '%{http_code}' \
  -X POST "http://127.0.0.1:$DATA_PORT/v1/responses" \
  -H "Authorization: Bearer $wait_key_token" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: wait-cli' \
  --data '{"model":"gpt-5.4","input":"hello wait","stream":false}' >"$TMP_DIR/wait_status.txt"

drift_tenant_json="$(create_tenant drift-edge 'Drift Edge')"
drift_tenant_id="$(json_field "$drift_tenant_json" id)"
drift_key_json="$(create_api_key "$drift_tenant_id" 'Drift Key')"
drift_key_token="$(json_field "$drift_key_json" token)"

drift_fail_json="$(import_account "{\"tenantId\":\"$drift_tenant_id\",\"label\":\"A-Model-Drift\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.80,\"quotaHeadroom5h\":0.25,\"quotaHeadroom7d\":0.90,\"healthScore\":0.99,\"egressStability\":0.99,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"drift-secret\"}")"
drift_fail_id="$(json_field "$drift_fail_json" id)"
drift_backup_json="$(import_account "{\"tenantId\":\"$drift_tenant_id\",\"label\":\"B-Drift-Backup\",\"models\":[\"gpt-5.4\"],\"quotaHeadroom\":0.95,\"quotaHeadroom5h\":0.95,\"quotaHeadroom7d\":0.95,\"healthScore\":0.10,\"egressStability\":0.10,\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"healthy-secret\"}")"
drift_backup_id="$(json_field "$drift_backup_json" id)"

drift_resp="$(curl -fsS -X POST "http://127.0.0.1:$DATA_PORT/v1/responses" \
  -H "Authorization: Bearer $drift_key_token" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: drift-cli' \
  --data '{"model":"gpt-5.4","input":"hello drift","stream":false}')"

python3 - "$quota_resp" "$quota_leases" "$quota_incidents" "$quota_accounts" "$quota_fail_id" "$quota_backup_id" "$chat_resp" "$chat_leases" "$chat_fail_id" "$chat_backup_id" "$stream_resp" "$stream_leases" "$stream_fail_id" "$stream_backup_id" "$drift_resp" "$drift_fail_id" "$drift_backup_id" "$(cat "$TMP_DIR/wait_status.txt")" "$(cat "$TMP_DIR/wait_response.json")" <<'PY'
import json
import sys

quota_resp = json.loads(sys.argv[1])
quota_leases = json.loads(sys.argv[2])
quota_incidents = json.loads(sys.argv[3])
quota_accounts = json.loads(sys.argv[4])
quota_fail_id = sys.argv[5]
quota_backup_id = sys.argv[6]
chat_resp = json.loads(sys.argv[7])
chat_leases = json.loads(sys.argv[8])
chat_fail_id = sys.argv[9]
chat_backup_id = sys.argv[10]
stream_resp = sys.argv[11]
stream_leases = json.loads(sys.argv[12])
stream_fail_id = sys.argv[13]
stream_backup_id = sys.argv[14]
drift_resp = json.loads(sys.argv[15])
drift_fail_id = sys.argv[16]
drift_backup_id = sys.argv[17]
wait_status = sys.argv[18]
wait_resp = json.loads(sys.argv[19])

assert quota_resp["status"] == "completed"
assert "healthy unary ok" in json.dumps(quota_resp)
assert "insufficient_quota" not in json.dumps(quota_resp)

lease_map = {item["principalId"]: item["accountId"] for item in quota_leases}
assert lease_map["tenant:quota-edge/principal:quota-cli"] == quota_backup_id

assert any(item["severity"] == "quota" and item["accountId"] == quota_fail_id for item in quota_incidents)
quota_fail_summary = next(item for item in quota_accounts if item["id"] == quota_fail_id)
assert quota_fail_summary["nearQuotaGuardEnabled"] is True
assert abs(quota_fail_summary["quotaHeadroom5h"] - 0.20) < 1e-9

assert chat_resp["object"] == "chat.completion"
assert chat_resp["choices"][0]["message"]["content"] == "healthy unary ok"
assert "insufficient_quota" not in json.dumps(chat_resp)
chat_lease_map = {item["principalId"]: item["accountId"] for item in chat_leases}
assert chat_lease_map["tenant:chat-edge/principal:chat-cli"] == chat_backup_id

assert "healthy stream ok" in stream_resp
assert "insufficient_quota" not in stream_resp
assert "usage_limit_reached" not in stream_resp
stream_lease_map = {item["principalId"]: item["accountId"] for item in stream_leases}
assert stream_lease_map["tenant:stream-edge/principal:stream-cli"] == stream_backup_id

assert wait_status == "503"
assert wait_resp["error"]["type"] == "server_busy"
assert "insufficient_quota" not in json.dumps(wait_resp)

assert drift_resp["status"] == "completed"
assert "healthy unary ok" in json.dumps(drift_resp)
assert "gpt-4.1-mini" not in json.dumps(drift_resp)
PY

echo "edge smoke passed"
