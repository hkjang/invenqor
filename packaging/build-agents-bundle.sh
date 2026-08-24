#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "Usage: $0 <version> [output-directory]" >&2
    exit 2
fi

VERSION=$1
if ! printf '%s\n' "$VERSION" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'; then
    echo "version must contain three decimal numbers, for example 0.2.15" >&2
    exit 2
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
OUTPUT_DIR=${2:-${OUTPUT_DIR:-$ROOT}}
NAME="invenqor-agents-$VERSION"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
PACKAGE="$STAGE/$NAME"

mkdir -p "$OUTPUT_DIR" "$PACKAGE"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd)
for archive in \
    invenqor-agent-linux-x86_64.tar.gz \
    invenqor-agent-linux-aarch64.tar.gz \
    invenqor-agent-windows-x86_64.zip
do
    for file in "$archive" "$archive.sha256"; do
        if [ ! -f "$OUTPUT_DIR/$file" ]; then
            echo "missing Agent release asset: $OUTPUT_DIR/$file" >&2
            exit 1
        fi
    done
    # Never turn a stale binary and an unrelated sidecar into an apparently
    # valid outer offline bundle. Verify each fixed-name pair before copying it.
    (cd "$OUTPUT_DIR" && sha256sum --strict -c "$archive.sha256")
    install -m 0644 "$OUTPUT_DIR/$archive" "$PACKAGE/$archive"
    install -m 0644 "$OUTPUT_DIR/$archive.sha256" "$PACKAGE/$archive.sha256"
done
install -m 0755 "$ROOT/scripts/sign-agent-update-manifest-v2.py" \
    "$PACKAGE/sign-agent-update-manifest-v2.py"
sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/AGENTS_BUNDLE_README.md" \
    > "$PACKAGE/README.md"
chmod 0644 "$PACKAGE/README.md"

ARCHIVE="$NAME.tar.gz"
tar -C "$STAGE" --sort=name --mtime="@${SOURCE_DATE_EPOCH:-0}" \
    --owner=0 --group=0 --numeric-owner -czf "$OUTPUT_DIR/$ARCHIVE" "$NAME"
(cd "$OUTPUT_DIR" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
echo "$OUTPUT_DIR/$ARCHIVE"
echo "$OUTPUT_DIR/$ARCHIVE.sha256"
