# Codex Manager 2.0

Codex Manager 2.0 is a fresh implementation of a lease-bound Codex orchestration system. The
default runtime topology is:

- `web`: Node SSR management console
- `server`: Rust control plane + gateway data plane in one binary with two listeners
- `browser-assist`: dedicated challenge/login sidecar
- `postgres`
- `redis`

## Current Status

This repository now contains a working alpha of the new architecture:

- monorepo layout for `web`, `server`, `browser-assist`, and shared contracts
- default Docker Compose topology with separate `web` and `server` containers
- Rust gateway with:
  - `/v1/responses`
  - `/v1/chat/completions`
  - `/v1/models`
- admin APIs for dashboard, tenants, accounts, gateway API keys, leases, cache metrics, CF incidents, and browser tasks
- async Postgres persistence with startup snapshot restore and memory-only fallback
- Redis control bus for cross-instance lease and route-state propagation with graceful fallback
- first-pass schema migration under `services/server/migrations`
- dual-candidate lease selection primitives
- direct-to-warp and warp cooldown state machine
- replay pack compiler and prefix warmup decision logic
- persisted replay context cache that recompiles recent effective context into upstream requests after lease generation changes and survives restarts
- direct/warp proxy-group selection for both server upstream traffic and browser-assist tasks
- admin API for egress slots
- real upstream forwarding for credentialed accounts:
  - `/v1/responses` unary passthrough
  - `/v1/responses` SSE passthrough
  - `/v1/chat/completions` responses-first adapter
  - `Authorization`, `ChatGPT-Account-ID`, `session_id`, `x-client-request-id`, and `x-openai-subagent` forwarding semantics
- hard failures are hidden from downstream and push the lease back into queueing or failover instead of returning mock content
- browser-assist sidecar now runs real Playwright-backed login/recover tasks with persistent Chromium profiles
- SSR dashboard with a distinct visual treatment

## Local Development

### Web

```bash
npm install
npm run dev:web
```

### Browser Assist

```bash
npm run browser-assist
```

### Server

```bash
cargo run -p codex-manager-server
```

### Cluster Smoke

```bash
npm run smoke:cluster
```

This verifies:

- local `Postgres` and `Redis` are installed and reachable on `127.0.0.1`
- local `Postgres` and `Redis` connectivity
- bootstrap persistence into `Postgres`
- cross-instance Redis fanout
- restart-time snapshot restore from `Postgres`

### Edge Smoke

```bash
npm run smoke:edge
```

This verifies:

- downstream CLI-style requests do not receive raw `insufficient_quota`, `usage_limit_reached`, or
  hidden model-drift payloads
- low-headroom accounts (`5h < 30%` or `7d < 30%`) trigger the heavier hidden-failover guard path
- unary `/v1/responses` failover to a healthy backup account
- unary `/v1/chat/completions` failover through the responses-first adapter
- SSE `/v1/responses` preflight interception of `response.failed`
- no-backup scenarios degrade to hidden `server_busy` instead of leaking upstream quota errors
- unexpected upstream `openai-model` drift is intercepted and replaced with backup-account routing

### Full Smoke

```text
npm run smoke:full
```

This verifies:

- `browser-assist` task execution
- tenant creation and gateway key minting
- upstream account import with credentials
- `/v1/responses` unary proxying
- `/v1/chat/completions` unary and SSE adaptation through upstream `/responses`
- CF route-event ingestion
- dashboard materialization
- egress-slot introspection
- Web SSR startup against the live admin API

### Importing A Real Upstream Account

`POST /api/v1/accounts/import` now accepts optional upstream credential fields:

- `baseUrl`
- `bearerToken`
- `chatgptAccountId`
- `extraHeaders`

If `bearerToken` is present, the account is eligible for real upstream proxying. If it is absent,
the account is visible to the control plane but is excluded from live lease selection.

## Notes

- The current server keeps the hot path in memory and mirrors state to `Postgres` through an async
  writer. If `Postgres` is unavailable at boot, the server degrades to `memory-only` mode instead of
  refusing to start.
- Demo data is disabled by default. Set `CMGR_ENABLE_DEMO_SEED=true` only if you explicitly want a
  seeded local showcase.
- `Redis` now carries control-plane fanout for leases, route-state changes, incidents, and cache
  metrics. If `Redis` is unavailable at boot, the bus disables cleanly and the node keeps serving.
- `direct/warp` can now map to real proxy groups through `CMGR_DIRECT_PROXY_URL`,
  `CMGR_WARP_PROXY_URL`, `CMGR_BROWSER_ASSIST_DIRECT_PROXY_URL`, and
  `CMGR_BROWSER_ASSIST_WARP_PROXY_URL`.
- `browser-assist` is a dedicated cold-path container on purpose. It does not participate in normal
  forwarding.
- The hot path is designed so successful routing does not need synchronous database work.
- The current `/v1/chat/completions` path now targets upstream `/responses` and adapts the result
  back into chat-completions wire format, including streamed and unary function/tool-call mapping.
- If there is no exact-capability credentialed account, the gateway enters hidden wait mode instead
  of leaking quota exhaustion, account loss, or model-capability details downstream.
- `infra/docker/compose.hostnet.override.yml` exists only as a restricted-environment fallback for
  hosts that forbid Docker bridge network creation. It does not replace the default `compose.yml`.
