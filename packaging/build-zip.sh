#!/bin/sh
set -eu

# Builds the Windows distribution archive. A zip rather than an MSI: the package
# has one binary, one configuration file and a service registration, and an MSI
# would add a build dependency and an install database for no gain. The install
# script is idempotent, which is what an upgrade actually needs.

if [ "$#" -ne 1 ]; then
    echo "Usage: $0 <x86_64-pc-windows-gnu>" >&2
    exit 2
fi

TARGET=$1
case "$TARGET" in
    x86_64-pc-windows-gnu) ARCH=x86_64; TARGET_DIR=target-windows-x86_64 ;;
    *) echo "Unsupported target: $TARGET" >&2; exit 2 ;;
esac

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
NAME="invenqor-agent-windows-$ARCH"
PACKAGE="$STAGE/$NAME"

mkdir -p "$PACKAGE/bin" "$PACKAGE/config" "$PACKAGE/scripts"
install -m 0755 "$ROOT/$TARGET_DIR/$TARGET/release/invenqor-agent.exe" "$PACKAGE/bin/"
install -m 0644 "$ROOT/config/config.windows.toml" "$PACKAGE/config/config.toml"
install -m 0644 "$ROOT/packaging/windows/install.ps1" "$PACKAGE/scripts/"
install -m 0644 "$ROOT/packaging/windows/uninstall.ps1" "$PACKAGE/scripts/"
install -m 0644 "$ROOT/packaging/windows/README.txt" "$PACKAGE/"

ARCHIVE="$NAME.zip"
rm -f "$ROOT/$ARCHIVE"
if command -v zip >/dev/null 2>&1; then
    # -X drops extra attributes so the archive is reproducible between builds.
    (cd "$STAGE" && zip -qrX "$ROOT/$ARCHIVE" "$NAME")
elif command -v python3 >/dev/null 2>&1; then
    # The permission bits a zip can carry mean nothing on Windows, where the
    # extension decides what is executable, so the standard library is enough.
    (cd "$STAGE" && python3 -m zipfile -c "$ROOT/$ARCHIVE" "$NAME")
else
    echo "either zip or python3 is required to build the Windows package" >&2
    exit 1
fi
(cd "$ROOT" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
echo "$ROOT/$ARCHIVE"
echo "$ROOT/$ARCHIVE.sha256"
