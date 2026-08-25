#!/usr/bin/env bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
# shellcheck source=scripts/lib/e2e-wait.sh
. "$root/scripts/lib/e2e-wait.sh"
suffix=$$
network="invenqor-multipod-$suffix"
postgres="invenqor-multipod-postgres-$suffix"
pod_a="invenqor-multipod-a-$suffix"
pod_b="invenqor-multipod-b-$suffix"
pg_volume="invenqor-multipod-pg-$suffix"
state_a="invenqor-multipod-state-a-$suffix"
state_b="invenqor-multipod-state-b-$suffix"
updates="invenqor-multipod-updates-$suffix"
event_spool="invenqor-multipod-event-spool-$suffix"
port_a=${INVENQOR_MULTIPOD_PORT_A:-18101}
port_b=${INVENQOR_MULTIPOD_PORT_B:-18102}
work=$(mktemp -d)

cleanup() {
  docker rm -f "$pod_a" "$pod_b" "$postgres" >/dev/null 2>&1 || true
  docker volume rm "$pg_volume" "$state_a" "$state_b" "$updates" "$event_spool" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

# Without this a pod that fails to start reports only that nothing answered on
# its port; the reason is in the container's own log, which the cleanup trap
# then removes. Print it before anything is torn down.
report_failure() {
  status=$?
  echo "E2E FAIL: command at line $1 exited with status $status" >&2
  for pod in "$pod_a" "$pod_b" "$postgres"; do
    if docker inspect "$pod" >/dev/null 2>&1; then
      echo "E2E $pod state: $(docker inspect -f '{{.State.Status}} exit={{.State.ExitCode}}' "$pod")" >&2
      echo "E2E $pod log tail:" >&2
      docker logs --tail 80 "$pod" >&2 2>&1 || true
    else
      echo "E2E $pod was never created" >&2
    fi
  done
  exit "$status"
}
trap 'report_failure "$LINENO"' ERR

cd "$root"
docker build -q -t invenqor-server:e2e-multipod .
docker network create "$network" >/dev/null
docker volume create "$pg_volume" >/dev/null
docker volume create "$state_a" >/dev/null
docker volume create "$state_b" >/dev/null
docker volume create "$updates" >/dev/null
docker volume create "$event_spool" >/dev/null
# Model CSI mount roots exactly: root owns the directory, fsGroup can write it,
# and the non-root Server cannot chmod the mount point. The application must
# accept this private 0770 layout while retaining 0600 for files it creates.
for volume in "$state_a" "$state_b" "$updates" "$event_spool"; do
  docker run --rm -v "$volume:/data" alpine:3.22 \
    sh -c 'chown root:65532 /data && chmod 0770 /data'
  test "$(docker run --rm -v "$volume:/data" alpine:3.22 stat -c '%u:%g %a' /data)" = \
    "0:65532 770"
done
head -c 32 /dev/urandom > "$work/master.key"
chmod 0444 "$work/master.key"
openssl genpkey -algorithm ED25519 -out "$work/update-private.pem" >/dev/null 2>&1
update_public_key=$(
  openssl pkey -in "$work/update-private.pem" -pubout -outform DER |
    tail -c 32 | base64 | tr -d '\r\n'
)

docker run -d --name "$postgres" --network "$network" \
  -e POSTGRES_DB=invenqor -e POSTGRES_USER=invenqor \
  -e POSTGRES_PASSWORD=multipod-contract-password \
  -v "$pg_volume:/var/lib/postgresql/data" postgres:17-alpine >/dev/null
wait_for_postgres "$postgres" invenqor invenqor

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
    -e INVENQOR_EVENT_SPOOL_DIR=/var/lib/invenqor-event-spool \
    -e INVENQOR_UPDATE_PUBLIC_KEY="$update_public_key" \
    -v "$state:/var/lib/invenqor-server" \
    -v "$updates:/var/lib/invenqor-updates" \
    -v "$event_spool:/var/lib/invenqor-event-spool" \
    -v "$work/master.key:/run/secrets/master.key:ro" \
    invenqor-server:e2e-multipod >/dev/null
}
run_pod "$pod_a" "$state_a" "$port_a" &
run_pod "$pod_b" "$state_b" "$port_b" &
wait

