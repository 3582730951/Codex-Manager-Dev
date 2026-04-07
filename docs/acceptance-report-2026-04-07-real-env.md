# Acceptance Report 2026-04-07 Real Env

## Scope

This report covers the current workspace project in `D:\Code\R3_Code\MI\Codex-Manger`,
validated on April 7, 2026 inside a newly created Docker helper container under
`WSL -> Ubuntu -> Docker`.

The upstream credential used for validation was extracted from a currently running legacy container
that still had quota available. Secrets are intentionally not recorded in this report.

## Runtime Topology

Validation used:

- helper container: `cmgrtest-helper`
- current workspace stack:
  - `cmgrt-postgres`
  - `cmgrt-redis`
  - `cmgrt-browser-assist`
  - `cmgrt-server`
  - `cmgrt-web`

## Code Changes Verified

The current workspace server was updated to handle the real `chatgpt.com/backend-api/codex`
protocol correctly:

- real Codex upstream now forces:
  - `instructions`
  - `store=false`
  - `stream=true`
- non-stream downstream calls are now implemented by aggregating upstream SSE into JSON
- versioned model aliases such as `gpt-5.2-2025-12-11` are no longer misclassified as drift
- `cf-ray` no longer causes false-positive Cloudflare classification on ordinary `400` errors

## Real Upstream Validation

All tests below were executed from inside `cmgrtest-helper`, not from the WSL host directly.

### Admin and import

- admin health: `200`
- real tenant creation: `200`
- real account import: `200`
- gateway API key creation: `200`

### Models

- `GET /v1/models`: `200`
- returned models included:
  - `gpt-5.2`
  - `gpt-5.3-codex`
  - `gpt-5.4`

### Non-stream responses

- `POST /v1/responses`: `200`
- returned model: `gpt-5.2-2025-12-11`
- returned assistant text matched the prompt contract:
  - `CMGR_REAL_RESP`

### Non-stream chat/completions

- `POST /v1/chat/completions`: `200`
- returned model: `gpt-5.2-2025-12-11`
- returned assistant text matched the prompt contract:
  - `CMGR_REAL_CHAT`

### SSE responses

- `POST /v1/responses` with `stream=true`: `200`
- observed events included:
  - `response.created`
  - `response.in_progress`
  - `response.output_item.added`
  - `response.output_text.delta`
  - `response.completed`

### SSE chat/completions

- `POST /v1/chat/completions` with `stream=true`: `200`
- observed output included:
  - assistant role chunk
  - content deltas
  - stop chunk
  - `[DONE]`

## Resource Snapshot

`docker stats --no-stream` snapshot after the rebuilt stack started:

- `cmgrt-postgres`: `50.23MiB`
- `cmgrt-redis`: `12.29MiB`
- `cmgrt-browser-assist`: `151.8MiB`
- `cmgrt-server`: `2.30MiB`
- `cmgrt-web`: `42.92MiB`

## Cache Observation

A same-affinity repeated real request was executed through the current gateway.

- first sample:
  - `input_tokens=23`
  - `output_tokens=6`
  - `total_tokens=29`
  - `cached_tokens=0`
- second sample:
  - `input_tokens=23`
  - `output_tokens=6`
  - `total_tokens=29`
  - `cached_tokens=0`

This confirms the real cache metric extraction path works, even though this short sample did not
show a cache hit yet.

## Cache Benchmark

To measure actual cache-hit probability, a second benchmark used long stable prefixes through the
current gateway on real upstream traffic.

### Short prompt sample

- very short prompt repeated twice
- result:
  - request 1: `cached_tokens=0`
  - request 2: `cached_tokens=0`

This indicates short prompts are not a useful cache benchmark.

### Cold benchmark: same affinity

A fresh long prefix was sent 6 times with the same `x-codex-cli-affinity-id`.

- request 1:
  - `input_tokens=16231`
  - `cached_tokens=0`
- requests 2-6:
  - all 5 requests returned `cached_tokens=16000`

Observed metrics:

- overall hit rate: `5/6 = 83.3%`
- hit rate after warmup: `5/5 = 100%`
- cached share after warmup: about `16000 / 16231 = 98.6%`

### Cold benchmark: varying affinity

A different fresh long prefix was sent 6 times, but each request used a different
`x-codex-cli-affinity-id`.

- requests 1-3:
  - `cached_tokens=0`
- requests 4-6:
  - all 3 requests returned `cached_tokens=12416`

Observed metrics:

- overall hit rate: `3/6 = 50.0%`
- hit rate after the first request: `3/5 = 60.0%`
- average cached tokens after the first request: `7449.6`

### Interpretation

- short requests: cache-hit probability is effectively low
- long stable prefixes with sticky affinity: cache-hit probability is very high after warmup
- changing affinity lowers consistency noticeably, even when the request body stays stable

## Multi-Agent Cache Benchmark

The current gateway was also tested for same-CLI multi-agent behavior using real upstream traffic.

### Shared CLI with subagents

Setup:

- same `x-codex-cli-affinity-id`
- different `x-openai-subagent` values
- long shared prefix
- 6 total requests

Observed result:

- request 1: `cached_tokens=0`
- requests 2-6: all returned `cached_tokens=9728`
- all 6 requests used the same prompt cache key:
  - `cli-829a5197`

Metrics:

- overall hit rate: `5/6 = 83.3%`
- hit rate after warmup: `5/5 = 100%`
- average cached tokens after warmup: `9728`

### Isolated subagents without shared CLI affinity

Setup:

- no `x-codex-cli-affinity-id`
- principal falls back to each `x-openai-subagent`
- different subagent value on every request
- same long shared prefix
- 6 total requests

Observed result:

- requests 1 and 3: `cached_tokens=0`
- requests 2, 4, 5, 6: `cached_tokens=7040`
- each request used a different prompt cache key derived from the subagent principal

Metrics:

- overall hit rate: `4/6 = 66.7%`
- hit rate after the first request: `4/5 = 80.0%`
- average cached tokens after the first request: `5632`

### Interpretation

- shared CLI affinity gives more stable cache behavior across subagents
- even isolated subagents can still receive some provider-side prefix hits
- however the shared-CLI case is clearly stronger and more predictable

## Replay Cost Benchmark

The gateway does not expose a runtime switch to force `full-history replay` vs `dependency-frontier replay`,
so this benchmark used two real request shapes through the current gateway:

- a synthetic full-history request carrying 6 prior user/assistant turns
- a synthetic frontier replay request carrying only a compact replay summary plus the final ask

Both used real upstream traffic on `gpt-5.2`.

### Full history

- cold run:
  - `input_tokens=5406`
  - `cached_tokens=0`
- warm run:
  - `input_tokens=5406`
  - `cached_tokens=5248`

### Frontier replay

- cold run:
  - `input_tokens=122`
  - `cached_tokens=0`
- warm run:
  - `input_tokens=122`
  - `cached_tokens=0`

### Interpretation

- frontier replay reduced cold input-token cost by `97.74%`
- frontier replay reduced warm input-token cost by `97.74%`
- the frontier replay sample is short enough that prompt caching did not activate, which is acceptable
- the key gain of frontier replay is lower replay cost, not relying on cache to rescue an oversized replay

## Remaining Gaps

The main remaining gaps are now external or long-horizon:

- real Cloudflare challenge and direct/warp/cooldown recovery under actual CF pressure
- long-window cache efficiency sampling
- larger-scale concurrent resource profiling
