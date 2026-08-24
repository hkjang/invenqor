# Invenqor Agents @VERSION@

이 묶음은 오프라인망에 반입할 수 있는 Invenqor Agent 전체 배포본입니다. Server Docker
이미지 묶음 `invenqor-@VERSION@.tar.gz`와 분리되어 있어, 자산 장비에는 필요한 Agent
패키지만 전달하면 됩니다.

## 포함 파일

- `invenqor-agent-linux-x86_64.tar.gz` — CentOS 7, RHEL 8/9, Ubuntu 22.04/24.04
- `invenqor-agent-linux-aarch64.tar.gz` — Linux aarch64
- `invenqor-agent-windows-x86_64.zip` — Windows x64 서비스 설치본
- 각 패키지의 `.sha256` — 반입·복사 무결성 확인값
- `sign-agent-update-manifest-v2.py` — 중앙 자동 업데이트 게시용 오프라인 이중 서명 도구

## 설치 순서

1. 대상 패키지와 같은 이름의 `.sha256`을 한 디렉터리에 둡니다.
2. Linux는 `sha256sum -c <파일>.sha256`, Windows는 `Get-FileHash -Algorithm SHA256`으로
   무결성을 확인합니다.
3. 압축을 풀고 Linux는 `sudo ./scripts/install.sh`, Windows는 관리자 PowerShell에서
   `.\scripts\install.ps1`을 실행합니다.
4. `config.toml`의 `[server] url`에 Invenqor 주소만 지정합니다. Server가 URL-only 자동
   등록 모드라면 별도 provisioned device token 없이 첫 통신에서 장비 전용 자격 증명을
   발급·보관합니다.
5. Linux는 `sudo systemctl status invenqor-agent`, Windows는
   `Get-Service invenqor-agent`로 기동 상태를 확인합니다.

세부 수집 항목, 프록시·사설 CA, 장애 진단과 자동 업데이트 절차는 같은 GitHub 릴리즈의
`USER_GUIDE`와 `ADMIN_GUIDE`(MD/PDF)를 따르십시오.
