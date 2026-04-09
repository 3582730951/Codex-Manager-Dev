#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This script must run inside WSL Ubuntu or another Linux shell."
  exit 1
fi

if ! grep -qiE "(microsoft|wsl)" /proc/version 2>/dev/null; then
  echo "Expected a WSL Linux environment. Refusing to run tests outside WSL."
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is not installed in this WSL environment."
  exit 1
fi

docker compose -f compose.test.yml up --build --abort-on-container-exit --exit-code-from server-test
