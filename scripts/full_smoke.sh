#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_BIN="$ROOT_DIR/target/debug/codex-manager-server"
TMP_DIR="$(mktemp -d)"
SERVER_PID=""
BROWSER_PID=""
UPSTREAM_PID=""
WEB_PID=""

cleanup() {
  local exit_code=$?
  for pid in "$SERVER_PID" "$BROWSER_PID" "$UPSTREAM_PID" "$WEB_PID"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ $exit_code -ne 0 ]]; then
    echo "full smoke failed" >&2
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

json_eval() {
  local payload="$1"
  local script="$2"
  python3 - "$payload" "$script" <<'PY'
import json
import sys

payload = json.loads(sys.argv[1])
script = sys.argv[2]
ns = {"payload": payload}
exec(script, ns, ns)
PY
}

require_cmd cargo
require_cmd curl
require_cmd node
require_cmd npm
require_cmd python3
require_cmd pg_isready
require_cmd psql
require_cmd redis-cli

PGPASSWORD="${PGPASSWORD:-codex_manager}"
export PGPASSWORD

UPSTREAM_PORT="${UPSTREAM_PORT:-$(pick_port)}"
BROWSER_PORT="${BROWSER_PORT:-$(pick_port)}"
DATA_PORT="${DATA_PORT:-$(pick_port)}"
ADMIN_PORT="${ADMIN_PORT:-$(pick_port)}"
WEB_PORT="${WEB_PORT:-$(pick_port)}"

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
(cd "$ROOT_DIR" && npm run build:web >/dev/null)

cat >"$TMP_DIR/upstream_stub.py" <<'PY'
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/openai-login":
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.end_headers()
        self.wfile.write(b"""<!doctype html>
<html>
  <body>
    <button id="login-button">Log in</button>
    <form id="auth-flow" style="display:none">
      <input type="email" name="username" autocomplete="username" />
      <button type="button" id="email-continue">Continue</button>
      <input type="password" name="password" autocomplete="current-password" style="display:none" />
      <button type="button" id="password-continue" style="display:none">Continue</button>
      <input name="otp" autocomplete="one-time-code" style="display:none" />
      <button type="button" id="otp-continue" style="display:none">Verify</button>
    </form>
    <script>
      const loginButton = document.getElementById("login-button");
      const authFlow = document.getElementById("auth-flow");
      const emailContinue = document.getElementById("email-continue");
      const passwordInput = document.querySelector('input[type="password"]');
      const passwordContinue = document.getElementById("password-continue");
      const otpInput = document.querySelector('input[name="otp"]');
      const otpContinue = document.getElementById("otp-continue");
      loginButton.addEventListener("click", () => {
        authFlow.style.display = "block";
      });
      emailContinue.addEventListener("click", () => {
        document.body.dataset.email = document.querySelector('input[type="email"]').value;
        emailContinue.style.display = "none";
        passwordInput.style.display = "block";
        passwordContinue.style.display = "inline-block";
      });
      passwordContinue.addEventListener("click", () => {
        document.body.dataset.password = passwordInput.value;
        passwordContinue.style.display = "none";
        otpInput.style.display = "block";
        otpContinue.style.display = "inline-block";
      });
      otpContinue.addEventListener("click", () => {
        document.body.dataset.otp = otpInput.value;
        localStorage.setItem("cmgr-openai-auth", "ok");
        const node = document.createElement("div");
        node.dataset.testid = "openai-authenticated";
        node.textContent = "authenticated";
        document.body.appendChild(node);
        document.title = "OpenAI Workspace";
      });
    </script>
  </body>
</html>""")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length).decode("utf-8") if length else "{}"
        payload = json.loads(body or "{}")
        if self.path != "/v1/responses":
            self.send_response(404)
            self.end_headers()
            return
        tools_requested = bool(payload.get("tools"))
        if payload.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            if tools_requested:
                parts = [
                    "event: response.created\n",
                    'data: {"type":"response.created","response":{"id":"resp_tool_alpha","model":"%s"}}\n\n' % payload.get("model", "unknown"),
                    "event: response.output_item.added\n",
                    'data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_shell_1","name":"shell"}}\n\n',
                    "event: response.function_call_arguments.delta\n",
                    'data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\\\"command\\\":\\\"echo "}\n\n',
                    "event: response.function_call_arguments.done\n",
                    'data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\\\"command\\\":\\\"echo alpha\\\"}"}\n\n',
                    "event: response.output_item.done\n",
                    'data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","call_id":"call_shell_1","name":"shell","arguments":"{\\\"command\\\":\\\"echo alpha\\\"}"}}\n\n',
                    "event: response.completed\n",
                    'data: {"type":"response.completed","response":{"id":"resp_tool_alpha","status":"completed","usage":{"input_tokens":14,"output_tokens":4,"total_tokens":18}}}\n\n',
                ]
            else:
                parts = [
                    "event: response.created\n",
                    'data: {"type":"response.created","response":{"id":"resp_alpha","model":"%s"}}\n\n' % payload.get("model", "unknown"),
                    "event: response.output_text.delta\n",
                    'data: {"type":"response.output_text.delta","delta":"alpha stream ok"}\n\n',
                    "event: response.completed\n",
                    'data: {"type":"response.completed","response":{"id":"resp_alpha","status":"completed"}}\n\n',
                ]
            for part in parts:
                self.wfile.write(part.encode("utf-8"))
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        if tools_requested:
            response = {
                "id": "resp_tool_alpha",
                "object": "response",
                "status": "completed",
                "model": payload.get("model"),
                "output": [{
                    "type": "function_call",
                    "call_id": "call_shell_1",
                    "name": "shell",
                    "arguments": "{\"command\":\"echo alpha\"}"
                }],
                "usage": {"input_tokens": 14, "output_tokens": 4}
            }
        else:
            response = {
                "id": "resp_alpha",
                "object": "response",
                "status": "completed",
                "model": payload.get("model"),
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "alpha unary ok"}]
                }],
                "usage": {"input_tokens": 12, "output_tokens": 5}
            }
        self.wfile.write(json.dumps(response).encode("utf-8"))

    def log_message(self, *args):
        return

