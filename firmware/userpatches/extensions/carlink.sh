function post_family_config__carlink_packages() {
    # Keep the UDC unbound for the forthcoming AOA/FunctionFS service.
    MODULES_CURRENT=""
    add_packages_to_image openssh-server usbutils usb-modeswitch iw rfkill bluez hostapd \
        dnsmasq-base avahi-utils iperf3 tcpdump curl ca-certificates python3
}

function uboot_make_config__carlink_verify_boot_parameters() {
    # Runs after Armbian patches, defconfig and olddefconfig, just before make.
    # The pinned framework already supplies the vendor's conservative values.
    local expected
    for expected in CONFIG_DRAM_CLK=408 CONFIG_DRAM_ZQ=3881979 \
        CONFIG_DRAM_ODT_EN=y CONFIG_SYS_CLK_FREQ=480000000; do
        if ! grep -qx "$expected" .config; then
            exit_with_error "Unexpected WuKongPi U-Boot configuration" "$expected"
            return 1
        fi
    done
    mkdir -p "${SRC}/output/info"
    cp .config "${SRC}/output/info/carlink-uboot.config"
    display_alert "Verified WuKongPi U-Boot" "DRAM 408 MHz; boot CPU 480 MHz" "info"
}
