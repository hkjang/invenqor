#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
suffix=$$
network="invenqor-e2e-$suffix"
postgres="invenqor-e2e-postgres-$suffix"
server="invenqor-e2e-server-$suffix"
client="invenqor-e2e-agent-$suffix"
update_client="invenqor-e2e-agent-update-$suffix"
bootstrap_client="invenqor-e2e-agent-bootstrap-$suffix"
state_volume="invenqor-e2e-state-$suffix"
pg_volume="invenqor-e2e-pg-$suffix"
agent_volume="invenqor-e2e-agent-state-$suffix"
port=${INVENQOR_E2E_PORT:-18091}
work=$(mktemp -d)

cleanup() {
  docker rm -f "$client" "$update_client" "$bootstrap_client" "$server" "$postgres" >/dev/null 2>&1 || true
  docker volume rm "$state_volume" "$pg_volume" "$agent_volume" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

cd "$root"
CARGO_TARGET_DIR=target-x86_64 RUSTFLAGS="-C target-cpu=x86-64" \
  cross build --locked --release --target x86_64-unknown-linux-musl
docker build -q -t "invenqor-agent:e2e" -f e2e/Dockerfile.agent .
docker build -q --build-arg BASE_IMAGE=quay.io/centos/centos:7 \
  -t "invenqor-agent:e2e-centos7" -f e2e/Dockerfile.agent-enterprise .
docker build -q --build-arg BASE_IMAGE=registry.access.redhat.com/ubi8/ubi:8.10 \
  -t "invenqor-agent:e2e-rhel8" -f e2e/Dockerfile.agent-enterprise .
docker build -q --build-arg BASE_IMAGE=registry.access.redhat.com/ubi9/ubi:9.6 \
  -t "invenqor-agent:e2e-rhel9" -f e2e/Dockerfile.agent-enterprise .
docker build -q --build-arg BASE_IMAGE=ubuntu:22.04 \
  -t "invenqor-agent:e2e-ubuntu2204" -f e2e/Dockerfile.agent-enterprise .
docker build -q --build-arg BASE_IMAGE=ubuntu:24.04 \
  -t "invenqor-agent:e2e-ubuntu2404" -f e2e/Dockerfile.agent-enterprise .
docker build -q -t "invenqor-server:e2e" .
docker network create "$network" >/dev/null
docker volume create "$state_volume" >/dev/null
docker volume create "$pg_volume" >/dev/null
docker run -d --name "$postgres" --network "$network" \
  -e POSTGRES_DB=invenqor -e POSTGRES_USER=invenqor \
  -e POSTGRES_PASSWORD=e2e-contract-password \
  -v "$pg_volume:/var/lib/postgresql/data" postgres:17-alpine >/dev/null
for _ in $(seq 1 30); do
  docker exec "$postgres" pg_isready -U invenqor >/dev/null 2>&1 && break
  sleep 1
done
docker run -d --name "$server" --network "$network" -p "127.0.0.1:$port:7070" \
  -e INVENQOR_POSTGRES_DSN='postgres://invenqor:e2e-contract-password@invenqor-e2e-postgres-'$suffix':5432/invenqor?sslmode=disable' \
  -v "$state_volume:/var/lib/invenqor-server" invenqor-server:e2e >/dev/null
for _ in $(seq 1 30); do
  curl -fsS "http://127.0.0.1:$port/health/ready" >/dev/null 2>&1 && break
  sleep 1
done

docker cp "$server:/var/lib/invenqor-server/initial-admin.token" "$work/token"
bootstrap_token=$(tr -d '\r\n' < "$work/token")
curl -fsS -X POST "http://127.0.0.1:$port/api/v1/bootstrap/admin" \
  -H "X-Invenqor-Bootstrap-Token: $bootstrap_token" \
  -H 'Content-Type: application/json' \
  -d '{"username":"e2e.admin","password":"CorrectHorse!42","display_name":"E2E Admin"}' >/dev/null
login=$(curl -fsS -c "$work/cookies" -H 'Content-Type: application/json' \
  -d '{"username":"e2e.admin","password":"CorrectHorse!42"}' \
  "http://127.0.0.1:$port/api/v1/auth/local/login")
csrf=$(printf '%s' "$login" | jq -r .csrf_token)

# Verify that enrollment itself creates a visible PostgreSQL-backed host asset
# before any inventory event arrives.
enrollment_only_agent_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
curl -fsS -H 'Content-Type: application/json' \
  -d '{"agent_id":"'"$enrollment_only_agent_id"'","hostname":"enrollment-only-host","claim_token":"ivq_ec_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}' \
  "http://127.0.0.1:$port/v1/agent/enroll" > "$work/enrollment-only.json"
jq -e '.token | startswith("ivq_at_")' "$work/enrollment-only.json" >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/assets?limit=200" |
  jq -e --arg key "agent:$enrollment_only_agent_id" \
    '.items[] | select(.asset_key == $key) | .status == "discovered" and .source == "agent_enrollment"' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/diagnostics/logs?q=$enrollment_only_agent_id" |
  jq -e '.items[] | select(.event_code == "AGENT_ENROLLMENT_SUCCEEDED")' >/dev/null

# Verify that a rejected Agent request can be correlated from its response to
# the shared Server diagnostics API without exposing credentials.
rejected=$(curl -sS -H 'Content-Type: application/json' \
  -d '{"agent_id":"not-a-uuid","hostname":"rejected-host","claim_token":"ivq_ec_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}' \
  "http://127.0.0.1:$port/v1/agent/enroll")
rejected_request_id=$(printf '%s' "$rejected" | jq -r .request_id)
test -n "$rejected_request_id"
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/diagnostics/logs?q=$rejected_request_id" |
  jq -e '.items[] | select(.event_code == "INVALID_AGENT_IDENTITY")' >/dev/null

docker volume create "$agent_volume" >/dev/null
mkdir -p "$work/config"

openssl genpkey -algorithm ED25519 -out "$work/update-private.pem" >/dev/null 2>&1
update_public_key=$(
  openssl pkey -in "$work/update-private.pem" -pubout -outform DER |
    tail -c 32 | base64 | tr -d '\r\n'
)
openssl pkeyutl -sign -rawin -inkey "$work/update-private.pem" \
  -in "$root/target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent" \
  -out "$work/update.sig"
update_signature=$(base64 < "$work/update.sig" | tr -d '\r\n')
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -F "artifact=@$root/target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent" \
  -F version=0.2.4 -F channel=stable -F os=linux -F architecture=x86_64 \
  -F "signature=$update_signature" -F rollout_percent=100 \
  "http://127.0.0.1:$port/api/v1/admin/agent-updates" \
  > "$work/update-manifest.json"
jq -e '.version == "0.2.4" and .size > 0' "$work/update-manifest.json" >/dev/null

sed \
  -e 's|# url = .*|url = "http://'"$server"':7070"|' \
  -e 's|enabled = false|enabled = true|' \
  -e 's|heartbeat_seconds = 300|heartbeat_seconds = 1|' \
  -e 's|# public_key = .*|public_key = "'"$update_public_key"'"|' \
  -e 's|state_dir = .*|state_dir = "/var/lib/invenqor-agent"|' \
  "$root/config/config.toml" > "$work/config/config.toml"
chmod 0640 "$work/config/config.toml"
chmod -R a+rX "$work/config"

docker run --rm --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --once > "$work/snapshot.json"
jq -e '.records | length > 0' "$work/snapshot.json" >/dev/null
docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  sh -c 'test -s /state/device-credential.json && test -s /state/enrollment-claim.json'
docker run --name "$update_client" --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --check-update > "$work/update-check.txt"
grep -q 'staged invenqor-agent update 0.2.4' "$work/update-check.txt"
docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  cat /state/updates/pending.json | jq -e '.manifest.version == "0.2.4"' >/dev/null
docker rm "$update_client" >/dev/null
docker run -d --name "$client" --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e >/dev/null
sleep 2
test "$(docker inspect -f '{{.State.Running}}' "$client")" = "true"

assets=$(curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/assets?limit=200")
printf '%s' "$assets" | jq -e '.items | length > 0' >/dev/null
api_key_response=$(curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -d '{"name":"e2e-mcp","scopes":["mcp.access","assets.read"]}' \
  "http://127.0.0.1:$port/api/v1/admin/api-keys")
api_key_id=$(printf '%s' "$api_key_response" | jq -r .api_key.id)
api_key_secret=$(printf '%s' "$api_key_response" | jq -r .secret)
test -n "$api_key_id"
test -n "$api_key_secret"
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/api-keys/$api_key_id" |
  jq -e '.api_key.secret == null and .api_key.prefix != ""' >/dev/null
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  "http://127.0.0.1:$port/api/v1/external/assets?limit=10" |
  jq -e '.items | length > 0' >/dev/null
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -d '{"scopes":["agents.read"]}' \
  "http://127.0.0.1:$port/api/v1/admin/api-keys/$api_key_id/scopes" |
  jq -e '.api_key.scopes | index("agents.read") != null' >/dev/null
mcp_initialize=$(curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"e2e","version":"1"}}}' \
  "http://127.0.0.1:$port/mcp")
