# Acceptance Report 2026-04-06

## Scope

This report covers the current Codex-Manager alpha implementation in `/workspace`:

- separated `web`, `server`, and `browser-assist`
- lease-bound single-CLI affinity
- hidden failover for quota/auth/capability/CF conditions
- responses-first chat adapter
- Postgres persistence and Redis bus
- direct/warp routing state machine
- near-quota guard gating based on `5h` and `7d` headroom

## Codex Reference Signals

The gateway interception logic was aligned to signals visible in the public `openai/codex` codebase
under `/home/tmp/openai-codex`, especially:

- `response.failed` SSE events
- `error.code`
- `error.type`
- `rate_limit_exceeded`
- `insufficient_quota`
- `usage_limit_reached`
- `openai-model`
- `x-reasoning-included`
- `X-Models-Etag`
- `plan_type`
- `resets_at`

Relevant local reference points:

- `/home/tmp/openai-codex/codex-rs/codex-api/src/sse/responses.rs`
- `/home/tmp/openai-codex/codex-rs/codex-api/src/api_bridge.rs`
- `/home/tmp/openai-codex/codex-rs/codex-api/src/rate_limits.rs`

## Implemented Behavior

### Hidden failure interception

The gateway now intercepts:

- upstream HTTP failures already classified as `cf`, `auth`, `quota`, or `capability`
- upstream HTTP 200 JSON payloads that secretly carry failure objects
- upstream SSE `response.failed` records for guarded accounts
- upstream model drift signaled through `openai-model` or mismatched response `model`

Intercepted failures are not passed downstream as raw upstream errors. The downstream sees either:

- a transparent retry on a backup account, or
- hidden wait / `server_busy` if no valid exact-capability backup exists

### Near-quota guard gate

The heavier interception path is now gated by explicit windowed headroom:

- enable guard when `quotaHeadroom5h < 0.30`
- enable guard when `quotaHeadroom7d < 0.30`

Healthy accounts avoid the heavier raw-responses stream preflight path to reduce CPU and latency
overhead.

### Automatic failover

The current implementation automatically failovers for:

- `Quota`
- `Auth`
- `Capability`
- `CF` according to the direct/warp state machine

For quota and capability failures, the old lease is evicted and the same CLI principal is rebound to
a backup account when available.

## Validation Performed

### Unit and workspace tests

Executed successfully:

- `cargo test --workspace --quiet`
- `npm run build:web`

Current workspace Rust test count: `25`.

### Smoke suites

Executed successfully:

- `bash scripts/full_smoke.sh`
- `bash scripts/cluster_smoke.sh`
- `bash scripts/edge_smoke.sh`

### Edge cases explicitly covered

`edge_smoke.sh` now validates:

- `/v1/responses` unary hidden quota failure -> backup account success
- `/v1/chat/completions` unary hidden quota failure -> backup account success
- `/v1/responses` SSE immediate `response.failed` -> backup account stream success
- no-backup quota failure -> hidden `server_busy`, no raw quota text
- upstream hidden model drift (`openai-model: gpt-4.1-mini`) -> intercepted and retried
- account summaries expose `quotaHeadroom5h`, `quotaHeadroom7d`, and `nearQuotaGuardEnabled`

## Docker Validation Result

### What succeeded

- Docker Engine and `docker-compose` were installed successfully on this machine.
- `docker info` succeeded against a manually started daemon.
- `compose.yml` validated successfully.
- `infra/docker/compose.hostnet.override.yml` was added as a restricted-environment fallback for
  hosts that forbid bridge network creation.

### What failed

Actual container startup is blocked by this host kernel/runtime policy, not by the repository:

- default bridge-network Compose failed with:
  - `operation not permitted` during Docker network creation
- host-network fallback still failed during image layer application with:
  - `Error creating mount namespace before pivot: operation not permitted`

This means this specific environment does not allow nested Docker containers to actually run.

### Impact

Container definitions, Dockerfiles, and Compose structure were validated as far as this environment
permits, but full live container execution could not be completed on this host.

## Remaining Gaps

The main remaining production gap is not code-completeness but external-environment validation:

- real OpenAI/Codex account validation on the actual upstream site
- real Cloudflare challenge behavior in the wild
- a host that permits nested Docker containers for end-to-end Compose startup

## Key Files

- `/workspace/services/server/src/http/data.rs`
- `/workspace/services/server/src/upstream.rs`
- `/workspace/services/server/src/models.rs`
- `/workspace/services/server/src/state.rs`
- `/workspace/services/server/src/scheduler/router.rs`
- `/workspace/scripts/edge_smoke.sh`
- `/workspace/compose.yml`
- `/workspace/infra/docker/compose.hostnet.override.yml`