HTTPServer(("127.0.0.1", int(__import__("os").environ["UPSTREAM_PORT"])), Handler).serve_forever()
PY

UPSTREAM_PORT="$UPSTREAM_PORT" python3 "$TMP_DIR/upstream_stub.py" >"$TMP_DIR/upstream.log" 2>&1 &
UPSTREAM_PID=$!

BROWSER_PORT="$BROWSER_PORT" PORT="$BROWSER_PORT" node "$ROOT_DIR/services/browser-assist/server.mjs" >"$TMP_DIR/browser.log" 2>&1 &
BROWSER_PID=$!
wait_http "http://127.0.0.1:$BROWSER_PORT/health"

CMGR_INSTANCE_ID=cmgr-full-smoke \
CMGR_SERVER_DATA_PORT="$DATA_PORT" \
CMGR_SERVER_ADMIN_PORT="$ADMIN_PORT" \
CMGR_BROWSER_ASSIST_URL="http://127.0.0.1:$BROWSER_PORT" \
"$SERVER_BIN" >"$TMP_DIR/server.log" 2>&1 &
SERVER_PID=$!
wait_http "http://127.0.0.1:$ADMIN_PORT/health"

PORT="$WEB_PORT" \
SERVER_ADMIN_ORIGIN="http://127.0.0.1:$ADMIN_PORT" \
npm run start -w @codex-manager/web >"$TMP_DIR/web.log" 2>&1 &
WEB_PID=$!
wait_http "http://127.0.0.1:$WEB_PORT/"

TENANT_JSON="$(curl -fsS -X POST http://127.0.0.1:$ADMIN_PORT/api/v1/tenants \
  -H 'Content-Type: application/json' \
  --data '{"slug":"alpha-lab","name":"Alpha Lab"}')"
TENANT_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$TENANT_JSON")"

API_KEY_JSON="$(curl -fsS -X POST http://127.0.0.1:$ADMIN_PORT/api/v1/gateway/api-keys \
  -H 'Content-Type: application/json' \
  --data "{\"tenantId\":\"$TENANT_ID\",\"name\":\"Alpha Key\"}")"
API_KEY_TOKEN="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["token"])' "$API_KEY_JSON")"

curl -sS -o "$TMP_DIR/no_account_response.json" -w '%{http_code}' \
  -X POST http://127.0.0.1:$DATA_PORT/v1/responses \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-empty' \
  --data '{"model":"gpt-5.4","input":"hello alpha","stream":false}' >"$TMP_DIR/no_account_status.txt"
