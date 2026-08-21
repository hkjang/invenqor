#!/bin/sh
set -eu

TARGETS=${TARGETS:-"x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu"}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)

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
    # Re-enter the canonical repository path for every target. On WSL, cross
    # can otherwise carry a differently-cased /mnt/c path into the next Docker
    # invocation and rustc reports that its working directory no longer exists.
    (
        cd "$ROOT"
        CARGO_TARGET_DIR="$ROOT/$target_dir" RUSTFLAGS="-C target-cpu=$cpu" \
            cross build --locked --release --target "$target"
    )
done
