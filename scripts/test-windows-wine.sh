#!/usr/bin/env bash
# Runs the Windows agent under Wine and asserts a full collection cycle completes.
#
# Everything Windows-specific in this project used to be verified by building,
# linking, reading disassembly and computing struct offsets by hand, because no
# Windows machine was available. That is a poor substitute for running the code,
# and it let two defects ship: a service status handle declared 32-bit that the
# Service Control Manager truncated, and a bitwise complement used as a logical
# negation that hung the service enumeration forever.
#
# Wine is not Windows. It stubs NetUserEnum, so the accounts collector returns
# nothing here and stays unverified. What it does exercise is every FFI struct
# layout, calling convention and buffer protocol on the paths it implements -
# the registry, SMBIOS, the service control manager, the adapter list, the
# process snapshot - which is exactly where those two defects lived.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
image=${WINE_IMAGE:-scottyhardy/docker-wine:latest}
binary="$root/target-windows-x86_64/x86_64-pc-windows-gnu/release/invenqor-agent.exe"

if [ ! -f "$binary" ]; then
  echo "building the Windows binary first" >&2
  (cd "$root" && CARGO_TARGET_DIR=target-windows-x86_64 \
    cross build --locked --release --target x86_64-pc-windows-gnu)
fi

# A docker credential helper that fails must not stop the run: this only ever
# pulls public images.
config=$(mktemp -d)
echo '{}' > "$config/config.json"
export DOCKER_CONFIG="$config"
trap 'rm -rf "$config"' EXIT

docker run --rm --entrypoint /bin/bash \
  -v "$(dirname "$binary"):/w:ro" "$image" -lc '
set -euo pipefail
export WINEDEBUG=-all
cd /tmp
cp /w/invenqor-agent.exe .
wine invenqor-agent.exe --print-default-config > agent.toml 2>/dev/null
# Deliberately not under set -e. A cycle that dies part way through is the
# failure this exists to find, and letting the shell abort here reports it as
# silence - which is precisely how the defect presented on the real host.
status=0
wine invenqor-agent.exe --config agent.toml --once > cycle.json 2>/dev/null || status=$?
log=$(find /root/.wine -name agent.log | head -1)

fail() { echo "FAIL: $*" >&2; exit 1; }

if [ "$status" -ne 0 ]; then
  echo "--- the agent exited $status part way through a cycle ---" >&2
  echo "--- last collector to start was the one that did not survive ---" >&2
  grep -a "collector started" "$log" | tail -3 >&2
  grep -a "the agent panicked" "$log" >&2 || true
  fail "the collection cycle did not complete (exit $status)"
fi

# Every collector must be named as started and as finished. A collector that
# starts and never finishes is the shape of the crash this exists to catch: the
# process dies mid-cycle and the last "collector started" line names the culprit.
started=$(grep -ac "collector started" "$log" || true)
finished=$(grep -ac "collector finished" "$log" || true)
[ "$started" -eq 10 ] || fail "10 collectors should start, $started did"
[ "$finished" -eq "$started" ] || {
  echo "--- collectors that started but never finished ---" >&2
  comm -23 \
    <(grep -ao "collector started collector=\"[a-z]*\"" "$log" | grep -o "\"[a-z]*\"" | sort) \
    <(grep -ao "collector finished collector=\"[a-z]*\"" "$log" | grep -o "\"[a-z]*\"" | sort) >&2
  fail "$started collectors started but only $finished finished"
}

grep -aq "the agent panicked" "$log" && fail "a collector panicked - see $log"

python3 - <<PY
import json, sys
cycle = json.load(open("cycle.json"))
errors = cycle.get("errors", [])
if errors:
    print("FAIL: collectors reported errors:", json.dumps(errors, indent=2), file=sys.stderr)
    sys.exit(1)
records = cycle.get("records", [])
if len(records) < 10:
    print(f"FAIL: only {len(records)} records collected", file=sys.stderr)
    sys.exit(1)
print(f"{len(records)} records, no collector errors")
PY

echo "--- per-collector timing ---"
grep -a "collector finished" "$log" | sed "s/.*collectors: //"
'
