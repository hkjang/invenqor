#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=${1:-0.2.36}
output_dir=${2:-"$root/dist"}
# 이미지 이름과 파일 이름을 다른 저장소와 맞춘다: 태그는 <서비스>:v<버전>,
# 배포 파일은 <서비스>-v<버전>.tar.gz. 예전에는 invenqor-server:0.2.34 와
# invenqor-0.2.34.tar.gz 였는데, 릴리즈 태그가 v0.2.34 라 파일 이름을 태그에서
# 유추하면 어긋났다.
server_image="invenqor:v$version"
postgres_image="postgres:17-alpine"
archive="$output_dir/invenqor-v$version.tar.gz"
archive_name=$(basename "$archive")

mkdir -p "$output_dir"
cd "$root"

# A configured credential helper that fails takes every pull down with it, and
# these are public images that need no credentials at all. Use an empty config
# unless the caller supplied one.
if [ -z "${DOCKER_CONFIG:-}" ]; then
  docker_config=$(mktemp -d)
  echo '{}' > "$docker_config/config.json"
  export DOCKER_CONFIG="$docker_config"
  trap 'rm -rf "$docker_config"' EXIT
fi
# 버전·커밋·빌드 시각을 이미지에 새긴다. 넣지 않으면 떠 있는 컨테이너가
# 언제나 Commit "unknown" 을 알린다.
docker build --platform linux/amd64 \
  --build-arg "VERSION=$version" \
  --build-arg "COMMIT=$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)" \
  --build-arg "BUILT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  -t "$server_image" .

# Refresh the base image, but do not fail the release build when the registry is
# unreachable and the exact tag is already present locally - a broken credential
# helper or an offline build host is not a reason to have no bundle. Say so
# loudly, because the bundle then ships whatever that local image happens to be.
if ! docker pull --platform linux/amd64 "$postgres_image"; then
  if docker image inspect "$postgres_image" >/dev/null 2>&1; then
    echo "WARNING: could not pull $postgres_image; bundling the local copy" >&2
    echo "WARNING: local digest $(docker image inspect "$postgres_image" --format '{{index .RepoDigests 0}}' 2>/dev/null || echo unknown)" >&2
  else
    echo "ERROR: cannot pull $postgres_image and it is not present locally" >&2
    exit 1
  fi
fi

test "$(docker image inspect "$server_image" --format '{{.Architecture}}')" = amd64
test "$(docker image inspect "$postgres_image" --format '{{.Architecture}}')" = amd64
docker save "$server_image" "$postgres_image" | gzip -9 > "$archive"
gzip -t "$archive"
(
  cd "$output_dir"
  sha256sum "$archive_name" > "$archive_name.sha256"
)
echo "$archive"
