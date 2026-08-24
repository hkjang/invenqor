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
OUTPUT_DIR=${OUTPUT_DIR:-$ROOT}
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT HUP INT TERM
NAME="invenqor-agent-windows-$ARCH"
PACKAGE="$STAGE/$NAME"

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd)

# The installer runs elevated on machines nobody can debug remotely, and it runs
# under Windows PowerShell 5.1, which disagrees with the PowerShell 7 a build
# machine is likely to have. Verify before packaging rather than after shipping.
# Silence is never a pass: if no PowerShell is available the skip is announced.
VERIFY="$ROOT/packaging/windows/verify-scripts.ps1"
if command -v pwsh >/dev/null 2>&1; then
    pwsh -NoProfile -File "$VERIFY"
elif command -v docker >/dev/null 2>&1 && [ "${SKIP_SCRIPT_VERIFY:-}" != "1" ]; then
    # One line: a shell line-continuation inside this quoted PowerShell would
    # reach PowerShell as a literal backslash. Errors are not redirected, so a
    # failure to install the analyzer is not mistaken for a clean result.
    docker run --rm -v "$ROOT:/work:ro" mcr.microsoft.com/powershell:latest pwsh -NoProfile -Command "Install-Module PSScriptAnalyzer -RequiredVersion 1.22.0 -Force -Scope CurrentUser -AllowClobber | Out-Null; & /work/packaging/windows/verify-scripts.ps1 -Path /work/packaging/windows/*.ps1"
else
    echo "WARNING: neither pwsh nor docker is available, so the Windows scripts" >&2
    echo "         were NOT verified against Windows PowerShell 5.1." >&2
    echo "         Set SKIP_SCRIPT_VERIFY=1 to acknowledge this deliberately." >&2
    if [ "${SKIP_SCRIPT_VERIFY:-}" != "1" ]; then
        exit 1
    fi
fi

mkdir -p "$PACKAGE/bin" "$PACKAGE/config" "$PACKAGE/scripts"
install -m 0755 "$ROOT/$TARGET_DIR/$TARGET/release/invenqor-agent.exe" "$PACKAGE/bin/"
install -m 0644 "$ROOT/config/config.windows.toml" "$PACKAGE/config/config.toml"
install -m 0644 "$ROOT/packaging/windows/install.ps1" "$PACKAGE/scripts/"
install -m 0644 "$ROOT/packaging/windows/uninstall.ps1" "$PACKAGE/scripts/"
install -m 0644 "$ROOT/packaging/windows/README.txt" "$PACKAGE/"

# ZIP stores a DOS timestamp and therefore cannot represent dates before 1980.
# Normalize every staged entry and feed zip a sorted path list so two builds of
# the same binary and packaging sources produce the same archive bytes. Honour
# SOURCE_DATE_EPOCH when possible, while clamping older values to ZIP's floor.
PACKAGE_EPOCH=${SOURCE_DATE_EPOCH:-315532800}
case "$PACKAGE_EPOCH" in
    *[!0-9]*|'') echo "SOURCE_DATE_EPOCH must be a non-negative integer" >&2; exit 2 ;;
esac
if [ "$PACKAGE_EPOCH" -lt 315532800 ]; then
    PACKAGE_EPOCH=315532800
fi
find "$PACKAGE" -exec touch -d "@$PACKAGE_EPOCH" {} +

ARCHIVE="$NAME.zip"
rm -f "$OUTPUT_DIR/$ARCHIVE"
if command -v zip >/dev/null 2>&1; then
    # -X drops host-specific extra attributes. UTC plus the sorted input list
    # removes timezone and directory-enumeration differences.
    (cd "$STAGE" && find "$NAME" -print | LC_ALL=C sort | \
        TZ=UTC zip -qX "$OUTPUT_DIR/$ARCHIVE" -@)
elif command -v python3 >/dev/null 2>&1; then
    # Deterministic fallback for minimal builders without the zip utility.
    (cd "$STAGE" && python3 - "$OUTPUT_DIR/$ARCHIVE" "$NAME" "$PACKAGE_EPOCH" <<'PY'
import os
import sys
import time
import zipfile

archive, package, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
timestamp = time.gmtime(epoch)[:6]
paths = [package]
for directory, directories, files in os.walk(package):
    directories.sort()
    files.sort()
    paths.extend(os.path.join(directory, name) for name in directories)
    paths.extend(os.path.join(directory, name) for name in files)

with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
    for path in sorted(paths):
        is_directory = os.path.isdir(path)
        archive_name = path.replace(os.sep, "/") + ("/" if is_directory else "")
        entry = zipfile.ZipInfo(archive_name, timestamp)
        entry.create_system = 3
        entry.external_attr = (os.stat(path).st_mode & 0xFFFF) << 16
        if is_directory:
            entry.external_attr |= 0x10
            output.writestr(entry, b"", compress_type=zipfile.ZIP_STORED)
        else:
            with open(path, "rb") as source:
                output.writestr(
                    entry,
                    source.read(),
                    compress_type=zipfile.ZIP_DEFLATED,
                    compresslevel=9,
                )
PY
    )
else
    echo "either zip or python3 is required to build the Windows package" >&2
    exit 1
fi
(cd "$OUTPUT_DIR" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
echo "$OUTPUT_DIR/$ARCHIVE"
echo "$OUTPUT_DIR/$ARCHIVE.sha256"
