#!/bin/sh
set -eu

TARGETS=${TARGETS:-"x86_64-unknown-linux-musl aarch64-unknown-linux-musl"}

if ! command -v cross >/dev/null 2>&1; then
    echo "cross is required: cargo install cross --locked" >&2
    exit 1
fi

for target in $TARGETS; do
    case "$target" in
        x86_64-unknown-linux-musl) cpu=x86-64 ;;
        aarch64-unknown-linux-musl) cpu=generic ;;
        *) echo "Unsupported release target: $target" >&2; exit 1 ;;
    esac
    target_dir="target-${target%%-*}"
    CARGO_TARGET_DIR="$target_dir" RUSTFLAGS="-C target-cpu=$cpu" \
        cross build --locked --release --target "$target"
done
