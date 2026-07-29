#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DOCS="$ROOT/docs"
MARKED_VERSION=${MARKED_VERSION:-18.0.7}

if ! command -v npx >/dev/null 2>&1; then
    echo "npx is required to render Markdown" >&2
    exit 1
fi

BROWSER=${BROWSER:-}
if [ -z "$BROWSER" ]; then
    for candidate in chromium chromium-browser google-chrome google-chrome-stable; do
        if command -v "$candidate" >/dev/null 2>&1; then
            BROWSER=$(command -v "$candidate")
            break
        fi
    done
fi
if [ -z "$BROWSER" ]; then
    echo "Chromium or Google Chrome is required to create PDFs" >&2
    exit 1
fi

BUILD_DIR=$(mktemp -d "$DOCS/.pdf-build.XXXXXX")
trap 'rm -rf "$BUILD_DIR"' EXIT HUP INT TERM

for name in USER_GUIDE ADMIN_GUIDE EXECUTIVE_REPORT SERVER_INSTALLATION API_MCP_GUIDE; do
    markdown="$DOCS/$name.md"
    html="$BUILD_DIR/$name.html"
    pdf="$DOCS/$name.pdf"
    case "$name" in
        USER_GUIDE) title="Invenqor Agent 사용자 가이드" ;;
        ADMIN_GUIDE) title="Invenqor Agent 관리자 가이드" ;;
        EXECUTIVE_REPORT) title="Invenqor Agent 임원 보고서" ;;
        SERVER_INSTALLATION) title="Invenqor Server 설치 및 운영 가이드" ;;
        API_MCP_GUIDE) title="Invenqor 자산 API·MCP·키 관리 가이드" ;;
    esac

    npx --yes "marked@$MARKED_VERSION" "$markdown" --output "$html"
    sed -i "1i <!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><meta name=\"author\" content=\"Invenqor Project\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>$title</title><link rel=\"stylesheet\" href=\"../pdf.css\"></head><body>" "$html"
    sed -i '$a </body></html>' "$html"

    "$BROWSER" \
        --headless \
        --no-sandbox \
        --disable-gpu \
        --disable-dev-shm-usage \
        --no-pdf-header-footer \
        --generate-pdf-document-outline \
        --print-to-pdf="$pdf" \
        "file://$html" >/dev/null 2>&1
    echo "$pdf"
done