NO_ACCOUNT_STATUS="$(cat "$TMP_DIR/no_account_status.txt")"

UNBOUND_ACCOUNT_JSON="$(curl -fsS -X POST http://127.0.0.1:$ADMIN_PORT/api/v1/accounts/import \
  -H 'Content-Type: application/json' \
  --data "{\"tenantId\":\"$TENANT_ID\",\"label\":\"Ghost Upstream\",\"models\":[\"gpt-5.4\"]}")"

curl -sS -o "$TMP_DIR/no_credential_response.json" -w '%{http_code}' \
  -X POST http://127.0.0.1:$DATA_PORT/v1/responses \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-ghost' \
  --data '{"model":"gpt-5.4","input":"hello alpha","stream":false}' >"$TMP_DIR/no_credential_status.txt"
NO_CREDENTIAL_STATUS="$(cat "$TMP_DIR/no_credential_status.txt")"

ACCOUNT_JSON="$(curl -fsS -X POST http://127.0.0.1:$ADMIN_PORT/api/v1/accounts/import \
  -H 'Content-Type: application/json' \
  --data "{\"tenantId\":\"$TENANT_ID\",\"label\":\"Alpha Upstream\",\"models\":[\"gpt-5.4\"],\"baseUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/v1\",\"bearerToken\":\"upstream-secret\",\"chatgptAccountId\":\"acct-alpha\",\"extraHeaders\":[[\"x-alpha\",\"1\"]]}")"
ACCOUNT_ID="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["id"])' "$ACCOUNT_JSON")"

LOGIN_JSON="$(curl -fsS -X POST http://127.0.0.1:$ADMIN_PORT/api/v1/browser/tasks/login \
  -H 'Content-Type: application/json' \
  --data "{\"accountId\":\"$ACCOUNT_ID\",\"provider\":\"openai\",\"loginUrl\":\"http://127.0.0.1:$UPSTREAM_PORT/openai-login\",\"email\":\"alpha@example.com\",\"password\":\"topsecret\",\"otpCode\":\"123456\",\"notes\":\"alpha login\",\"headless\":true}")"

for _ in $(seq 1 80); do
  TASKS_JSON="$(curl -fsS http://127.0.0.1:$ADMIN_PORT/api/v1/browser/tasks)"
  if python3 - "$TASKS_JSON" <<'PY'
import json,sys
payload=json.loads(sys.argv[1])
raise SystemExit(0 if any(item["status"] == "completed" for item in payload) else 1)
PY
  then
    break
  fi
  sleep 0.5
done

RESP_JSON="$(curl -fsS -X POST http://127.0.0.1:$DATA_PORT/v1/responses \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-shell' \
  --data '{"model":"gpt-5.4","input":"hello alpha","stream":false}')"
CHAT_JSON="$(curl -fsS -X POST http://127.0.0.1:$DATA_PORT/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-chat' \
  --data '{"model":"gpt-5.4","messages":[{"role":"user","content":"hello chat"}],"stream":false}')"
CHAT_STREAM="$(curl -fsS -N -X POST http://127.0.0.1:$DATA_PORT/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-chat-stream' \
  --data '{"model":"gpt-5.4","messages":[{"role":"user","content":"hello chat"}],"stream":true}')"
CHAT_TOOL_JSON="$(curl -fsS -X POST http://127.0.0.1:$DATA_PORT/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-chat-tool' \
  --data '{"model":"gpt-5.4","messages":[{"role":"user","content":"call shell"}],"tools":[{"type":"function","function":{"name":"shell","parameters":{"type":"object"}}}],"tool_choice":"auto","stream":false}')"
CHAT_TOOL_STREAM="$(curl -fsS -N -X POST http://127.0.0.1:$DATA_PORT/v1/chat/completions \
  -H "Authorization: Bearer $API_KEY_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'x-codex-cli-affinity-id: alpha-chat-tool-stream' \
  --data '{"model":"gpt-5.4","messages":[{"role":"user","content":"call shell"}],"tools":[{"type":"function","function":{"name":"shell","parameters":{"type":"object"}}}],"tool_choice":"auto","stream":true}')"
printf '%s' "$CHAT_STREAM" >"$TMP_DIR/chat_stream.txt"
printf '%s' "$CHAT_TOOL_STREAM" >"$TMP_DIR/chat_tool_stream.txt"
printf '%s' "$TASKS_JSON" >"$TMP_DIR/tasks.json"

