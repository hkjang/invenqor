#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
suffix=$$
network="invenqor-multipod-$suffix"
postgres="invenqor-multipod-postgres-$suffix"
pod_a="invenqor-multipod-a-$suffix"
pod_b="invenqor-multipod-b-$suffix"
pg_volume="invenqor-multipod-pg-$suffix"
state_a="invenqor-multipod-state-a-$suffix"
state_b="invenqor-multipod-state-b-$suffix"
updates="invenqor-multipod-updates-$suffix"
port_a=${INVENQOR_MULTIPOD_PORT_A:-18101}
port_b=${INVENQOR_MULTIPOD_PORT_B:-18102}
work=$(mktemp -d)

cleanup() {
  docker rm -f "$pod_a" "$pod_b" "$postgres" >/dev/null 2>&1 || true
  docker volume rm "$pg_volume" "$state_a" "$state_b" "$updates" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

cd "$root"
docker build -q -t invenqor-server:e2e-multipod .
docker network create "$network" >/dev/null
docker volume create "$pg_volume" >/dev/null
docker volume create "$state_a" >/dev/null
docker volume create "$state_b" >/dev/null
docker volume create "$updates" >/dev/null
for volume in "$state_a" "$state_b" "$updates"; do
  docker run --rm -v "$volume:/data" alpine:3.22 chown 65532:65532 /data
done
head -c 32 /dev/urandom > "$work/master.key"
chmod 0444 "$work/master.key"

docker run -d --name "$postgres" --network "$network" \
  -e POSTGRES_DB=invenqor -e POSTGRES_USER=invenqor \
  -e POSTGRES_PASSWORD=multipod-contract-password \
  -v "$pg_volume:/var/lib/postgresql/data" postgres:17-alpine >/dev/null
for _ in $(seq 1 30); do
  docker exec "$postgres" pg_isready -U invenqor >/dev/null 2>&1 && break
  sleep 1
done

dsn="postgres://invenqor:multipod-contract-password@$postgres:5432/invenqor?sslmode=disable"
run_pod() {
  name=$1
  state=$2
  port=$3
  docker run -d --name "$name" --network "$network" \
    -p "127.0.0.1:$port:7070" \
    -e INVENQOR_POSTGRES_DSN="$dsn" \
    -e INVENQOR_MASTER_KEY_FILE=/run/secrets/master.key \
    -e INVENQOR_UPDATE_DIR=/var/lib/invenqor-updates \
    -v "$state:/var/lib/invenqor-server" \
    -v "$updates:/var/lib/invenqor-updates" \
    -v "$work/master.key:/run/secrets/master.key:ro" \
    invenqor-server:e2e-multipod >/dev/null
}
run_pod "$pod_a" "$state_a" "$port_a" &
run_pod "$pod_b" "$state_b" "$port_b" &
wait

for port in "$port_a" "$port_b"; do
  for _ in $(seq 1 45); do
    curl -fsS "http://127.0.0.1:$port/health/ready" >/dev/null 2>&1 && break
    sleep 1
  done
  curl -fsS "http://127.0.0.1:$port/health/ready" >/dev/null
done

token=
for pod in "$pod_a" "$pod_b"; do
  if docker cp "$pod:/var/lib/invenqor-server/initial-admin.token" "$work/token" \
      >/dev/null 2>&1; then
    token=$(tr -d '\r\n' < "$work/token")
    break
  fi
done
test -n "$token"
curl -fsS -X POST "http://127.0.0.1:$port_a/api/v1/bootstrap/admin" \
  -H "X-Invenqor-Bootstrap-Token: $token" \
  -H 'Content-Type: application/json' \
  -d '{"username":"multipod.admin","password":"CorrectHorse!42","display_name":"Multi Pod Admin"}' \
  >/dev/null
login=$(curl -fsS -c "$work/cookies" -H 'Content-Type: application/json' \
  -d '{"username":"multipod.admin","password":"CorrectHorse!42"}' \
  "http://127.0.0.1:$port_b/api/v1/auth/local/login")
csrf=$(printf '%s' "$login" | jq -r .csrf_token)
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_a/api/v1/auth/me" |
  jq -e '.user.username == "multipod.admin"' >/dev/null

curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_a/api/v1/admin/settings/agent-enrollment" |
  jq -e '.mode == "open" and .source == "database"' >/dev/null
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -X PATCH \
  -d '{"mode":"disabled","reason":"multi-pod enrollment E2E"}' \
  "http://127.0.0.1:$port_a/api/v1/admin/settings/agent-enrollment" |
  jq -e '.mode == "disabled"' >/dev/null
disabled_status=$(curl -sS -o "$work/disabled-enrollment.json" -w '%{http_code}' \
  -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$(cat /proc/sys/kernel/random/uuid)\",\"hostname\":\"multipod-disabled\",\"claim_token\":\"ivq_ec_$(printf 'a%.0s' {1..64})\"}" \
  "http://127.0.0.1:$port_b/v1/agent/enroll")
test "$disabled_status" = 403
disabled_request_id=$(jq -r .request_id "$work/disabled-enrollment.json")
test -n "$disabled_request_id"
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_a/api/v1/admin/diagnostics/logs?q=$disabled_request_id" |
  jq -e '.items[] | select(.event_code == "AGENT_AUTO_ENROLLMENT_DISABLED")' >/dev/null

curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -X PATCH \
  -d '{"mode":"open","reason":"URL-only enrollment E2E"}' \
  "http://127.0.0.1:$port_b/api/v1/admin/settings/agent-enrollment" |
  jq -e '.mode == "open"' >/dev/null
curl -fsS -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$(cat /proc/sys/kernel/random/uuid)\",\"hostname\":\"multipod-open\",\"claim_token\":\"ivq_ec_$(printf 'b%.0s' {1..64})\"}" \
  "http://127.0.0.1:$port_a/v1/agent/enroll" |
  jq -e '.token | startswith("ivq_at_")' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/admin/diagnostics/logs" |
  jq -e '(.instances | length) >= 2 and
    (.items | map(.event_code) | index("AGENT_ENROLLMENT_SUCCEEDED") != null)' >/dev/null

registration_response=$(curl -fsS -b "$work/cookies" \
  -H "X-CSRF-Token: $csrf" -H 'Content-Type: application/json' \
  -d '{"reason":"protected enrollment E2E"}' \
  "http://127.0.0.1:$port_a/api/v1/admin/settings/agent-enrollment/token")
registration_token=$(printf '%s' "$registration_response" | jq -r .registration_token)
printf '%s' "$registration_response" |
  jq -e '.mode == "token" and .shown_once == true' >/dev/null
protected_status=$(curl -sS -o "$work/protected-enrollment.json" -w '%{http_code}' \
  -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$(cat /proc/sys/kernel/random/uuid)\",\"hostname\":\"multipod-protected\",\"claim_token\":\"ivq_ec_$(printf 'c%.0s' {1..64})\"}" \
  "http://127.0.0.1:$port_b/v1/agent/enroll")
test "$protected_status" = 401
curl -fsS -H 'Content-Type: application/json' \
  -H "X-Invenqor-Enrollment-Token: $registration_token" \
  -d "{\"agent_id\":\"$(cat /proc/sys/kernel/random/uuid)\",\"hostname\":\"multipod-token\",\"claim_token\":\"ivq_ec_$(printf 'd%.0s' {1..64})\"}" \
  "http://127.0.0.1:$port_b/v1/agent/enroll" |
  jq -e '.token | startswith("ivq_at_")' >/dev/null

api_key_response=$(curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -d '{"name":"multipod-mcp","scopes":["mcp.access","assets.read"]}' \
  "http://127.0.0.1:$port_a/api/v1/admin/api-keys")
api_key_secret=$(printf '%s' "$api_key_response" | jq -r .secret)
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
  "http://127.0.0.1:$port_b/mcp" |
  jq -e '[.result.tools[].name] | index("asset_search") != null' >/dev/null

migrations=$(docker exec "$postgres" psql -U invenqor -d invenqor -Atc \
  'SELECT COUNT(*) FROM schema_migrations')
test "$migrations" -ge 4
test "$(docker inspect -f '{{.State.Running}}' "$pod_a")" = true
test "$(docker inspect -f '{{.State.Running}}' "$pod_b")" = true
echo "E2E PASS: two server pods shared authentication, Agent policy, assets, and diagnostics"
