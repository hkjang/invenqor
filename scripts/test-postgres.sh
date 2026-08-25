#!/usr/bin/env bash
# Runs the Go test suite against a real PostgreSQL instead of the SQLite fallback.
#
# The suite defaults to SQLite because it needs nothing installed, and that let
# two defects reach production while every test passed:
#
#   - a duplicate-detection query used LIKE on payload_json. That column is TEXT
#     in SQLite and JSONB in PostgreSQL, which has no LIKE for JSONB, so every
#     ingest failed with SQLSTATE 42883.
#   - reviewing a proposed relation reused one parameter for two columns, and
#     PostgreSQL deduced conflicting types for it, so every approve and reject
#     returned HTTP 500.
#
# SQLite also ignores ASCII case in LIKE where PostgreSQL does not, so a search
# that matches under the fallback can return nothing in production.
#
# Starts a throwaway container, runs the suite against it, and removes it.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
container=${POSTGRES_CONTAINER:-invenqor-pgtest}
port=${POSTGRES_PORT:-55432}
image=${POSTGRES_IMAGE:-postgres:17-alpine}

# A configured credential helper that fails takes every pull down with it, and
# this is a public image that needs no credentials.
if [ -z "${DOCKER_CONFIG:-}" ]; then
  docker_config=$(mktemp -d)
  echo '{}' > "$docker_config/config.json"
  export DOCKER_CONFIG="$docker_config"
fi

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
  [ -n "${docker_config:-}" ] && rm -rf "$docker_config"
}
trap cleanup EXIT

docker rm -f "$container" >/dev/null 2>&1 || true
docker run -d --name "$container" \
  -e POSTGRES_PASSWORD=test -e POSTGRES_DB=invenqor \
  -p "$port:5432" "$image" >/dev/null

ready=0
for _ in $(seq 1 60); do
  if docker exec "$container" pg_isready -U postgres >/dev/null 2>&1; then
    ready=1
    break
  fi
done
[ "$ready" -eq 1 ] || { echo "PostgreSQL did not become ready" >&2; exit 1; }

export INVENQOR_TEST_POSTGRES_DSN="postgres://postgres:test@127.0.0.1:$port/invenqor?sslmode=disable"
cd "$root/server"
echo "running the suite against $image"
go test "$@" ./...
