#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
VERSION=${1:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)}
OUTPUT_DIR=${2:-"$ROOT/dist"}

if [ -z "$VERSION" ]; then
    echo "could not determine the release version" >&2
    exit 1
fi

OUTPUT_DIR="$OUTPUT_DIR" "$ROOT/packaging/build-tar.sh" x86_64-unknown-linux-musl
OUTPUT_DIR="$OUTPUT_DIR" "$ROOT/packaging/build-tar.sh" aarch64-unknown-linux-musl
OUTPUT_DIR="$OUTPUT_DIR" "$ROOT/packaging/build-zip.sh" x86_64-pc-windows-gnu
"$ROOT/packaging/build-agents-bundle.sh" "$VERSION" "$OUTPUT_DIR"
