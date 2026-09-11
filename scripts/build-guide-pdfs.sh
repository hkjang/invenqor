#!/bin/sh
# Renders docs/USER_GUIDE.pdf and docs/ADMIN_GUIDE.pdf with the shared guide
# tool. The other documents are built by scripts/build-docs.sh; these two carry
# screenshots and follow the cross-repository guide standard, so they use the
# common renderer instead of this repository's own layout.
#
#   GUIDE_TOOL=/path/to/aidev/tools/guide ./scripts/build-guide-pdfs.sh
#
# Markdown links between documents are relative. Chromium freezes those into the
# temporary build path, so every cross-document link in the PDF would point at
# the builder's filesystem and break. They are rewritten to the immutable
# versioned documentation first, exactly as scripts/build-docs.sh does.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
DOCS="$ROOT/docs"
GUIDE_TOOL=${GUIDE_TOOL:-/mnt/c/Users/USER/projects/aidev/tools/guide}
VERSION=${VERSION:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)}
REPOSITORY_URL=${REPOSITORY_URL:-https://github.com/hkjang/invenqor}
PUBLIC_DOCS_URL=${PUBLIC_DOCS_URL:-"$REPOSITORY_URL/blob/v$VERSION/docs"}
PUBLIC_SECURITY_URL=${PUBLIC_SECURITY_URL:-"$REPOSITORY_URL/blob/v$VERSION/SECURITY.md"}

if [ -z "$VERSION" ]; then
    echo "could not determine the release version from Cargo.toml" >&2
    exit 1
fi
if [ ! -f "$GUIDE_TOOL/md2pdf.mjs" ]; then
    echo "shared guide tool not found at $GUIDE_TOOL (set GUIDE_TOOL)" >&2
    exit 1
fi

BUILD_DIR=$(mktemp -d "$DOCS/.guide-build.XXXXXX")
trap 'rm -rf "$BUILD_DIR"' EXIT HUP INT TERM

# The renderer resolves image paths against the Markdown file, so the working
# copy keeps the same relative layout as docs/.
ln -s "$DOCS/assets" "$BUILD_DIR/assets"

for name in USER_GUIDE ADMIN_GUIDE; do
    case "$name" in
        USER_GUIDE)
            title="사용자 가이드"
            subtitle="Agent 설치·상태 확인과 관리 콘솔 일상 사용"
            ;;
        ADMIN_GUIDE)
            title="관리자 가이드"
            subtitle="수집 데이터 사전, 배포·인증·운영 통제와 장애 대응 기준서"
            ;;
    esac

    markdown="$BUILD_DIR/$name.md"
    cp "$DOCS/$name.md" "$markdown"
    # The closing contact block is raw HTML, so both Markdown and href links
    # have to be rewritten. Missing one leaves a file:// link in the PDF.
    for target in USER_GUIDE ADMIN_GUIDE EXECUTIVE_REPORT SERVER_INSTALLATION API_MCP_GUIDE; do
        sed -i "s|]($target.md|]($PUBLIC_DOCS_URL/$target.md|g" "$markdown"
        sed -i "s|href=\"$target.md|href=\"$PUBLIC_DOCS_URL/$target.md|g" "$markdown"
    done
    sed -i "s|](../SECURITY.md|]($PUBLIC_SECURITY_URL|g" "$markdown"
    sed -i "s|href=\"../SECURITY.md|href=\"$PUBLIC_SECURITY_URL|g" "$markdown"
    if grep -Eq '(\]\(|href=")(\.\./)?[^)":]+\.md([#?][^)"]*)?[)"]' "$markdown"; then
        echo "unresolved local Markdown link in $DOCS/$name.md" >&2
        grep -nE '(\]\(|href=")(\.\./)?[^)":]+\.md([#?][^)"]*)?[)"]' "$markdown" >&2
        exit 1
    fi

    node "$GUIDE_TOOL/md2pdf.mjs" "$markdown" "$DOCS/$name.pdf" \
        --title "$title" --subtitle "$subtitle" \
        --project "Invenqor" --version "v$VERSION"
    if grep -a -q '/URI (file://' "$DOCS/$name.pdf"; then
        echo "local file URI leaked into $DOCS/$name.pdf" >&2
        exit 1
    fi
done
