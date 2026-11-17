#!/usr/bin/env bash
# Local development: Postgres from Homebrew, fixed setup code, verbose logs.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
export DATABASE_URL="${DATABASE_URL:-postgres://$(id -un)@localhost/eof}"
export PUBLIC_URL="${PUBLIC_URL:-http://localhost:8080}"
export EOF_SETUP_CODE="${EOF_SETUP_CODE:-TEST-CODE-2026}"
export RUST_LOG="${RUST_LOG:-eof=debug,tower_http=info}"
exec cargo run
