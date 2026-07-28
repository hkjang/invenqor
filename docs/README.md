# Invenqor Agent 문서

이 디렉터리는 Invenqor Agent v0.1.0의 역할별 공식 가이드를 제공합니다.

| 문서 | 대상 | Markdown | PDF |
|---|---|---|---|
| 사용자 가이드 | 설치 후 기본 사용과 상태 확인이 필요한 사용자 | [USER_GUIDE.md](USER_GUIDE.md) | [USER_GUIDE.pdf](USER_GUIDE.pdf) |
| 관리자 가이드 | 배포, 설정, 보안, 모니터링, 장애 대응 담당자 | [ADMIN_GUIDE.md](ADMIN_GUIDE.md) | [ADMIN_GUIDE.pdf](ADMIN_GUIDE.pdf) |
| 임원 보고서 | 도입·확대·통제 의사결정자 | [EXECUTIVE_REPORT.md](EXECUTIVE_REPORT.md) | [EXECUTIVE_REPORT.pdf](EXECUTIVE_REPORT.pdf) |

문서의 기준 릴리즈는 `v0.1.0`, 기준일은 2026-07-28입니다. 제품 동작과
문서가 다를 경우 해당 버전의 소스 코드와 설정 검증 결과를 우선하며, 문서 오류는
저장소 이슈로 보고해 주십시오.

PDF를 다시 생성하려면 저장소 루트에서 다음 명령을 실행합니다.

```bash
./scripts/build-docs.sh
```

빌드에는 Node.js 20 이상, `npx`, Chromium 계열 브라우저가 필요합니다.
