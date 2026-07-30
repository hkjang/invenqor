#!/bin/sh
set -eu

TARGETS=${TARGETS:-"x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu"}

if ! command -v cross >/dev/null 2>&1; then
    echo "cross is required: cargo install cross --locked" >&2
    exit 1
fi

for target in $TARGETS; do
    case "$target" in
        x86_64-unknown-linux-musl) cpu=x86-64; target_dir="target-x86_64" ;;
        aarch64-unknown-linux-musl) cpu=generic; target_dir="target-aarch64" ;;
        # The Windows build is named apart from the Linux x86_64 one so the two
        # never share a target directory and overwrite each other's artifacts.
        x86_64-pc-windows-gnu) cpu=x86-64; target_dir="target-windows-x86_64" ;;
        *) echo "Unsupported release target: $target" >&2; exit 1 ;;
    esac
    CARGO_TARGET_DIR="$target_dir" RUSTFLAGS="-C target-cpu=$cpu" \
        cross build --locked --release --target "$target"
done
