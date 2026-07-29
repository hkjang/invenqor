# Invenqor Agent

Invenqor는 Linux 자산 수집 Agent와 Go 기반 중앙 Server, React 관리 콘솔을
제공합니다. Server는 PostgreSQL Primary와 안전한 SQLite 기동 대체 모드,
로컬/Keycloak 인증, RBAC, 자산 정규화·이력·관계, Query DSL과 장애 이벤트
spool을 포함합니다.

빠른 시작:

```bash
export POSTGRES_PASSWORD='change-me-with-a-long-random-value'
export BOOTSTRAP_ADMIN='admin'
export BOOTSTRAP_ADMIN_PASSWORD='ChangeMe-With-A-Strong-Password-42!'
docker compose up -d --build
open http://127.0.0.1:7070
```

상세 절차는 [Server 설치 및 운영 가이드](docs/SERVER_INSTALLATION.md),
[사용자 가이드](docs/USER_GUIDE.md), [관리자 가이드](docs/ADMIN_GUIDE.md),
[임원 보고서](docs/EXECUTIVE_REPORT.md),
[자산 API·MCP·키 관리 가이드](docs/API_MCP_GUIDE.md)를 참조하십시오.

Invenqor Agent는 외부 언어 런타임 없이 여러 Linux 배포판에서 실행되는 자산 수집
에이전트입니다. Linux의 `/proc`, `/sys`, `/etc`를 우선 사용하고, 사용할 수 없는
기능은 해당 수집기만 실패 처리하는 점진적 기능 저하(progressive degradation)를
기본 원칙으로 합니다.

현재 버전은 운영 가능한 1~3단계 기반을 제공합니다.

- 설치 시 생성하고 `0600`으로 보존하는 Agent UUID
- Server URL만으로 장비별 Token을 자동 발급하는 zero-touch 등록
- OS, CPU, 메모리, 파일시스템, 네트워크, 프로세스, 패키지, 서비스, 계정,
  컨테이너 환경 수집
- 수집기별 장애 격리와 공통 JSON 스키마
- 안정적인 스냅샷 해시를 이용한 변경 감지
- 전송 완료 전까지 삭제하지 않는 크기 제한 JSONL 큐
- 지수 백오프가 적용된 outbound-only HTTPS 전송
- 웹·관리 API·Agent 통신을 통합한 단일 기본 포트 TCP 7070
- rustls, 사설 CA, 장비별 mTLS PEM 또는 장비별 bearer token
- Ed25519 서명·SHA-256·단계적 rollout 기반 Agent 자동 업데이트
- PostgreSQL advisory migration lock과 공용 Secret을 사용하는 K8s 멀티 파드
- 연결 테스트·암호화 저장을 제공하는 PostgreSQL 및 Keycloak OIDC 설정 화면
- 재기동 없이 URL-only/Token 보호/차단을 전환하고 등록 Token을 발급·회전·
  폐기하는 DB 기반 Agent 등록 설정 화면
- 로컬/SSO 역할 원천 분리, 계정 잠금·세션 폐기와 안전장치를 갖춘 사용자 관리
- 자산 최신성·Agent 건전성·수집 실패·7일 추이를 제공하는 운영 통계 화면
- 자산·관계·병합/분리·Query·감사·키·설정 API를 실제로 연결한 관리 콘솔
- 로그인 화면과 콘솔 상단의 실행 Server 버전 표시
- scoped API key 수명주기와 stateless Streamable HTTP MCP 자산 도구
- systemd, SysV init, OpenRC 서비스 정의
- x86_64와 aarch64용 musl 정적 빌드 구성

## 빠른 실행

Rust 1.85 이상이 설치된 개발 환경에서:

```bash
cargo test
cargo run -- --config config/config.toml --once
```

기본 설정에는 서버 URL이 없습니다. 이 경우 스냅샷을 표준 출력에 표시하고 전송
이벤트는 로컬 큐에 안전하게 남깁니다. 개발 머신에서 `/var/lib`를 쓸 수 없다면
설정 복사본의 `state_dir`을 사용자가 쓸 수 있는 절대 경로로 바꾸십시오.

