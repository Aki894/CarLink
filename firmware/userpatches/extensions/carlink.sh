function post_family_config__carlink_packages() {
    # Keep the UDC unbound for the forthcoming AOA/FunctionFS service.
    MODULES_CURRENT=""
    add_packages_to_image openssh-server usbutils iw rfkill bluez hostapd \
        dnsmasq-base avahi-utils iperf3 tcpdump curl ca-certificates python3
}