for port in "$port_a" "$port_b"; do
  wait_until 60 "the Server on port $port" \
    curl -fsS "http://127.0.0.1:$port/health/ready"
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
multipod_agent_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
curl -fsS -H 'Content-Type: application/json' \
  -d "{\"agent_id\":\"$multipod_agent_id\",\"hostname\":\"multipod-open\",\"claim_token\":\"ivq_ec_$(printf 'b%.0s' {1..64})\"}" \
  "http://127.0.0.1:$port_a/v1/agent/enroll" > "$work/open-enrollment.json"
multipod_agent_token=$(jq -r .token "$work/open-enrollment.json")
test "${multipod_agent_token#ivq_at_}" != "$multipod_agent_token"

# An inventory written through one pod must be immediately readable as the
# same normalized software product through another pod. This covers both the
# shared PostgreSQL transaction and the host-scoped runs_on relationship.
multipod_event_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
multipod_now=$(date +%s)
jq -n --arg agent "$multipod_agent_id" --arg event "$multipod_event_id" \
  --argjson now "$multipod_now" \
  '{schema_version:1,event_id:$event,agent_id:$agent,created_at:$now,
    kind:"inventory",snapshot_hash:"multipod-software-e2e",changes:[],collection_errors:[],
    snapshot:{schema_version:1,agent_id:$agent,collected_at:$now,duration_ms:10,errors:[],records:[
      {asset_id:"multipod-host",category:"system",source:"e2e",collected_at:$now,
       payload:{hostname:"multipod-open",os_family:"linux",architecture:"x86_64",os_release:{pretty_name:"E2E Linux"}}},
      {asset_id:"multipod-nginx-service",category:"service",source:"e2e",collected_at:$now,
       payload:{name:"nginx.service",active_state:"active",sub_state:"running"}},
      {asset_id:"multipod-nginx-package",category:"software.package",source:"e2e",collected_at:$now,
       payload:{name:"nginx",version:"1.26.2"}}
    ]}}' > "$work/multipod-event.json"
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  -H "X-Invenqor-Agent-Id: $multipod_agent_id" \
  -H "X-Invenqor-Event-Id: $multipod_event_id" \
  -H 'Content-Type: application/json' --data-binary "@$work/multipod-event.json" \
  "http://127.0.0.1:$port_a/v1/agent/events" |
  jq -e '.accepted == true' >/dev/null
# Warm the bounded credential cache on the second Pod as well. During a DB
# outage, an already authenticated Agent can then receive a durable 202 instead
# of losing an event merely because the load balancer selected either Pod.
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  -H "X-Invenqor-Agent-Id: $multipod_agent_id" \
  -H "X-Invenqor-Event-Id: $multipod_event_id" \
  -H 'Content-Type: application/json' --data-binary "@$work/multipod-event.json" \
  "http://127.0.0.1:$port_b/v1/agent/events" |
  jq -e '.accepted == true and .duplicate == true' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/assets/software-products?q=NGINX" |
  jq -e '.items[] | select(.product_key == "nginx") |
    .host.name == "multipod-open" and .runtime_state == "running"' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/admin/diagnostics/logs" |
  jq -e '(.instances | length) >= 2 and
    (.items | map(.event_code) | index("AGENT_ENROLLMENT_SUCCEEDED") != null)' >/dev/null

