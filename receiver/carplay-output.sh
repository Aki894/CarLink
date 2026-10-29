#!/bin/sh
# OXP consumer entry point. Keep codec paths in the same verified runtime.
set -eu
root=${CARLINK_CARPLAY_ROOT:-/usr/local/lib/carlink/carplay}
exec "${CARLINK_BINARY:-/usr/local/bin/carlink}" carplay-output \
    --audio-binary "$root/bin/carlink-audio" \
    --opus-library "$root/lib/libcarlink_opus.so" "$@"
