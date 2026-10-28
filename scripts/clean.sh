#!/usr/bin/env bash
# Frees the build cache of the launcher's own crates (k0t1k-core, k0t1k-app), which every edit rewrites and
# which grows with each incremental build. Compiled third-party dependencies stay, so the next build only
# recompiles the launcher itself.
# Usage: scripts/clean.sh [--all]    --all also drops the dependencies (a full `cargo clean`)
set -euo pipefail
cd "$(dirname "$0")/.."

before=$(du -sh target 2>/dev/null | cut -f1 || echo 0)
if [[ ${1:-} == --all ]]; then
  cargo clean
else
  # each profile keeps its own copy; `cargo clean -p` only looks at the one it is given
  for dir in target/*/; do
    name=$(basename "$dir")
    case $name in
      debug) cargo clean -p k0t1k-core -p k0t1k-app ;;
      release | dev-release) cargo clean -p k0t1k-core -p k0t1k-app --profile "$name" ;;
    esac
  done
fi
echo "target: $before -> $(du -sh target 2>/dev/null | cut -f1 || echo 0)"
