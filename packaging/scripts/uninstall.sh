#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "uninstall.sh must run as root" >&2
    exit 1
fi

if command -v systemctl >/dev/null 2>&1 && [ -e /etc/systemd/system/invenqor-agent.service ]; then
    systemctl disable --now invenqor-agent.service || true
    rm -f /etc/systemd/system/invenqor-agent.service
    systemctl daemon-reload
elif command -v rc-service >/dev/null 2>&1 && [ -e /etc/init.d/invenqor-agent ]; then
    rc-service invenqor-agent stop || true
    rc-update del invenqor-agent default || true
    rm -f /etc/init.d/invenqor-agent
elif [ -e /etc/init.d/invenqor-agent ]; then
    service invenqor-agent stop || true
    command -v update-rc.d >/dev/null 2>&1 && update-rc.d -f invenqor-agent remove || true
    command -v chkconfig >/dev/null 2>&1 && chkconfig --del invenqor-agent || true
    rm -f /etc/init.d/invenqor-agent
fi

rm -f /opt/invenqor-agent/bin/invenqor-agent
rmdir /opt/invenqor-agent/bin /opt/invenqor-agent 2>/dev/null || true

echo "Binary and service registration removed."
echo "Configuration and queued inventory remain under /etc/invenqor-agent and /var/lib/invenqor-agent."

