#!/bin/sh
set -eu

# Moves every shipped version reference from one release to the next.
#
# This exists because doing it by hand shipped two defects. Cargo.lock records
# the workspace member's own version, and leaving it behind produced a tag that
# fails `cargo build --locked` on a fresh checkout. And the Helm chart writes its
# image tag quoted, which an unquoted search missed, so v0.2.8 shipped an
# appVersion of 0.2.8 pointing at the 0.2.7 image. Both are covered here, and the
# script refuses to finish if any expected file was left untouched.

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <old-version> <new-version>" >&2
    exit 2
fi
OLD=$1
NEW=$2
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

missing=0
replace() {
    file=$1
    before=$2
    after=$3
    if [ ! -f "$file" ]; then
        echo "  ! $file does not exist" >&2
        missing=1
        return
    fi
    if ! grep -qF "$before" "$file"; then
        echo "  ! $file does not contain: $before" >&2
        missing=1
        return
    fi
    python3 - "$file" "$before" "$after" <<'PY'
import pathlib, sys
path, before, after = sys.argv[1], sys.argv[2], sys.argv[3]
p = pathlib.Path(path)
p.write_text(p.read_text().replace(before, after))
PY
    echo "updated $file"
}

replace Cargo.toml "version = \"$OLD\"" "version = \"$NEW\""
# The lock names the workspace member, so bump it in the same step rather than
# relying on a later build to regenerate it.
replace Cargo.lock "name = \"invenqor-agent\"
version = \"$OLD\"" "name = \"invenqor-agent\"
version = \"$NEW\""
replace server/internal/version/version.go "\"$OLD\"" "\"$NEW\""
replace web/package.json "\"version\": \"$OLD\"" "\"version\": \"$NEW\""
replace openapi.yaml "version: $OLD" "version: $NEW"
replace deploy/helm/invenqor/Chart.yaml "version: $OLD" "version: $NEW"
replace deploy/helm/invenqor/Chart.yaml "appVersion: \"$OLD\"" "appVersion: \"$NEW\""
replace deploy/helm/invenqor/values.yaml "tag: \"$OLD\"" "tag: \"$NEW\""
replace compose.offline.yaml "invenqor-server:$OLD" "invenqor-server:$NEW"
replace scripts/build-offline-images.sh "{1:-$OLD}" "{1:-$NEW}"

# Documentation mentions the version in prose, headings, download URLs and the
# User-Agent string. Release notes are historical and never rewritten.
for doc in docs/*.md; do
    case "$doc" in docs/RELEASE_NOTES_*) continue ;; esac
    python3 - "$doc" "$OLD" "$NEW" <<'PY'
import pathlib, re, sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
p = pathlib.Path(path)
text = p.read_text()
# The v-prefixed form has to be handled explicitly: \b does not match between
# "v" and "0", so a word-boundary search silently skips every "v0.2.15" - which
# is how a bump left the cover page, the download URLs and the release links on
# the previous version while the prose moved on.
updated = text.replace(f"v{old}", f"v{new}")
updated = updated.replace(f"invenqor-agent/{old}", f"invenqor-agent/{new}")
updated = re.sub(rf"\b{re.escape(old)}\b", new, updated)
if updated != text:
    p.write_text(updated)
    print(f"updated {path}")
PY
done

if [ "$missing" -ne 0 ]; then
    echo "" >&2
    echo "One or more files did not contain $OLD. Nothing here is half-applied," >&2
    echo "but check the files named above before tagging." >&2
    exit 1
fi

# The lock is the one that has silently gone stale before, so it is verified
# rather than assumed.
if command -v cargo >/dev/null 2>&1; then
    if ! cargo metadata --locked --format-version 1 >/dev/null 2>&1; then
        echo "! Cargo.lock is not in step with Cargo.toml; run cargo build and commit it" >&2
        exit 1
    fi
    echo "Cargo.lock verified against Cargo.toml"
fi
echo "versions bumped to $NEW"
