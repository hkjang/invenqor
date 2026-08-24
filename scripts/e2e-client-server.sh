#!/usr/bin/env bash
set -Eeuo pipefail

report_failure() {
  status=$?
  line=$1
  echo "E2E FAIL: command at line $line exited with status $status" >&2
  if [[ -n "${work:-}" && -f "$work/windows-event-response.json" ]]; then
    echo "E2E Windows ingest response (including request_id when provided):" >&2
    sed -n '1,80p' "$work/windows-event-response.json" >&2
  fi
  if [[ -n "${server:-}" ]] && docker inspect "$server" >/dev/null 2>&1; then
    echo "E2E Server log tail:" >&2
    docker logs --tail 160 "$server" >&2 || true
  fi
  if [[ -n "${packaged_client:-}" ]] && docker inspect "$packaged_client" >/dev/null 2>&1; then
    echo "E2E packaged Agent systemd status:" >&2
    docker exec "$packaged_client" systemctl --no-pager status invenqor-agent.service >&2 || true
    echo "E2E packaged Agent journal tail:" >&2
    docker exec "$packaged_client" journalctl --no-pager -u invenqor-agent.service -n 160 >&2 || true
  fi
  exit "$status"
}
trap 'report_failure "$LINENO"' ERR

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
suffix=$$
network="invenqor-e2e-$suffix"
postgres="invenqor-e2e-postgres-$suffix"
server="invenqor-e2e-server-$suffix"
client="invenqor-e2e-agent-$suffix"
update_client="invenqor-e2e-agent-update-$suffix"
bootstrap_client="invenqor-e2e-agent-bootstrap-$suffix"
packaged_client="invenqor-e2e-agent-packaged-$suffix"
packaged_hostname="invenqor-packaged-$suffix"
state_volume="invenqor-e2e-state-$suffix"
pg_volume="invenqor-e2e-pg-$suffix"
agent_volume="invenqor-e2e-agent-state-$suffix"
port=${INVENQOR_E2E_PORT:-18091}
work=$(mktemp -d)

cleanup() {
  docker rm -f "$client" "$update_client" "$bootstrap_client" "$packaged_client" \
    "$server" "$postgres" >/dev/null 2>&1 || true
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
openssl genpkey -algorithm ED25519 -out "$work/update-private.pem" >/dev/null 2>&1
update_public_key=$(
  openssl pkey -in "$work/update-private.pem" -pubout -outform DER |
    tail -c 32 | base64 | tr -d '\r\n'
)
docker run -d --name "$server" --network "$network" -p "127.0.0.1:$port:7070" \
  -e INVENQOR_POSTGRES_DSN='postgres://invenqor:e2e-contract-password@invenqor-e2e-postgres-'$suffix':5432/invenqor?sslmode=disable' \
  -e INVENQOR_UPDATE_PUBLIC_KEY="$update_public_key" \
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

# Exercise the Windows inventory contract against the real PostgreSQL-backed
# Server. The native executable is cross-built in CI; this event fixes the
# wire-level promise that Windows OS metadata and noisy process/service/package
# evidence become a small set of host-scoped managed software products.
windows_agent_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
windows_event_id=$(tr -d '\r\n' < /proc/sys/kernel/random/uuid)
windows_now=$(date +%s)
curl -fsS -H 'Content-Type: application/json' \
  -d '{"agent_id":"'"$windows_agent_id"'","hostname":"windows-e2e-host","claim_token":"ivq_ec_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}' \
  "http://127.0.0.1:$port/v1/agent/enroll" > "$work/windows-enrollment.json"
windows_token=$(jq -r .token "$work/windows-enrollment.json")
test "${windows_token#ivq_at_}" != "$windows_token"
jq -n \
  --arg agent "$windows_agent_id" --arg event "$windows_event_id" \
  --argjson now "$windows_now" \
  '{
    schema_version: 1,
    event_id: $event,
    agent_id: $agent,
    created_at: $now,
    kind: "inventory",
    snapshot_hash: "windows-managed-software-e2e",
    snapshot: {
      schema_version: 1,
      agent_id: $agent,
      collected_at: $now,
      duration_ms: 125,
      errors: [],
      records: [
        {asset_id:"windows-host", category:"system", source:"windows_system", collected_at:$now,
         payload:{hostname:"windows-e2e-host", os_family:"windows", os_name:"Windows 11 Enterprise", os_version:"24H2", os_build:"26100.4652", architecture:"x86_64"}},
        {asset_id:"mssql-service", category:"service", source:"windows_services", collected_at:$now,
         payload:{name:"MSSQLSERVER", display_name:"SQL Server (MSSQLSERVER)", state:"running", active:true, enabled:true, image_path:"C:\\Program Files\\Microsoft SQL Server\\MSSQL\\Binn\\sqlservr.exe -sMSSQLSERVER"}},
        {asset_id:"w3svc-service", category:"service", source:"windows_services", collected_at:$now,
         payload:{name:"W3SVC", display_name:"World Wide Web Publishing Service", state:"running", active:true, enabled:true, image_path:"C:\\Windows\\system32\\svchost.exe -k iissvcs"}},
        {asset_id:"sqlservr-process", category:"process", source:"windows_processes", collected_at:$now,
         payload:{name:"sqlservr.exe", executable:"C:\\Program Files\\Microsoft SQL Server\\MSSQL\\Binn\\sqlservr.exe", pid:4120}},
        {asset_id:"w3wp-process", category:"process", source:"windows_processes", collected_at:$now,
         payload:{name:"w3wp.exe", executable:"C:\\Windows\\System32\\inetsrv\\w3wp.exe", pid:4288}},
        {asset_id:"mssql-package", category:"software.package", source:"windows_registry", collected_at:$now,
         payload:{name:"Microsoft SQL Server 2022 (64-bit)", version:"16.0.4135.4", publisher:"Microsoft Corporation"}}
      ]
    },
    changes: [],
    collection_errors: []
  }' > "$work/windows-event.json"