설정 검증과 전체 기본값 확인:

```bash
cargo run -- --config config/config.toml --validate-config
cargo run -- --print-default-config
```

로그 수준은 `RUST_LOG`로 설정합니다. 토큰, 인증서 원문, 프로세스 명령행은
로그에 기록하지 않습니다.

## 역할별 문서

- [사용자 가이드](docs/USER_GUIDE.md): 패키지 선택, 설치, 최초 설정, 상태 확인,
  일상 사용과 기본 문제 해결
- [관리자 가이드](docs/ADMIN_GUIDE.md): 전체 수집 필드, 인증·전송 계약, 대규모
  배포, 보안 통제, 모니터링, 업그레이드와 장애 대응
- [임원 보고서](docs/EXECUTIVE_REPORT.md): 도입 가치, 통제 경계, 위험, 단계별
  확산안과 의사결정 항목

각 문서는 [PDF 형식](docs/README.md)으로도 제공합니다.

## 수집 데이터

| 수집기 | 기본 소스 | 지원 불가 시 동작 |
|---|---|---|
| OS | `/etc/os-release`, `/proc` | 오류 레코드 |
| CPU·메모리 | `/proc/cpuinfo`, `/proc/meminfo` | 오류 레코드 |
| 디스크 | `/proc/self/mounts`, `statvfs` | 읽을 수 있는 마운트만 반환 |
| 네트워크 | `getifaddrs`, `/sys/class/net`, `/proc/net` | 부분 반환 |
| 프로세스 | `/proc/[pid]` | 종료/권한 거부 PID만 제외 |
| DEB·APK | 패키지 DB 직접 읽기 | 수집기 제외 |
| RPM | 형식 차이를 처리하기 위한 고정 인자 `rpm -qa` | 수집기 오류 |
| 서비스 | systemd/OpenRC 고정 조회 또는 init 디렉터리 | 기능별 부분 반환 |
| 계정 | `/etc/passwd`, `/etc/group` | 수집기 오류 |
| 컨테이너 | 런타임 소켓, cgroup 표식 | 탐지 결과만 반환 |

프로세스 명령행은 비밀번호와 토큰을 포함할 수 있으므로 기본적으로 수집하지
않습니다. 취약점/CVE 매핑, 위험도 계산, 정책 판정은 에이전트가 아니라 중앙
서버의 역할입니다.

## 게이트웨이 계약

에이전트는 등록된 URL의 다음 경로로 이벤트를 전송합니다.

```text
POST {server.url}/v1/agent/events
Content-Type: application/json
Authorization: Bearer ...             # 자동 저장된 장비 Token 또는 수동 bearer_token
X-Invenqor-Agent-Id: <uuid>
X-Invenqor-Event-Id: <uuid>
```

첫 인벤토리는 `kind: "inventory"`와 전체 정규화 `snapshot`을 보냅니다. 이후에는
안정적인 `asset_id`별 `changes` 배열에 `added`, `updated`, `removed`만 보냅니다.
수집기 오류가 있는 주기에는 누락된 레코드를 삭제로 판정하지 않습니다. 바뀌지
않으면 설정된 주기에 `kind: "heartbeat"`를 보냅니다. 이벤트 ID는 재시도 중
유지되므로 게이트웨이는 이 값을 idempotency key로 사용해야 합니다. 성공 응답:

```json
{
  "accepted": true,
  "policy_version": "2026-07-29.1"
}
```

2xx와 `accepted: true`가 모두 확인된 뒤에만 큐 파일을 삭제합니다. 네트워크
오류, TLS 오류, 비-2xx 응답, 잘못된 응답은 모두 재시도 대상입니다. 게이트웨이
응답의 정책 버전은 현재 관찰만 하며 원격 명령을 실행하지 않습니다.

## 정적 빌드와 패키지

재현 가능한 릴리스 빌드는 보수적인 CPU 기준을 명시합니다.

