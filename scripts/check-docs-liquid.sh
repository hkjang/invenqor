#!/bin/sh
# GitHub Pages publishes docs/ with Jekyll, and Jekyll runs Liquid over every
# Markdown and HTML page before rendering it. A '{%' in the body is a Liquid
# tag opener even inside a code span or a fenced block, so prose that merely
# quotes one fails the whole deployment instead of being published. v0.2.45
# shipped `LIKE '{%'` in a SQL example and the Pages build died with
#
#   Liquid Exception: Liquid syntax error (line 150): Tag '{%' was not properly
#   terminated with regexp: /\%\}/ in RELEASE_NOTES_v0.2.45.md
#
# None of these documents templates anything, so no '{%' here is intentional.
#
# '{{' is deliberately not rejected: Liquid resolves an unknown expression to an
# empty string instead of failing, which is why docs/SERVER_INSTALLATION.md has
# carried '{{.Id}}' for `docker image inspect --format` across many green Pages
# builds. That still renders those two commands wrong on the published site, but
# it is a rendering defect and not the build break this guard exists to stop.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

found=$(
    find docs -type f \
        \( -name '*.md' -o -name '*.markdown' -o -name '*.html' \) \
        -exec grep -n -F -H '{%' {} + || true
)

if [ -n "$found" ]; then
    echo 'A published document contains a Liquid tag opener, which fails the GitHub Pages build:' >&2
    echo "$found" >&2
    echo "Rewrite the text so that it does not contain '{%'." >&2
    exit 1
fi
