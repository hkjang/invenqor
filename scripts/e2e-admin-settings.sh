#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/lib/e2e-wait.sh
. "$root/scripts/lib/e2e-wait.sh"
suffix=$$
network="invenqor-admin-e2e-$suffix"
postgres="invenqor-admin-pg-$suffix"
server="invenqor-admin-server-$suffix"
state_volume="invenqor-admin-state-$suffix"
pg_volume="invenqor-admin-pgdata-$suffix"
port=${INVENQOR_ADMIN_E2E_PORT:-18097}
work=$(mktemp -d)
expected_version=$(jq -r .version "$root/web/package.json")

cleanup() {
  docker container stop "$server" "$postgres" >/dev/null 2>&1 || true
  docker container rm "$server" "$postgres" >/dev/null 2>&1 || true
  docker volume rm "$state_volume" "$pg_volume" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  find "$work" -type f -delete >/dev/null 2>&1 || true
  find "$work" -depth -type d -empty -delete >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "$root"
docker build -q -t invenqor-server:e2e-admin .
docker network create "$network" >/dev/null
docker volume create "$state_volume" >/dev/null
docker volume create "$pg_volume" >/dev/null
docker run -d --name "$postgres" --network "$network" \
  -e POSTGRES_DB=invenqor \
  -e POSTGRES_USER=invenqor \
  -e POSTGRES_PASSWORD=target-e2e-password \
  -v "$pg_volume:/var/lib/postgresql/data" \
  postgres:17-alpine >/dev/null
wait_for_postgres "$postgres" invenqor invenqor

dsn="postgres://invenqor:target-e2e-password@$postgres:5432/invenqor?sslmode=disable"
docker run -d --name "$server" --network "$network" \
  -p "127.0.0.1:$port:7070" \
  -e "postgres_dsn=$dsn" \
  -e bootstrap_admin=actual.admin \
  -e bootstrap_admin_password='ActualAdmin@42-Strong' \
  -v "$state_volume:/var/lib/invenqor-server" \
  invenqor-server:e2e-admin >/dev/null
wait_until 60 "the Server on port $port" \
  curl -fsS "http://127.0.0.1:$port/health/ready"

curl -fsS "http://127.0.0.1:$port/api/v1/system/info" |
  jq -e --arg version "$expected_version" \
    '.server_version == $version and (.database_mode | not)' >/dev/null

login=$(
  curl -fsS -c "$work/admin.cookies" \
    -H 'Content-Type: application/json' \
    -d '{"username":"actual.admin","password":"ActualAdmin@42-Strong"}' \
    "http://127.0.0.1:$port/api/v1/auth/local/login"
)
csrf=$(printf '%s' "$login" | jq -r .csrf_token)
printf '%s' "$login" |
  jq -e '.user.super_admin == true and (.user.permissions | index("users.manage") != null)' >/dev/null
curl -fsS -b "$work/admin.cookies" \
  "http://127.0.0.1:$port/health/database" |
  jq -e '.mode == "POSTGRES_ACTIVE"' >/dev/null
curl -fsS -b "$work/admin.cookies" \
  "http://127.0.0.1:$port/api/v1/admin/system/info" |
  jq -e --arg version "$expected_version" \
    '.server_version == $version and .database_mode == "POSTGRES_ACTIVE"' >/dev/null

postgres_status=$(
  curl -fsS -b "$work/admin.cookies" \
    "http://127.0.0.1:$port/api/v1/admin/settings/postgresql"
)
printf '%s' "$postgres_status" |
  jq -e '.database_mode == "POSTGRES_ACTIVE" and .environment_override == true and .effective.host != ""' >/dev/null
if printf '%s' "$postgres_status" | grep -q 'target-e2e-password'; then
  echo "PostgreSQL status leaked a password" >&2
  exit 1
fi
postgres_payload=$(
  jq -cn --arg dsn "$dsn" \
    '{dsn:$dsn,reason:"actual PostgreSQL settings E2E"}'
)
postgres_saved=$(
  curl -fsS -b "$work/admin.cookies" \
    -H "X-CSRF-Token: $csrf" \
    -H 'Content-Type: application/json' \
    -X PATCH \
    -d "$postgres_payload" \
    "http://127.0.0.1:$port/api/v1/admin/settings/postgresql"
)
if printf '%s' "$postgres_saved" | grep -q 'target-e2e-password'; then
  echo "PostgreSQL save response leaked a password" >&2
  exit 1
fi

created=$(
  curl -fsS -b "$work/admin.cookies" \
    -H "X-CSRF-Token: $csrf" \
    -H 'Content-Type: application/json' \
    -d '{"username":"managed.e2e","display_name":"Managed E2E","email":"managed.e2e@example.test","password":"ManagedE2E@42-Strong","roles":["viewer"],"reason":"actual user E2E"}' \
    "http://127.0.0.1:$port/api/v1/admin/users"
)
user_id=$(printf '%s' "$created" | jq -r .user.id)
printf '%s' "$created" |
  jq -e '.user.local_roles == ["viewer"] and .user.oidc_roles == []' >/dev/null
curl -fsS -b "$work/admin.cookies" \
  -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -X PATCH \
  -d '{"roles":["operator"],"reason":"actual RBAC E2E"}' \
  "http://127.0.0.1:$port/api/v1/admin/users/$user_id" |
  jq -e '.updated == true' >/dev/null
curl -fsS -b "$work/admin.cookies" \
  "http://127.0.0.1:$port/api/v1/admin/users" |
  jq -e '.users[] | select(.username == "managed.e2e") | .local_roles == ["operator"] and .roles == ["operator"]' >/dev/null

missing_secret_status=$(
  curl -sS -o "$work/missing-secret.json" -w '%{http_code}' \
    -b "$work/admin.cookies" \
    -H "X-CSRF-Token: $csrf" \
    -H 'Content-Type: application/json' \
    -X PATCH \
    -d '{"settings":{"enabled":true,"issuer_url":"https://keycloak.example.test","realm":"invenqor","client_id":"invenqor","redirect_uri":"https://invenqor.example.test/api/v1/auth/keycloak/callback","logout_redirect_uri":"","scopes":["openid","profile","email"],"username_claim":"preferred_username","email_claim":"email","name_claim":"name","group_claim":"groups","role_claim":"realm_access.roles","role_mappings":{},"group_mappings":{},"auto_create_users":true,"default_role":"viewer","allowed_email_domains":[],"private_ca_pem":"","last_connection_ok":false},"reason":"missing secret E2E"}' \
    "http://127.0.0.1:$port/api/v1/admin/settings/keycloak"
)
test "$missing_secret_status" = 400
jq -e '.error.code == "KEYCLOAK_SECRET_REQUIRED"' \
  "$work/missing-secret.json" >/dev/null

keycloak_current=$(
  curl -fsS -b "$work/admin.cookies" \
    "http://127.0.0.1:$port/api/v1/admin/settings/keycloak"
)
keycloak_payload=$(
  printf '%s' "$keycloak_current" |
    jq '.settings.role_claim="realm_access.roles" |
        .settings.role_mappings={"inventory-reader":"viewer"} |
        {settings:.settings,client_secret:"actual-client-secret",reason:"actual Keycloak settings E2E"}'
)
curl -fsS -b "$work/admin.cookies" \
  -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -X PATCH \
  -d "$keycloak_payload" \
  "http://127.0.0.1:$port/api/v1/admin/settings/keycloak" |
  jq -e '.saved == true' >/dev/null
keycloak_after=$(
  curl -fsS -b "$work/admin.cookies" \
    "http://127.0.0.1:$port/api/v1/admin/settings/keycloak"
)
printf '%s' "$keycloak_after" |
  jq -e '.client_secret_configured == true and
         .settings.role_claim == "realm_access.roles" and
         .settings.role_mappings["inventory-reader"] == "viewer"' >/dev/null
if printf '%s' "$keycloak_after" | grep -q 'actual-client-secret'; then
  echo "Keycloak settings leaked a Client Secret" >&2
  exit 1
fi

index=$(curl -fsS "http://127.0.0.1:$port/")
printf '%s' "$index" | grep -q '<div id="root"></div>'
asset_path=$(printf '%s' "$index" | sed -n 's/.*src="\([^"]*\.js\)".*/\1/p')
curl -fsS "http://127.0.0.1:$port$asset_path" -o "$work/console.js"
grep -q 'Keycloak OIDC' "$work/console.js"
grep -q '최소 정보 빠른 연동' "$work/console.js"
grep -q '자동 등록 허용 IP / CIDR' "$work/console.js"
grep -q 'Server 진단 로그' "$work/console.js"

echo "E2E PASS: PostgreSQL, bootstrap admin, RBAC, enrollment, Keycloak, diagnostics, and web console"