windows_event_status=$(curl -sS -o "$work/windows-event-response.json" -w '%{http_code}' \
  -H "Authorization: Bearer $windows_token" \
  -H "X-Invenqor-Agent-Id: $windows_agent_id" \
  -H "X-Invenqor-Event-Id: $windows_event_id" \
  -H 'User-Agent: invenqor-agent/0.2.15' \
  -H 'Content-Type: application/json' \
  --data-binary "@$work/windows-event.json" \
  "http://127.0.0.1:$port/v1/agent/events")
if [[ ! "$windows_event_status" =~ ^2[0-9][0-9]$ ]]; then
  echo "Windows inventory ingest returned HTTP $windows_event_status" >&2
  false
fi
jq -e '.accepted == true' "$work/windows-event-response.json" >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/agents" |
  jq -e --arg id "$windows_agent_id" \
    '.agents[] | select(.agent_id == $id) | .os_name == "Windows 11 Enterprise"' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/assets/software-products?limit=200" \
  > "$work/windows-software.json"
jq -e '
  [.items[] | select(.product_key == "microsoft-sql-server")][0] as $sql |
  [.items[] | select(.product_key == "microsoft-iis")][0] as $iis |
  ($sql.host.name == "windows-e2e-host" and
   $sql.runtime_state == "running" and $sql.install_state == "installed" and
   ($sql.evidence | map(.kind) | index("process") != null) and
   ($sql.evidence | map(.kind) | index("service") != null) and
   ($sql.evidence | map(.kind) | index("package") != null) and
   $iis.host.name == "windows-e2e-host" and $iis.runtime_state == "running")
' "$work/windows-software.json" >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/assets?scope=managed&q=sqlservr.exe&limit=200" |
  jq -e '[.items[] | select(.type == "process")] | length == 0' >/dev/null

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

# Verify that an Agent, or an operator holding only curl, can establish whether
# registration would succeed before any state exists.
curl -fsS "http://127.0.0.1:$port/v1/agent/preflight" > "$work/preflight.json"
jq -e '.enrollment.would_enroll == true and
       .enrollment.reason == "AGENT_ENROLLMENT_READY" and
       .credential.state == "absent" and
       (.observed_source_ip | length > 0)' "$work/preflight.json" >/dev/null

# A base URL carrying a stray path must answer JSON, not the console SPA, and
# must leave a record an administrator can find.
stray_status=$(curl -sS -o "$work/stray.json" -w '%{http_code}' -X POST \
  -H 'Content-Type: application/json' -d '{}' \
  "http://127.0.0.1:$port/invenqor/v1/agent/enroll")
test "$stray_status" = 404
jq -e '.error.code == "AGENT_ENDPOINT_NOT_FOUND"' "$work/stray.json" >/dev/null

curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/diagnostics/enrollment?hours=24" \
  > "$work/enrollment-diagnostics.json"
jq -e '.totals.succeeded >= 1 and
       (.by_event_code[] | select(.event_code == "AGENT_ENDPOINT_NOT_FOUND")) and
       (.awaiting_inventory | length >= 1)' \
  "$work/enrollment-diagnostics.json" >/dev/null

