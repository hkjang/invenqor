# Invenqor Server 설치 및 운영 가이드

대상 버전: v0.2.0 · 기준일: 2026-07-28

## 1. 수집 내용

Agent는 Linux 호스트에서 다음 원천 레코드를 수집해 `/v1/agent/events`로 전송합니다.

| 영역 | 주요 필드 | 대표 자산 |
|---|---|---|
| 시스템 | 호스트명, 배포판, Kernel, Architecture, Boot time, Timezone | Host |
| CPU/메모리/디스크 | 모델, 코어, 용량, Mount, Filesystem | Hardware component |
| 네트워크 | Interface, MAC, IP, Route, DNS | Network interface |
| 프로세스 | PID, 실행 파일, 사용자, 명령 | Process |
| 패키지 | dpkg/rpm/apk 이름, 버전, Architecture | Software package |
| 서비스 | systemd/OpenRC/SysV 상태와 시작 정책 | Service |
| 계정 | UID/GID, Shell, Group, sudo 관련 정보 | Account |
| 컨테이너 | Docker/Podman/containerd 컨테이너 메타데이터 | Container |

서버는 `Agent UUID + category + asset_id`를 원천 키로 사용하며 대표 자산과
분리 저장합니다. 첫 전송은 Snapshot, 이후 전송은 added/updated/removed
변경분이며 Collector 오류가 있으면 누락을 삭제로 추론하지 않습니다. 원본
이벤트, Snapshot, 변경 전후 값과 Collector 오류를 모두 보존합니다.

## 2. 빠른 설치

### Docker Compose

```bash
git clone https://github.com/hkjang/invenqor.git
cd invenqor
export POSTGRES_PASSWORD='충분히-긴-임의-비밀번호'
docker compose up -d --build
curl http://127.0.0.1:8080/health/ready
```

운영 환경에서는 TLS 종료 Reverse Proxy를 앞에 두십시오. 로그인 Session
Cookie는 `Secure`, `HttpOnly`, `SameSite=Strict`입니다.

### 단일 바이너리와 SQLite

```bash
cd server
go build -trimpath -o invenqor-server ./cmd/invenqor-server
sudo install -m 0755 invenqor-server /usr/local/bin/
sudo install -d -m 0700 -o invenqor -g invenqor /var/lib/invenqor-server
sudo -u invenqor env \
  INVENQOR_LISTEN_ADDRESS=127.0.0.1:8080 \
  INVENQOR_STATE_DIR=/var/lib/invenqor-server \
  /usr/local/bin/invenqor-server
```

DSN이 없거나 형식·DNS·연결·인증 확인에 실패하면 SQLite로 안전하게
기동합니다. SQLite 파일, `master.key`, `bootstrap.enc`, 이벤트 spool은
권한 0600이고 상태 디렉터리는 0700입니다.

## 3. 최초 관리자

서버 로그의 `bootstrap_token_file` 경로에서 일회용 토큰을 읽습니다.

```bash
TOKEN=$(sudo cat /var/lib/invenqor-server/initial-admin.token)
curl -X POST http://127.0.0.1:8080/api/v1/bootstrap/admin \
  -H "X-Invenqor-Bootstrap-Token: $TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"CorrectHorse!42","display_name":"관리자"}'
```

성공 시 토큰 파일은 삭제되고 재사용할 수 없습니다. 브라우저에서 서버 URL을
열어 로컬 로그인하거나 관리자가 구성한 Keycloak Code Flow+PKCE를 사용합니다.

## 4. Agent 연결

관리 콘솔의 Agent 메뉴 또는 `agents.manage` 권한 API에서 Agent UUID를
등록합니다. 응답의 Bearer Token은 한 번만 노출되며 DB에는 SHA-256 해시만
저장됩니다. Agent `config.toml`:

```toml
[server]
url = "https://invenqor.example.com"
bearer_token = "ivq_at_..."
ca_file = "/etc/invenqor-agent/ca.pem"
timeout_seconds = 30
```

mTLS를 사용하면 서버에 SHA-256 인증서 Fingerprint와 만료일을 등록하고
Agent의 `client_identity_pem`을 설정합니다. 토큰 교체 시 0~7일 유예기간을
지정할 수 있고 차단된 Agent의 요청은 즉시 거부됩니다.

## 5. 운영 확인과 복구

| 경로 | 의미 |
|---|---|
| `/health/live` | 프로세스 생존 |
| `/health/ready` | 현재 DB 준비 상태 |
| `/health/database` | `POSTGRES_ACTIVE`, `POSTGRES_DEGRADED`, `SQLITE_FALLBACK` |
| `/api/v1/system/info` | 버전, Commit, Build time, DB 모드 |

PostgreSQL 운영 중 장애는 SQLite 전환을 유발하지 않습니다. 인증 Cache로
검증 가능한 Agent 이벤트만 로컬 append-only 세그먼트에 내구성 있게 기록하며,
DB 복구 후 순서대로 재처리합니다. 관리자 쓰기는 장애 중 성공으로 오인되지
않습니다. 복구 전에는 상태 디렉터리와 DB를 백업하고, 로그에 비밀번호나
Token을 포함하지 마십시오.

## 6. Kubernetes

```bash
kubectl create secret generic invenqor-database \
  --from-literal=dsn='postgres://user:password@host/db?sslmode=require'
helm upgrade --install invenqor deploy/helm/invenqor
```

운영에서는 NetworkPolicy, Ingress TLS, PostgreSQL 백업, PVC Snapshot,
Secret Manager 연동과 PodDisruptionBudget을 추가하십시오.