# Both Pods share one RWX spool. Accept two equal-agent-time events while
# PostgreSQL is down, remove the Pod that accepted the first event, then prove
# the surviving Pod replays both in durable server-arrival order. This models a
# StatefulSet scale-down without orphaning an acknowledged RWO-local segment.
spooled_first_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
spooled_second_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
spooled_now=$(date +%s)
for item in "first:$spooled_first_id:$port_a" "second:$spooled_second_id:$port_b"; do
  label=${item%%:*}
  remainder=${item#*:}
  event_id=${remainder%%:*}
  jq -n --arg agent "$multipod_agent_id" --arg event "$event_id" \
    --arg hostname "multipod-spooled-$label" --argjson now "$spooled_now" \
    '{schema_version:1,event_id:$event,agent_id:$agent,created_at:$now,
      kind:"inventory",snapshot_hash:("multipod-spooled-"+$hostname),
      changes:[],collection_errors:[],snapshot:{schema_version:1,agent_id:$agent,
      collected_at:$now,duration_ms:10,errors:[],records:[
        {asset_id:"multipod-host",category:"system",source:"e2e",collected_at:$now,
         payload:{hostname:$hostname,os_family:"linux",architecture:"x86_64",
         os_release:{pretty_name:"E2E Linux"}}}]}}' > "$work/spooled-$label.json"
done
docker stop "$postgres" >/dev/null
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  -H "X-Invenqor-Agent-Id: $multipod_agent_id" \
  -H "X-Invenqor-Event-Id: $spooled_first_id" \
  -H 'Content-Type: application/json' --data-binary "@$work/spooled-first.json" \
  "http://127.0.0.1:$port_a/v1/agent/events" |
  jq -e '.accepted == true and .spooled == true' >/dev/null
sleep 1
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  -H "X-Invenqor-Agent-Id: $multipod_agent_id" \
  -H "X-Invenqor-Event-Id: $spooled_second_id" \
  -H 'Content-Type: application/json' --data-binary "@$work/spooled-second.json" \
  "http://127.0.0.1:$port_b/v1/agent/events" |
  jq -e '.accepted == true and .spooled == true' >/dev/null
pending_spool=$(docker run --rm -v "$event_spool:/data" alpine:3.22 \
  sh -c 'find /data -maxdepth 1 -type f -name "*.json" | wc -l')
test "$pending_spool" = 2
docker rm -f "$pod_a" >/dev/null
docker start "$postgres" >/dev/null
wait_for_postgres "$postgres" invenqor invenqor
for _ in $(seq 1 45); do
  replayed=$(docker exec "$postgres" psql -U invenqor -d invenqor -Atc \
    "SELECT COUNT(*) FROM agent_events WHERE event_id IN ('$spooled_first_id','$spooled_second_id') AND processing_status='processed'")
  test "$replayed" = 2 && break
  sleep 1
done
test "$replayed" = 2
test "$(docker exec "$postgres" psql -U invenqor -d invenqor -Atc \
  "SELECT hostname FROM agents WHERE agent_id='$multipod_agent_id'")" = \
  "multipod-spooled-second"
test "$(docker run --rm -v "$event_spool:/data" alpine:3.22 \
  sh -c 'find /data -maxdepth 1 -type f -name "*.json" | wc -l')" = 0

# Scale the removed ordinal back up and ensure both Pods can still mount the
# root-owned state/update/spool roots without gaining privileged ownership.
run_pod "$pod_a" "$state_a" "$port_a"
wait_until 60 "the Server on port $port_a" \
  curl -fsS "http://127.0.0.1:$port_a/health/ready"
curl -fsS "http://127.0.0.1:$port_a/health/ready" >/dev/null

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

# A Client Secret saved through one Pod must be encrypted in PostgreSQL and
# immediately usable/readable as configured through another Pod. Dedicated OIDC
# keys must never leak through or be writable through the generic settings API.
keycloak_current=$(curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_a/api/v1/admin/settings/keycloak")
keycloak_payload=$(printf '%s' "$keycloak_current" | jq \
  '.settings.role_claim="realm_access.roles" |
   {settings:.settings,client_secret:"multipod-client-secret",reason:"multi-pod OIDC E2E"}')
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -X PATCH -d "$keycloak_payload" \
  "http://127.0.0.1:$port_a/api/v1/admin/settings/keycloak" |
  jq -e '.saved == true' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/admin/settings/keycloak" |
  jq -e '.client_secret_configured == true and
         .settings.role_claim == "realm_access.roles"' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/admin/settings" |
  jq -e '[.items[].key | select(. == "auth.keycloak" or
         . == "auth.keycloak.client_secret")] | length == 0' >/dev/null
