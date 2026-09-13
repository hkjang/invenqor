#!/usr/bin/env bash
# 폐쇄망 배포 묶음에 함께 나가는 것들을 만든다.
#
# 이미지 묶음(invenqor-v<버전>.tar.gz)은 build-offline-images.sh 가 만든다.
# 여기서는 그 묶음을 받는 쪽이 필요한 나머지를 만든다 — 체크섬을 확인하고
# 적재하는 스크립트(리눅스·윈도우), 그 버전에 묶인 compose 파일, 그리고
# 값이 비면 기동이 멈추는 환경 변수의 예시.
#
# 왜 필요한가: .sha256 은 이미 함께 나가지만 그것을 확인하는 것은 아무것도
# 없었고, compose 는 POSTGRES_PASSWORD 가 없으면 곧바로 멈추는데 그 값을
# 무엇으로 채우는지 보여 주는 파일이 없었다. 다른 저장소(ptium)는 둘 다
# 함께 내보낸다.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=${1:?사용법: offline-bundle-assets.sh <버전> [출력 디렉터리]}
out=${2:-"$root/dist"}
mkdir -p "$out"

archive="invenqor-v$version.tar.gz"

# 버전이 박힌 compose. 저장소의 compose.offline.yaml 을 그대로 복사한다 —
# bump-version.sh 가 그 안의 이미지 태그를 이미 맞춰 두었다.
cp "$root/compose.offline.yaml" "$out/compose.invenqor-$version.yaml"

cat > "$out/invenqor-$version.env.example" <<'ENVEOF'
# compose.invenqor-<버전>.yaml 이 읽는 값. 이 파일을 .env 로 복사해 채운다.
#
#   cp invenqor-<버전>.env.example .env
#   docker compose -f compose.invenqor-<버전>.yaml --env-file .env up -d

# 필수. 비어 있으면 기동이 그 자리에서 멈춘다.
# 만드는 법:  openssl rand -hex 32
# 16진수를 권하는 이유는 DSN 에 그대로 들어가기 때문이다. 예약 문자가 든
# 비밀번호를 쓰려면 아래 POSTGRES_DSN 을 직접 적고 비밀번호를 퍼센트
# 인코딩한다.
POSTGRES_PASSWORD=

# 선택. 위 비밀번호로 만든 기본 DSN 을 대신한다.
#POSTGRES_DSN=postgres://invenqor:<퍼센트인코딩한비밀번호>@postgres:5432/invenqor?sslmode=disable

# 선택. 첫 관리자를 만들어 두고 시작한다. 비워 두면 화면에서 만든다.
#BOOTSTRAP_ADMIN=admin@example.com
#BOOTSTRAP_ADMIN_PASSWORD=

# 선택. 웹을 다른 포트로 연다 (기본 7070).
#INVENQOR_WEB_PORT=7070

# 선택. 에이전트 자동 등록 (기본 켜짐).
#AGENT_AUTO_ENROLLMENT=true
#AGENT_ENROLLMENT_TOKEN=
ENVEOF

cat > "$out/load-invenqor-$version.sh" <<LOADEOF
#!/usr/bin/env bash
# 폐쇄망 호스트에서 Invenqor 묶음을 확인하고 적재한다.
set -euo pipefail

archive="\${1:-$archive}"
if [[ ! -f "\$archive" ]]; then
    echo "없는 파일: \$archive" >&2
    echo "사용법: load-invenqor-$version.sh [invenqor-v$version.tar.gz]" >&2
    exit 1
fi

checksum="\$archive.sha256"
if [[ -f "\$checksum" ]]; then
    expected="\$(cut -d' ' -f1 < "\$checksum")"
    actual="\$(sha256sum "\$archive" | cut -d' ' -f1)"
    if [[ "\$expected" != "\$actual" ]]; then
        echo "체크섬이 다릅니다 — 옮기는 중에 깨졌을 수 있습니다." >&2
        echo "  기대  \$expected" >&2
        echo "  실제  \$actual" >&2
        exit 1
    fi
    echo "체크섬 확인: \$actual"
else
    echo "경고: \$checksum 이 없어 확인 없이 적재합니다." >&2
fi

docker load --input "\$archive"
docker images --format '{{.Repository}}:{{.Tag}}\t{{.Size}}' | grep -E '^(invenqor|postgres)' || true

cat <<'NEXT'

다음 단계:
  1. cp invenqor-$version.env.example .env  후 POSTGRES_PASSWORD 를 채운다
     (openssl rand -hex 32)
  2. docker compose -f compose.invenqor-$version.yaml --env-file .env up -d
  3. http://<호스트>:7070 으로 접속

이 묶음에는 invenqor-server 와 postgres 이미지가 함께 들어 있어
compose 는 pull_policy: never 로 뜹니다 — 레지스트리에 닿지 않습니다.
NEXT
LOADEOF
chmod +x "$out/load-invenqor-$version.sh"

cat > "$out/load-invenqor-$version.ps1" <<PSEOF
# 폐쇄망 호스트에서 Invenqor 묶음을 확인하고 적재한다 (Windows).
[CmdletBinding()]
param([string]\$Archive = "$archive")

\$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath \$Archive)) {
    Write-Error "없는 파일: \$Archive"
}

\$checksum = "\$Archive.sha256"
if (Test-Path -LiteralPath \$checksum) {
    \$expected = (Get-Content -LiteralPath \$checksum -Raw).Split(' ')[0].Trim()
    \$actual = (Get-FileHash -LiteralPath \$Archive -Algorithm SHA256).Hash.ToLower()
    if (\$expected -ne \$actual) {
        Write-Host "체크섬이 다릅니다 - 옮기는 중에 깨졌을 수 있습니다." -ForegroundColor Red
        Write-Host "  기대  \$expected"
        Write-Host "  실제  \$actual"
        exit 1
    }
    Write-Host "체크섬 확인: \$actual"
} else {
    Write-Warning "\$checksum 이 없어 확인 없이 적재합니다."
}

docker load --input \$Archive
docker images --format '{{.Repository}}:{{.Tag}}' | Select-String -Pattern '^(invenqor|postgres)'

Write-Host ""
Write-Host "다음 단계:"
Write-Host "  1. copy invenqor-$version.env.example .env  후 POSTGRES_PASSWORD 를 채운다"
Write-Host "  2. docker compose -f compose.invenqor-$version.yaml --env-file .env up -d"
Write-Host "  3. http://<호스트>:7070 으로 접속"
PSEOF

echo "$out/compose.invenqor-$version.yaml"
echo "$out/invenqor-$version.env.example"
echo "$out/load-invenqor-$version.sh"
echo "$out/load-invenqor-$version.ps1"