docker volume create "$agent_volume" >/dev/null
mkdir -p "$work/config"

"$root/scripts/sign-agent-update-manifest-v2.py" \
  --artifact "$root/target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent" \
  --private-key "$work/update-private.pem" \
  --version 99.0.0 --channel stable --os linux --architecture x86_64 \
  > "$work/update-signature.json"

# A v2 signature whose bound artifact size and digest belong to another file
# must be refused when it is published, not discovered by every fleet Agent.
"$root/scripts/sign-agent-update-manifest-v2.py" \
  --artifact "$work/token" --private-key "$work/update-private.pem" \
  --version 99.0.0 --channel stable --os linux --architecture x86_64 \
  > "$work/wrong-signature.json"
rejected_publish=$(curl -sS -o "$work/rejected-publish.json" -w '%{http_code}' \
  -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -F "artifact=@$root/target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent" \
  -F "signature_bundle_file=@$work/wrong-signature.json;type=application/json" \
  -F rollout_percent=100 \
  "http://127.0.0.1:$port/api/v1/admin/agent-updates")
test "$rejected_publish" = 400
jq -e '.error.code == "UPDATE_SIGNATURE_REJECTED"' "$work/rejected-publish.json" >/dev/null

# One offline bundle carries both the legacy bridge signature and authenticated
# v2 manifest signature, and a canary rollout starts small.
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" \
  -F "artifact=@$root/target-x86_64/x86_64-unknown-linux-musl/release/invenqor-agent" \
  -F "signature_bundle_file=@$work/update-signature.json;type=application/json" \
  -F rollout_percent=10 \
  -F "notes=e2e canary" \
  "http://127.0.0.1:$port/api/v1/admin/agent-updates" \
  > "$work/update-manifest.json"
jq -e '.version == "99.0.0" and .size > 0 and .signature_verified == true and
       .signature_scheme == "ed25519" and .signature_version == 2 and
       .rollout_percent == 10' "$work/update-manifest.json" >/dev/null

# Rollout is widened without re-uploading, and the listing reports progress.
curl -fsS -b "$work/cookies" -H "X-CSRF-Token: $csrf" -X PATCH \
  -H 'Content-Type: application/json' -d '{"rollout_percent":100}' \
  "http://127.0.0.1:$port/api/v1/admin/agent-updates/99.0.0-linux-x86_64" |
  jq -e '.release.rollout_percent == 100' >/dev/null
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/agent-updates" |
  jq -e '.signature_verified == true and
         (.releases[] | select(.version == "99.0.0") |
          .signature_verified == true and .rollout_percent == 100)' >/dev/null

sed \
  -e 's|# url = .*|url = "http://'"$server"':7070"|' \
  -e 's|enabled = false|enabled = true|' \
  -e 's|heartbeat_seconds = 300|heartbeat_seconds = 1|' \
  -e 's|# public_key = .*|public_key = "'"$update_public_key"'"|' \
  -e 's|state_dir = .*|state_dir = "/var/lib/invenqor-agent"|' \
  "$root/config/config.toml" > "$work/config/config.toml"
chmod 0640 "$work/config/config.toml"
chmod -R a+rX "$work/config"

# Before the first cycle, diagnosis must distinguish a registration path that
# works from the expected fact that collection has never run. That is useful
# failure information, not an overall healthy result yet.
if docker run --rm --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --diagnose > "$work/diagnose-before.txt"; then
  echo "E2E FAIL: a never-run Agent diagnosis unexpectedly reported healthy" >&2
  exit 1
fi
grep -q '\[PASS\] registration policy' "$work/diagnose-before.txt"
grep -q '\[FAIL\] collection activity' "$work/diagnose-before.txt"
docker run --rm --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --once > "$work/snapshot.json"
jq -e '.records | length > 0' "$work/snapshot.json" >/dev/null
# After the real cycle has enrolled, collected and delivered, the same
# diagnosis must become fully healthy without any operator editing config.
docker run --rm --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --diagnose > "$work/diagnose-after.txt"
grep -q 'result: OK' "$work/diagnose-after.txt"
grep -q '\[PASS\] registration policy' "$work/diagnose-after.txt"
grep -q '\[PASS\] collection activity' "$work/diagnose-after.txt"
docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  sh -c 'test -s /state/device-credential.json && test -s /state/enrollment-claim.json'
# status.json is the record that survives when no Server is reachable at all.
docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 cat /state/status.json |
  jq -e '.enrollment.state == "enrolled" and
         .delivery.delivered_events >= 1 and
         .delivery.last_error == null' >/dev/null
