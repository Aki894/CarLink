#!/bin/sh
# systemd supervises restarts; this process supervises the two data endpoints.
set -eu
exec python3 "${CARLINK_CARPLAY_ROOT:-/usr/local/lib/carlink/carplay}/supervisor.py"
