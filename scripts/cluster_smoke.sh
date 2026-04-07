#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_BIN="$ROOT_DIR/target/debug/codex-manager-server"
TMP_DIR="$(mktemp -d)"
A_PID=""
B_PID=""
BROWSER_PID=""
UPSTREAM_PID=""

cleanup() {
  local exit_code=$?
  for pid in "$A_PID" "$B_PID" "$BROWSER_PID" "$UPSTREAM_PID"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ $exit_code -ne 0 ]]; then
    echo "cluster smoke failed" >&2
    for log in "$TMP_DIR"/*.log; do
      [[ -f "$log" ]] || continue
      echo "--- $(basename "$log") ---" >&2
      tail -n 120 "$log" >&2 || true
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
  for _ in $(seq 1 60); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for $url" >&2
  return 1
}

wait_json_assert() {
  local mode="$1"
  local url="$2"
  local payload=""
  for _ in $(seq 1 80); do
    payload="$(curl -fsS "$url" 2>/dev/null || true)"
    if [[ -n "$payload" ]] && json_assert "$mode" "$payload" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  json_assert "$mode" "$payload"
}

json_assert() {
  local mode="$1"
  local payload="$2"
  python3 - "$mode" "$payload" <<'PY'
import json
import sys

mode = sys.argv[1]
payload = json.loads(sys.argv[2])

if mode == "health":
    assert payload["postgresConnected"] is True
    assert payload["redisConnected"] is True
elif mode == "live_dashboard":
    assert payload["counts"]["tenants"] == 1
    assert payload["counts"]["warpAccounts"] == 1
    assert payload["counts"]["activeLeases"] == 1
elif mode == "restored_dashboard":
    assert payload["counts"]["tenants"] == 1
    assert payload["counts"]["warpAccounts"] == 1
    assert payload["counts"]["activeLeases"] == 1
elif mode == "leases":
    principals = {item["principalId"] for item in payload}
    assert "tenant:ops-lab/principal:ops-shell" in principals
elif mode == "incidents":
    assert len(payload) >= 3
    assert payload[0]["routeMode"] == "warp"
else:
    raise AssertionError(f"unknown mode: {mode}")
PY
}

require_cmd cargo
require_cmd curl
require_cmd node
require_cmd pg_isready
require_cmd psql
require_cmd python3
require_cmd redis-cli

PGPASSWORD="${PGPASSWORD:-codex_manager}"
export PGPASSWORD

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
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8") if length else "{}"
        payload = json.loads(body or "{}")
        if self.path != "/v1/responses":
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps({
            "id": "resp_cluster",
            "object": "response",
            "status": "completed",
            "model": payload.get("model"),
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "cluster ok"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 4}
        }).encode("utf-8"))

    def log_message(self, *args):
        return

HTTPServer(("127.0.0.1", 19100), Handler).serve_forever()
PY

python3 "$TMP_DIR/upstream_stub.py" >"$TMP_DIR/upstream.log" 2>&1 &
UPSTREAM_PID=$!

node "$ROOT_DIR/services/browser-assist/server.mjs" >"$TMP_DIR/browser.log" 2>&1 &
BROWSER_PID=$!

CMGR_INSTANCE_ID=cmgr-a \
CMGR_SERVER_DATA_PORT=8080 \
CMGR_SERVER_ADMIN_PORT=8081 \
"$SERVER_BIN" >"$TMP_DIR/server-a.log" 2>&1 &
A_PID=$!

wait_http "http://127.0.0.1:8081/health"
HEALTH_A="$(curl -fsS http://127.0.0.1:8081/health)"
json_assert health "$HEALTH_A"

CMGR_INSTANCE_ID=cmgr-b \
CMGR_SERVER_DATA_PORT=9080 \
CMGR_SERVER_ADMIN_PORT=9081 \
"$SERVER_BIN" >"$TMP_DIR/server-b.log" 2>&1 &
B_PID=$!

wait_http "http://127.0.0.1:9081/health"
HEALTH_B="$(curl -fsS http://127.0.0.1:9081/health)"
json_assert health "$HEALTH_B"

TENANT_JSON="$(curl -fsS -X POST http://127.0.0.1:8081/api/v1/tenants \
  -H 'Content-Type: application/json' \
  --data '{"slug":"ops-lab","name":"Ops Lab"}')"
TENANT_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$TENANT_JSON")"

API_KEY_JSON="$(curl -fsS -X POST http://127.0.0.1:8081/api/v1/gateway/api-keys \
  -H 'Content-Type: application/json' \
  --data "{\"tenantId\":\"$TENANT_ID\",\"name\":\"Ops Key\"}")"
API_KEY_TOKEN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["token"])' "$API_KEY_JSON")"

ACCOUNT_JSON="$(curl -fsS -X POST http://127.0.0.1:8081/api/v1/accounts/import \
  -H 'Content-Type: application/json' \
  --data "{\"tenantId\":\"$TENANT_ID\",\"label\":\"Ops Upstream\",\"models\":[\"gpt-5.4\"],\"baseUrl\":\"http://127.0.0.1:19100/v1\",\"bearerToken\":\"ops-secret\",\"chatgptAccountId\":\"acct-ops\"}")"
ACCOUNT_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$ACCOUNT_JSON")"

curl -fsS -X POST http://127.0.0.1:8080/v1/responses \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: ops-shell' \
  --data '{"model":"gpt-5.4","input":"create distributed lease"}' >/dev/null

for _ in 1 2 3; do
  curl -fsS -X POST "http://127.0.0.1:8081/api/v1/accounts/$ACCOUNT_ID/route-events" \
    -H 'Content-Type: application/json' \
    --data '{"mode":"direct","kind":"cf_hit"}' >/dev/null
done

sleep 1

wait_json_assert live_dashboard "http://127.0.0.1:9081/api/v1/dashboard"
wait_json_assert leases "http://127.0.0.1:9081/api/v1/leases"
wait_json_assert incidents "http://127.0.0.1:9081/api/v1/cf-incidents"

kill "$B_PID"
wait "$B_PID" || true
B_PID=""

CMGR_INSTANCE_ID=cmgr-b \
CMGR_SERVER_DATA_PORT=9080 \
CMGR_SERVER_ADMIN_PORT=9081 \
"$SERVER_BIN" >"$TMP_DIR/server-b.log" 2>&1 &
B_PID=$!

wait_http "http://127.0.0.1:9081/health"
wait_json_assert restored_dashboard "http://127.0.0.1:9081/api/v1/dashboard"
wait_json_assert leases "http://127.0.0.1:9081/api/v1/leases"
python3 - "$(psql -h 127.0.0.1 -U codex_manager -d codex_manager -Atqc 'select count(*) from conversation_contexts')" <<'PY'
import sys
assert int(sys.argv[1]) >= 1
PY

echo "cluster smoke passed"
