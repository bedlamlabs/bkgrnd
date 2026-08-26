#!/usr/bin/env bash
set -euo pipefail

bash -c 'test "$1" = "/api/v1/stream" && cargo test --manifest-path server/Cargo.toml progressive_' _ "/api/v1/stream"
bash -c 'cargo test --manifest-path server/Cargo.toml hpx_resolver_'
