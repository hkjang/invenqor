# Readiness waits shared by the end-to-end scripts.
#
# Every one of these used to be an inline loop that simply fell through when it
# ran out of attempts, so a service that never came up surfaced much later as
# whatever failed next: a missing file inside a container, or curl refusing a
# connection to a port nobody was listening on. The cause was several steps and
# a hundred lines away from the message.
#
# Source this file; it defines wait_until and wait_for_postgres.

# wait_until <seconds> <what> <command...>
wait_until() {
    limit=$1
    what=$2
    shift 2
    attempt=0
    while [ "$attempt" -lt "$limit" ]; do
        if "$@" >/dev/null 2>&1; then
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 1
    done
    echo "E2E FAIL: $what did not become ready within ${limit}s" >&2
    echo "E2E last attempt output:" >&2
    "$@" >&2 2>&1 || true
    return 1
}

# wait_for_postgres <container> <user> <database>
#
# The waits these replaced allowed 30 to 45 seconds. On a GitHub runner
# PostgreSQL took longer than that to initialise - the failing job shows 31.7
# seconds between the image being pulled and the server giving up - so the loop
# ran out, fell through, and the server started against a database that was not
# accepting connections yet. Locally it passed every time. Ninety seconds, and a
# failure that says so.
#
# It connects over TCP rather than calling pg_isready because TCP is what the
# server actually needs; pg_isready over the unix socket answers a slightly
# different question. That difference did not cause this failure - a local
# attempt to catch pg_isready reporting ready before TCP accepted did not
# reproduce - but the stricter check costs nothing.
wait_for_postgres() {
    wait_until 90 "PostgreSQL in $1" \
        docker exec "$1" psql -h 127.0.0.1 -U "$2" -d "$3" -Atc 'SELECT 1'
}
