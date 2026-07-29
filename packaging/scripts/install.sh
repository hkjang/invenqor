#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "install.sh must run as root" >&2
    exit 1
fi

PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BIN_DIR=/opt/invenqor-agent/bin
CONFIG_DIR=/etc/invenqor-agent
STATE_DIR=/var/lib/invenqor-agent

if ! getent group invenqor-agent >/dev/null 2>&1; then
    if command -v groupadd >/dev/null 2>&1; then
        groupadd --system invenqor-agent
    else
        addgroup -S invenqor-agent
    fi
fi
if ! getent passwd invenqor-agent >/dev/null 2>&1; then
    if command -v useradd >/dev/null 2>&1; then
        useradd --system --gid invenqor-agent --home-dir "$STATE_DIR" \
            --shell /sbin/nologin --comment "Invenqor inventory agent" invenqor-agent
    else
        adduser -S -D -H -G invenqor-agent -h "$STATE_DIR" -s /sbin/nologin invenqor-agent
    fi
fi

install -d -m 0755 "$BIN_DIR"
install -d -m 0750 "$CONFIG_DIR"
install -d -m 0700 -o invenqor-agent -g invenqor-agent "$STATE_DIR"
install -m 0755 "$PACKAGE_DIR/bin/invenqor-agent" "$BIN_DIR/invenqor-agent"
install -m 0644 "$PACKAGE_DIR/README.md" /opt/invenqor-agent/README.md

if [ ! -e "$CONFIG_DIR/config.toml" ]; then
    install -m 0640 -o root -g invenqor-agent \
        "$PACKAGE_DIR/config/config.toml" "$CONFIG_DIR/config.toml"
fi

if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    install -m 0644 "$PACKAGE_DIR/service/invenqor-agent.service" \
        /etc/systemd/system/invenqor-agent.service
    install -m 0644 "$PACKAGE_DIR/service/invenqor-agent-update.service" \
        /etc/systemd/system/invenqor-agent-update.service
    install -m 0644 "$PACKAGE_DIR/service/invenqor-agent-update.path" \
        /etc/systemd/system/invenqor-agent-update.path
    systemctl daemon-reload
    systemctl enable --now invenqor-agent.service invenqor-agent-update.path
elif command -v rc-service >/dev/null 2>&1; then
    install -m 0755 "$PACKAGE_DIR/service/invenqor-agent.openrc" \
        /etc/init.d/invenqor-agent
    rc-update add invenqor-agent default
    rc-service invenqor-agent start
elif [ -d /etc/init.d ]; then
    install -m 0755 "$PACKAGE_DIR/service/invenqor-agent.init" \
        /etc/init.d/invenqor-agent
    if command -v update-rc.d >/dev/null 2>&1; then
        update-rc.d invenqor-agent defaults
    elif command -v chkconfig >/dev/null 2>&1; then
        chkconfig --add invenqor-agent
    fi
    service invenqor-agent start
else
    echo "No supported init system found."
    echo "Run: $BIN_DIR/invenqor-agent --config $CONFIG_DIR/config.toml"
fi

echo "Invenqor agent installed."

# A fresh configuration has no server.url, so the service starts and quietly
# collects into its local queue. Say so here rather than letting the operator
# discover it as a missing Agent in the console.
if ! grep -Eq '^[[:space:]]*url[[:space:]]*=' "$CONFIG_DIR/config.toml"; then
    echo
    echo "NEXT STEP: server.url is not set in $CONFIG_DIR/config.toml."
    echo "  Until it is, this Agent collects inventory into its local queue,"
    echo "  never registers with a Server, and nothing appears in the console."
    echo "  1) Set server.url to the Server scheme, host and port."
    echo "  2) Restart the service."
    echo "  3) Verify with:"
    echo "     $BIN_DIR/invenqor-agent --config $CONFIG_DIR/config.toml --diagnose"
else
    echo "Verify registration with:"
    echo "  $BIN_DIR/invenqor-agent --config $CONFIG_DIR/config.toml --diagnose"
fi