```bash
cargo install cross --locked
./scripts/build-release.sh
./packaging/build-tar.sh x86_64-unknown-linux-musl
./packaging/build-tar.sh aarch64-unknown-linux-musl
```

결과 바이너리는 다음처럼 확인합니다.

```bash
file target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent
ldd target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent
```

tar 패키지는 `bin`, 기본 설정, 설치/제거 스크립트, 세 가지 init 정의를
포함하며 같은 이름의 `.sha256` 체크섬도 생성합니다. 설치:

```bash
tar -xzf invenqor-agent-linux-x86_64.tar.gz
sudo ./invenqor-agent-linux-x86_64/scripts/install.sh
```

설치 스크립트는 비로그인 `invenqor-agent` 계정을 만들고 상태 디렉터리만 그
계정에 쓰기 허용합니다. 기존 설정은 덮어쓰지 않습니다. 제거 스크립트도 감사와
복구를 위해 설정 및 미전송 큐를 보존합니다.

## 지원 기준

| 등급 | 대상 | 범위 |
|---|---|---|
| 정식 목표 | Kernel 3.10+, RHEL 계열 7+, Ubuntu 18.04+, Debian 10+ | 전체 기본 수집 |
| 호환 목표 | Alpine, Amazon Linux, SUSE | 핵심 수집, init별 차이 허용 |
| 제한 목표 | Kernel 2.6 계열, CentOS 6 등 | `/proc` 기반 핵심 수집 |
| 미지원 | Kernel 2.4, Linux 이외 Unix | 실행 보장 없음 |

CPU 아키텍처마다 별도 바이너리가 필요합니다. x86_64 빌드는 `target-cpu=x86-64`,
aarch64 빌드는 `target-cpu=generic`으로 빌드하여 AVX2 같은 최신 명령어를
필수 조건으로 만들지 않습니다. 정적 링크도 실제 사용하는 시스템 호출보다 오래된
커널에서의 실행을 보장하지는 않습니다.

## 보안 경계

- 일반 사용자로 실행하고 capability 및 ambient capability를 부여하지 않습니다.
- systemd에서는 쓰기 경로, 주소 패밀리, privilege escalation을 제한합니다.
- HTTP는 `allow_insecure_http=true`를 명시한 격리 테스트에서만 허용하고
  운영 기본값은 HTTPS입니다.
- 사설 CA와 mTLS를 함께 사용하려면 `ca_file`과
  `client_identity_pem`을 설정합니다. 개인키가 든 PEM은 `0600`으로
  관리하십시오. 패키지 기본 설정은 실행 계정이 읽을 수 있도록
  `root:invenqor-agent`와 `0640`으로 설치됩니다.
- 원격 셸과 임의 명령 실행 기능은 없습니다. 이 버전에서 외부 명령 fallback은
  `rpm`, `systemctl`, `rc-status`에 고정된 인자만 전달합니다.
- 큐가 설정 용량에 도달하면 기존 미전송 데이터를 삭제하지 않고 새 이벤트 생성을
  실패시켜 데이터 손실을 명시적으로 드러냅니다.

원격 작업 기능은 제공하지 않습니다. 자동 업데이트의 privileged helper는
서명 검증을 통과해 스테이징된 파일만 원자 교체하고 이전 바이너리를 보존합니다.
서명 개인키는 반드시 오프라인으로 관리하고 canary rollout 후 확대하십시오.

## 프로젝트 구조

```text
src/
  collectors/       독립적으로 실패 가능한 자산 수집기
  config.rs          엄격한 TOML 설정과 검증
  identity.rs        영속 Agent UUID와 보조 식별자
  storage.rs         변경 해시와 JSONL 전송 큐
  transport.rs       rustls HTTPS/mTLS 클라이언트
  scheduler.rs       수집, heartbeat, 전송, backoff
packaging/
  systemd/ sysv/ openrc/
  scripts/           설치와 제거
config/              안전한 기본 설정
```
