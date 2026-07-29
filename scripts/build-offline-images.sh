#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=${1:-0.2.1}
output_dir=${2:-"$root/dist"}
server_image="invenqor-server:$version"
postgres_image="postgres:17-alpine"
archive="$output_dir/invenqor-$version.tar.gz"

mkdir -p "$output_dir"
cd "$root"
docker build --platform linux/amd64 -t "$server_image" .
docker pull --platform linux/amd64 "$postgres_image"
test "$(docker image inspect "$server_image" --format '{{.Architecture}}')" = amd64
test "$(docker image inspect "$postgres_image" --format '{{.Architecture}}')" = amd64
docker save "$server_image" "$postgres_image" | gzip -9 > "$archive"
gzip -t "$archive"
sha256sum "$archive" > "$archive.sha256"
echo "$archive"
