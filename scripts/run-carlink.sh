#!/bin/sh
# systemd supervises restarts; this process supervises the two data endpoints.
set -eu
manager_config=${CARLINK_MANAGER_CONFIG:-/etc/carlink/manager.json}
root=${CARLINK_CARPLAY_ROOT:-/usr/local/lib/carlink/carplay}
example=${CARLINK_MANAGER_EXAMPLE:-/usr/share/carlink/manager.example.json}
if [ -z "${CARLINK_MANAGER_CONFIG:-}" ] && [ ! -e "$manager_config" ] && [ -f "$example" ]; then
    python3 "$root/supervisor.py" --bootstrap "$manager_config" "$example"
fi
if [ -e "$manager_config" ]; then
    exec /usr/local/bin/carlink manager run --config "$manager_config"
fi
if [ -n "${CARLINK_MANAGER_CONFIG:-}" ]; then
    echo 'Configured manager file is missing.' >&2
    exit 1
fi
exec python3 "$root/supervisor.py"