dedicated_status=$(curl -sS -o "$work/dedicated-setting.json" -w '%{http_code}' \
  -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -X PATCH \
  -d '{"settings":[{"key":"auth.keycloak","value":{"enabled":false},"reason":"must fail"}]}' \
  "http://127.0.0.1:$port_b/api/v1/admin/settings")
test "$dedicated_status" = 409
jq -e '.error.code == "DEDICATED_SETTING_ENDPOINT"' "$work/dedicated-setting.json" >/dev/null
sealed_secret=$(docker exec "$postgres" psql -U invenqor -d invenqor -Atc \
  "SELECT value_json::text FROM settings WHERE key='auth.keycloak.client_secret'")
test -n "$sealed_secret"
if printf '%s' "$sealed_secret" | grep -q 'multipod-client-secret'; then
  echo "E2E FAIL: shared Keycloak Client Secret was stored in plaintext" >&2
  exit 1
fi

# Publish through one Pod and read/download through the other. Besides the
# advisory publication lock, this verifies that a root:fsGroup 0770 RWX mount
# is usable by the non-root image while all release files remain shared.
"$root/scripts/sign-agent-update-manifest-v2.py" \
  --artifact "$work/master.key" --private-key "$work/update-private.pem" \
  --version 98.0.0 --channel stable --os linux --architecture x86_64 \
  > "$work/multipod-update-signature.json"
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -F "artifact=@$work/master.key" \
  -F "signature_bundle_file=@$work/multipod-update-signature.json;type=application/json" \
  -F rollout_percent=100 \
  "http://127.0.0.1:$port_a/api/v1/admin/agent-updates" |
  jq -e '.version == "98.0.0" and .signature_verified == true' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port_b/api/v1/admin/agent-updates" |
  jq -e '.releases[] | select(.version == "98.0.0") |
    .signature_verified == true' >/dev/null
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  "http://127.0.0.1:$port_b/v1/agent/updates?agent_id=$multipod_agent_id&channel=stable&os=linux&arch=x86_64&current_version=0.2.15" \
  > "$work/multipod-update-manifest.json"
multipod_download_url=$(jq -er \
  '.download_url | select(startswith("/v1/agent/updates/"))' \
  "$work/multipod-update-manifest.json")
curl -fsS -H "Authorization: Bearer $multipod_agent_token" \
  "http://127.0.0.1:$port_b$multipod_download_url" \
  > "$work/downloaded-update"
test "$(sha256sum "$work/downloaded-update" | cut -d ' ' -f1)" = \
  "$(sha256sum "$work/master.key" | cut -d ' ' -f1)"

api_key_response=$(curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -d '{"name":"multipod-mcp","scopes":["mcp.access","assets.read"]}' \
  "http://127.0.0.1:$port_a/api/v1/admin/api-keys")
api_key_id=$(printf '%s' "$api_key_response" | jq -r .api_key.id)
api_key_original_secret=$(printf '%s' "$api_key_response" | jq -r .secret)
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' -X PATCH \
  -d '{"name":"multipod-mcp-rotated","scopes":["mcp.access","assets.read","agents.read"]}' \
  "http://127.0.0.1:$port_b/api/v1/admin/api-keys/$api_key_id" |
  jq -e '.api_key.name == "multipod-mcp-rotated" and
    (.api_key.scopes | index("agents.read") != null)' >/dev/null
api_key_rotation=$(curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -H 'Content-Type: application/json' \
  -d '{"grace_seconds":0}' \
  "http://127.0.0.1:$port_a/api/v1/admin/api-keys/$api_key_id/rotate")
api_key_secret=$(printf '%s' "$api_key_rotation" | jq -r .secret)
test "$api_key_secret" != "$api_key_original_secret"
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
echo "E2E PASS: two non-root server pods shared auth, OIDC, signed updates, assets, diagnostics, and DB-outage event spool replay"