docker run --name "$update_client" --network "$network" \
  -v "$work/config/config.toml:/etc/invenqor-agent/config.toml:ro" \
  -v "$agent_volume:/var/lib/invenqor-agent" \
  invenqor-agent:e2e --check-update > "$work/update-check.txt"
grep -q 'staged invenqor-agent update 99.0.0' "$work/update-check.txt"
docker run --rm -v "$agent_volume:/state:ro" alpine:3.22 \
  cat /state/updates/pending.json | jq -e '.manifest.version == "99.0.0"' >/dev/null
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
  jq -e '[.result.tools[].name] | index("asset_search") != null and index("software_inventory") != null and index("agents_list") != null' >/dev/null
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"asset_search","arguments":{"limit":10}}}' \
  "http://127.0.0.1:$port/mcp" |
  jq -e '.result.isError == false and (.result.structuredContent.items | length > 0)' >/dev/null
curl -fsS -H "Authorization: Bearer $api_key_secret" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"software_inventory","arguments":{"q":"SQL Server","runtime_state":"running","limit":10}}}' \
  "http://127.0.0.1:$port/mcp" |
  jq -e '.result.isError == false and
         (.result.structuredContent.items[] |
          select(.product_key == "microsoft-sql-server") |
          .host.name == "windows-e2e-host" and .evidence_count >= 3)' >/dev/null
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
curl -fsS -b "$work/cookies" \
  "http://127.0.0.1:$port/api/v1/admin/system/info" |
  jq -e '.agent_auto_enrollment == true and .agent_enrollment_mode == "open"
    and .port == 7070 and .listen_address == "0.0.0.0:7070"' >/dev/null
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
  jq -e '[.records | group_by(.asset_id)[] | select(length > 1)] | length == 0' \
    "$config_dir/snapshot.json" >/dev/null
  if [ "$label" = rhel9 ]; then
    # The minimal UBI image intentionally has no init/service manager. The
    # collector must degrade in isolation and explain that one unsupported
    # capability while every other collector and delivery continues.
    jq -e '.errors == [{"collector":"services","message":"unsupported on this host"}]' \
      "$config_dir/snapshot.json" >/dev/null
  else
    jq -e '.errors | length == 0' "$config_dir/snapshot.json" >/dev/null
  fi
  docker run --rm -v "$volume:/state:ro" alpine:3.22 \
    sh -c 'test -s /state/device-credential.json && test -s /state/enrollment-claim.json'
  docker volume rm "$volume" >/dev/null
  echo "E2E PASS: $label (unique asset IDs; expected collector status)"
}

run_enterprise_client "invenqor-agent:e2e-centos7" "centos7"
run_enterprise_client "invenqor-agent:e2e-rhel8" "rhel8"
run_enterprise_client "invenqor-agent:e2e-rhel9" "rhel9"
run_enterprise_client "invenqor-agent:e2e-ubuntu2204" "ubuntu2204"
run_enterprise_client "invenqor-agent:e2e-ubuntu2404" "ubuntu2404"