curl -fsS -X POST "http://127.0.0.1:$ADMIN_PORT/api/v1/accounts/$ACCOUNT_ID/route-events" \
  -H 'Content-Type: application/json' \
  --data '{"mode":"direct","kind":"cf_hit"}' >/dev/null
sleep 1
DASHBOARD_JSON="$(curl -fsS http://127.0.0.1:$ADMIN_PORT/api/v1/dashboard)"
EGRESS_JSON="$(curl -fsS http://127.0.0.1:$ADMIN_PORT/api/v1/egress-slots)"
WEB_HTML="$(curl -fsS http://127.0.0.1:$WEB_PORT/)"
CONTEXT_ROWS="$(psql -h 127.0.0.1 -U codex_manager -d codex_manager -Atqc 'select count(*) from conversation_contexts')"

json_eval "$RESP_JSON" 'assert payload["status"] == "completed"; assert payload["id"] == "resp_alpha"'
json_eval "$CHAT_JSON" 'assert payload["object"] == "chat.completion"; assert payload["choices"][0]["message"]["content"] == "alpha unary ok"'
json_eval "$CHAT_TOOL_JSON" 'assert payload["choices"][0]["finish_reason"] == "tool_calls"; assert payload["choices"][0]["message"]["tool_calls"][0]["function"]["name"] == "shell"; assert payload["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] == "{\"command\":\"echo alpha\"}"'
python3 - "$NO_ACCOUNT_STATUS" "$(cat "$TMP_DIR/no_account_response.json")" <<'PY'
import json, sys
assert sys.argv[1] == "503"
payload = json.loads(sys.argv[2])
assert payload["error"]["type"] == "server_busy"
PY
python3 - "$NO_CREDENTIAL_STATUS" "$(cat "$TMP_DIR/no_credential_response.json")" <<'PY'
import json, sys
assert sys.argv[1] == "503"
payload = json.loads(sys.argv[2])
assert payload["error"]["type"] == "server_busy"
PY
python3 - "$CHAT_STREAM" <<'PY'
import sys
body = sys.argv[1]
assert "chat.completion.chunk" in body
assert "[DONE]" in body
assert "alpha stream ok" in body
PY
python3 - "$CHAT_TOOL_STREAM" <<'PY'
import json
import sys

body = sys.argv[1]
assert "chat.completion.chunk" in body
assert "\"tool_calls\"" in body
assert "[DONE]" in body

argument_parts = []
finish_reasons = []
for frame in body.split("\n\n"):
    frame = frame.strip()
    if not frame or frame == "data: [DONE]":
        continue
    if not frame.startswith("data: "):
        continue
    payload = json.loads(frame[6:])
    choice = payload["choices"][0]
    finish_reasons.append(choice.get("finish_reason"))
    for tool_call in choice.get("delta", {}).get("tool_calls", []):
        argument_parts.append(tool_call.get("function", {}).get("arguments", ""))

assert "".join(argument_parts) == "{\"command\":\"echo alpha\"}"
assert "tool_calls" in finish_reasons
PY
json_eval "$LOGIN_JSON" 'assert payload["status"] in {"queued", "running", "completed"}'
python3 - "$TASKS_JSON" <<'PY'
import json, sys
payload = json.loads(sys.argv[1])
assert len(payload) >= 1
completed = [item for item in payload if item["status"] == "completed"]
assert completed, payload
assert any(item.get("provider") == "openai" for item in completed)
assert any(item.get("storageStatePath") for item in completed)
PY
json_eval "$DASHBOARD_JSON" 'assert payload["counts"]["browserTasks"] >= 1; assert len(payload["accounts"]) >= 1; assert len(payload["browserTasks"]) >= 1; assert "egressGroup" in payload["accounts"][0]; assert "proxyEnabled" in payload["accounts"][0]'
json_eval "$EGRESS_JSON" 'assert len(payload) == 2; assert {item["id"] for item in payload} == {"direct", "warp"}'
python3 - "$CONTEXT_ROWS" <<'PY'
import sys
assert int(sys.argv[1]) >= 1
PY
python3 - "$WEB_HTML" <<'PY'
import sys
html = sys.argv[1]
assert "Codex Manager 2.0" in html
assert "Alpha Upstream" in html
assert "Browser Assist" in html
PY

echo "full smoke passed"
