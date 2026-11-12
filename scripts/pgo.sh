#!/usr/bin/env bash
# Build a profile-guided release of the CLI and desktop app.
#
# Usage: scripts/pgo.sh <rom-directory> [output-target-dir]
#
# The training run uses only the user's own ROM directory and never writes
# ROM data into the repository. A PGO build is typically 10-15% faster than a
# plain release build on the accuracy core.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
ROM_DIR=${1:?usage: scripts/pgo.sh <rom-directory> [output-target-dir]}
OUT_DIR=${2:-"$ROOT/target/pgo"}
PROFILE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/unko-pgo.XXXXXX")
GEN_DIR="$ROOT/target/pgo-gen"
trap 'rm -rf "$PROFILE_DIR"' EXIT

if ! command -v llvm-profdata >/dev/null 2>&1; then
  echo "llvm-profdata is required (install the LLVM tools matching rustc)" >&2
  exit 1
fi

echo "==> instrumented build"
RUSTFLAGS="-Cprofile-generate=$PROFILE_DIR" \
  cargo build --release --target-dir "$GEN_DIR" -p unko-cli -p unko-app

echo "==> training on $ROM_DIR"
"$GEN_DIR/release/unko" corpus "$ROM_DIR" --frames 20 --limit 600 --jobs 0 >/dev/null

echo "==> merging profile"
llvm-profdata merge -o "$PROFILE_DIR/merged.profdata" "$PROFILE_DIR"/*.profraw

echo "==> profile-guided build"
RUSTFLAGS="-Cprofile-use=$PROFILE_DIR/merged.profdata -Cllvm-args=-pgo-warn-missing-function" \
  cargo build --release --target-dir "$OUT_DIR" -p unko-cli -p unko-app

echo "==> done"
echo "CLI: $OUT_DIR/release/unko"
echo "App: $OUT_DIR/release/unko-app"