# Exercise the artifact operators actually receive, its installer and the
# systemd unit. Directly running the static executable above catches collector
# portability; this additionally catches archive layout, ownership, service
# hardening and URL-only enrollment regressions.
run_packaged_systemd_client() {
  package_output="$work/packages"
  package_archive="$package_output/invenqor-agent-linux-x86_64.tar.gz"
  package_root=/tmp/invenqor-agent-linux-x86_64
  status_file="$work/packaged-agent-status.json"
  diagnose_file="$work/packaged-agent-diagnose.txt"

  mkdir -p "$package_output"
  OUTPUT_DIR="$package_output" \
    "$root/packaging/build-tar.sh" x86_64-unknown-linux-musl \
    > "$work/package-build.txt"
  (cd "$package_output" && sha256sum -c "$(basename "$package_archive").sha256")

  # UBI 8 carries the RHEL 8 systemd/userspace stack. A private cgroup view and
  # writable /run are required so this tests a real service manager rather
  # than invoking the unit command by hand.
  docker run -d --name "$packaged_client" --hostname "$packaged_hostname" \
    --network "$network" --privileged --cgroupns=host \
    --tmpfs /run --tmpfs /run/lock \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
    --stop-signal SIGRTMIN+3 \
    registry.access.redhat.com/ubi8/ubi:8.10 /sbin/init >/dev/null

  systemd_ready=false
  for _ in $(seq 1 30); do
    if docker exec "$packaged_client" systemctl is-system-running --quiet; then
      systemd_ready=true
      break
    fi
    sleep 1
  done
  test "$systemd_ready" = true

  docker cp "$package_archive" "$packaged_client:/tmp/invenqor-agent-linux-x86_64.tar.gz"
  docker exec "$packaged_client" tar -xzf \
    /tmp/invenqor-agent-linux-x86_64.tar.gz -C /tmp
  # Deliberately change only server.url. Open enrollment supplies and persists
  # the device credential, while all shipped intervals and security defaults
  # remain untouched.
  docker exec "$packaged_client" sed -i \
    's|^# url = .*|url = "http://'"$server"':7070"|' \
    "$package_root/config/config.toml"
  docker exec "$packaged_client" grep -Fx \
    'url = "http://'"$server"':7070"' "$package_root/config/config.toml" >/dev/null
  docker exec "$packaged_client" "$package_root/scripts/install.sh" \
    > "$work/packaged-agent-install.txt"
  grep -q '^Invenqor agent installed\.$' "$work/packaged-agent-install.txt"
  grep -q '^Verify registration with:$' "$work/packaged-agent-install.txt"
  if grep -q '^NEXT STEP:' "$work/packaged-agent-install.txt"; then
    echo "E2E FAIL: packaged config lost server.url before installation" >&2
    false
  fi

  docker exec "$packaged_client" cat /etc/invenqor-agent/config.toml \
    > "$work/packaged-agent-config.toml"
  cmp -s \
    <(sed 's|^# url = .*|url = "http://'"$server"':7070"|' \
      "$root/config/config.toml") \
    "$work/packaged-agent-config.toml"

  docker exec "$packaged_client" systemctl is-enabled --quiet invenqor-agent.service
  docker exec "$packaged_client" systemctl is-active --quiet invenqor-agent.service
  docker exec "$packaged_client" systemctl is-enabled --quiet invenqor-agent-update.path
  docker exec "$packaged_client" test -x /opt/invenqor-agent/bin/invenqor-agent
  docker exec --user invenqor-agent "$packaged_client" \
    test -r /etc/invenqor-agent/config.toml
  test "$(docker exec "$packaged_client" stat -c '%a:%U:%G' \
    /etc/invenqor-agent/config.toml)" = "640:root:invenqor-agent"

  delivered=false
  for _ in $(seq 1 90); do
    if docker exec "$packaged_client" test -s /var/lib/invenqor-agent/status.json; then
      docker exec "$packaged_client" cat /var/lib/invenqor-agent/status.json > "$status_file"
      if jq -e '.enrollment.state == "enrolled" and
                .delivery.delivered_events >= 1 and
                .delivery.last_error == null and
                .collection.records > 0' "$status_file" >/dev/null; then
        delivered=true
        break
      fi
    fi
    docker exec "$packaged_client" systemctl is-active --quiet invenqor-agent.service
    sleep 1
  done
  test "$delivered" = true

  packaged_agent_id=$(jq -r .agent_id "$status_file")
  test -n "$packaged_agent_id"
  curl -fsS -b "$work/cookies" \
    "http://127.0.0.1:$port/api/v1/admin/agents" |
    jq -e --arg id "$packaged_agent_id" --arg hostname "$packaged_hostname" \
      '.agents[] | select(.agent_id == $id) |
       .hostname == $hostname and .status == "active"' >/dev/null

  docker exec --user invenqor-agent "$packaged_client" \
    /opt/invenqor-agent/bin/invenqor-agent \
    --config /etc/invenqor-agent/config.toml --diagnose > "$diagnose_file"
  grep -q 'result: OK' "$diagnose_file"
  grep -q '\[PASS\] registration policy' "$diagnose_file"
  grep -q '\[PASS\] collection activity' "$diagnose_file"

  # Diagnosis is intentionally run next to the daemon, then the daemon is
  # checked again to prove no state/lock interaction stopped the real service.
  docker exec "$packaged_client" systemctl is-active --quiet invenqor-agent.service
  sleep 2
  docker exec "$packaged_client" systemctl is-active --quiet invenqor-agent.service
  test "$(docker exec "$packaged_client" systemctl show \
    -p NRestarts --value invenqor-agent.service)" = 0

  echo "E2E PASS: packaged RHEL 8 Agent installed, enrolled and delivered under systemd"
}

run_packaged_systemd_client

echo "E2E PASS: real agent collected and delivered assets to PostgreSQL-backed server"
