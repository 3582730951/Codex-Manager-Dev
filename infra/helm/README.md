# Helm

The chart now lives under `infra/helm/codex-manager` and deploys the same runtime contract as the
default Docker baseline:

- `web`
- `server`
- `browser-assist`
- `postgres`
- `redis`

The chart is intentionally compact and mirrors the Compose topology rather than introducing extra
runtime hops.
