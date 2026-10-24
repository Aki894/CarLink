#!/bin/sh
# systemd supervises restarts; this process supervises the two data endpoints.
set -eu
manager_config=${CARLINK_MANAGER_CONFIG:-/etc/carlink/manager.json}
if [ -s "$manager_config" ]; then
    exec /usr/local/bin/carlink manager run --config "$manager_config"
fi
exec python3 "${CARLINK_CARPLAY_ROOT:-/usr/local/lib/carlink/carplay}/supervisor.py"