printf '%s' "$mcp_initialize" |
  jq -e '.result.protocolVersion == "2025-11-25"' >/dev/null
test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $api_key_secret" -H 'Origin: https://evil.example' \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":99,"method":"tools/list","params":{}}' \
  "http://127.0.0.1:$port/mcp")" = 403
mcp_tools=$(curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  "http://127.0.0.1:$port/mcp")
printf '%s' "$mcp_tools" |
  jq -e '[.result.tools[].name] | index("asset_search") != null and index("agents_list") != null' >/dev/null
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"asset_search","arguments":{"limit":10}}}' \
  "http://127.0.0.1:$port/mcp" |
  jq -e '.result.isError == false and (.result.structuredContent.items | length > 0)' >/dev/null
curl -fsS -X DELETE -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  "http://127.0.0.1:$port/api/v1/admin/api-keys/$api_key_id/scopes/agents.read" |
  jq -e '.api_key.scopes | index("agents.read") == null' >/dev/null
rotated_key_response=$(curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -d '{"grace_seconds":60}' \
  "http://127.0.0.1:$port/api/v1/admin/api-keys/$api_key_id/rotate")
rotated_key_secret=$(printf '%s' "$rotated_key_response" | jq -r .secret)
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  "http://127.0.0.1:$port/api/v1/external/assets?limit=1" >/dev/null
curl -fsS -H "Authorization: Bearer $rotated_key_secret" \
  "http://127.0.0.1:$port/api/v1/external/assets?limit=1" >/dev/null
