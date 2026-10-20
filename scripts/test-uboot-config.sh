#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
source "$root/firmware/userpatches/extensions/carlink.sh"
exit_with_error() { echo "$*" >&2; exit 1; }
display_alert() { :; }
export SRC="$fixture/builder"
cd "$fixture"
printf '%s\n' CONFIG_DRAM_CLK=408 CONFIG_DRAM_ZQ=3881979 \
    CONFIG_DRAM_ODT_EN=y CONFIG_SYS_CLK_FREQ=480000000 > .config
uboot_make_config__carlink_verify_boot_parameters
cmp .config "$SRC/output/info/carlink-uboot.config"
sed -i 's/CONFIG_DRAM_CLK=408/CONFIG_DRAM_CLK=624/' .config
if (uboot_make_config__carlink_verify_boot_parameters) 2>/dev/null; then
    echo 'Unsafe DRAM configuration unexpectedly passed' >&2
    exit 1
fi
echo 'U-Boot configuration gate: expected values pass; unsafe DRAM fails.'
