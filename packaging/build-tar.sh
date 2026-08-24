#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <x86_64-unknown-linux-musl|aarch64-unknown-linux-musl>" >&2
    exit 2
fi

TARGET=$1
case "$TARGET" in
    x86_64-unknown-linux-musl) ARCH=x86_64; TARGET_DIR=target-x86_64 ;;
    aarch64-unknown-linux-musl) ARCH=aarch64; TARGET_DIR=target-aarch64 ;;
    *) echo "Unsupported target: $TARGET" >&2; exit 2 ;;
esac

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUTPUT_DIR=${OUTPUT_DIR:-$ROOT}
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
PACKAGE="$STAGE/invenqor-agent-linux-$ARCH"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd)

mkdir -p "$PACKAGE/bin" "$PACKAGE/config" "$PACKAGE/scripts" "$PACKAGE/service"
install -m 0755 "$ROOT/$TARGET_DIR/$TARGET/release/invenqor-agent" "$PACKAGE/bin/"
install -m 0600 "$ROOT/config/config.toml" "$PACKAGE/config/"
install -m 0755 "$ROOT/packaging/scripts/install.sh" "$PACKAGE/scripts/"
install -m 0755 "$ROOT/packaging/scripts/uninstall.sh" "$PACKAGE/scripts/"
install -m 0644 "$ROOT/packaging/systemd/invenqor-agent.service" "$PACKAGE/service/"
install -m 0644 "$ROOT/packaging/systemd/invenqor-agent-update.service" "$PACKAGE/service/"
install -m 0644 "$ROOT/packaging/systemd/invenqor-agent-update.path" "$PACKAGE/service/"
install -m 0755 "$ROOT/packaging/sysv/invenqor-agent" "$PACKAGE/service/invenqor-agent.init"
install -m 0755 "$ROOT/packaging/openrc/invenqor-agent" "$PACKAGE/service/invenqor-agent.openrc"
install -m 0644 "$ROOT/README.md" "$PACKAGE/"

tar -C "$STAGE" --sort=name --mtime="@${SOURCE_DATE_EPOCH:-0}" \
    --owner=0 --group=0 --numeric-owner -czf \
    "$OUTPUT_DIR/invenqor-agent-linux-$ARCH.tar.gz" "invenqor-agent-linux-$ARCH"
ARCHIVE="invenqor-agent-linux-$ARCH.tar.gz"
(cd "$OUTPUT_DIR" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
echo "$OUTPUT_DIR/$ARCHIVE"
echo "$OUTPUT_DIR/$ARCHIVE.sha256"