curl -fsS -X DELETE -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  "http://127.0.0.1:$port/api/v1/admin/api-keys/$api_key_id" >/dev/null
test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $rotated_key_secret" \
  "http://127.0.0.1:$port/api/v1/external/assets?limit=1")" = 401
agents=$(curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/agents")
printf '%s' "$agents" | jq -e '.agents[] | select(.status == "active")' >/dev/null
agent_internal_id=$(printf '%s' "$agents" | jq -r '.agents[] | select(.status == "active") | .id' | head -n1)
device_credential_before=$(docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  sha256sum /state/device-credential.json | awk '{print $1}')
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -d '{"grace_seconds":0}' \
  "http://127.0.0.1:$port/api/v1/admin/agents/$agent_internal_id/tokens/rotate" \
  >/dev/null
sleep 2
docker run --rm --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --once > "$work/recovery-snapshot.json"
jq -e '.records | length > 0' "$work/recovery-snapshot.json" >/dev/null
device_credential_after=$(docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  sha256sum /state/device-credential.json | awk '{print $1}')
test "$device_credential_before" != "$device_credential_after"
curl -fsS "http://127.0.0.1:$port/api/v1/system/info" |
  jq -e '.agent_auto_enrollment == true and .agent_enrollment_mode == "open" and .port == 7070' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/dashboard/statistics" |
  jq -e '.assets.total > 0 and .agents.healthy > 0 and (.collection.daily | length == 7)' >/dev/null
events=$(docker exec "$postgres" psql -U invenqor -d invenqor -Atc \
  'SELECT COUNT(*) FROM agent_events WHERE processing_status='\''processed'\''')
test "$events" -ge 1

run_enterprise_client() {
  image=$1
  label=$2
  volume="invenqor-e2e-$label-state-$suffix"
  bootstrap_name="invenqor-e2e-$label-bootstrap-$suffix"
  config_dir="$work/$label"
  docker volume create "$volume" >/dev/null
  mkdir -p "$config_dir"
  sed \
    -e 's|# url = .*|url = "http://'"$server"':7070"|' \
    "$root/config/config.toml" > "$config_dir/config.toml"
  chmod 0644 "$config_dir/config.toml"
  docker run --name "$bootstrap_name" --network "$network" \
    -v "$config_dir/config.toml:/etc/invenqor-agent/config.toml:ro" \
    -v "$volume:/var/lib/invenqor-agent" "$image" --once \
    > "$config_dir/snapshot.json"
  docker rm "$bootstrap_name" >/dev/null
  jq -e '.records | length > 0' "$config_dir/snapshot.json" >/dev/null
  docker run --rm -v "$volume:/state:ro" alpine:3.22 \
    sh -c 'test -s /state/device-credential.json && test -s /state/enrollment-claim.json'
  docker volume rm "$volume" >/dev/null
  echo "E2E PASS: $label"
}

run_enterprise_client "invenqor-agent:e2e-centos7" "centos7"
run_enterprise_client "invenqor-agent:e2e-rhel8" "rhel8"
run_enterprise_client "invenqor-agent:e2e-rhel9" "rhel9"
run_enterprise_client "invenqor-agent:e2e-ubuntu2204" "ubuntu2204"
run_enterprise_client "invenqor-agent:e2e-ubuntu2404" "ubuntu2404"

echo "E2E PASS: real agent collected and delivered assets to PostgreSQL-backed server"
