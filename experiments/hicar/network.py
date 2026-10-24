"""Temporary HiCar AP test window; preserve wired management and restore CarPlay."""
from dataclasses import dataclass, field
import ipaddress
import json
from pathlib import Path
import re
import secrets
import subprocess


def command(*args, optional=False):
    result = subprocess.run(args, capture_output=True, text=True, timeout=40)
    if result.returncode and not optional:
        # nmcli arguments may contain the temporary PSK. Never include them here.
        raise RuntimeError('network command failed: ' + args[0])
    return result.stdout.strip()


@dataclass(frozen=True)
class Offer:
    ssid: str = field(repr=False)
    bssid: str = field(repr=False)
    password: str = field(repr=False)
    owner_ip: str = '192.168.49.1'
    client_ip: str = '192.168.49.2'
    frequency: int = 5745
    port: int = 4005

    def __post_init__(self):
        for text in (self.ssid, self.password):
            if not text.isascii() or any(c in text for c in ('\n', '\r', '#', '\x00')):
                raise ValueError('invalid network text field')
        if not 1 <= len(self.ssid) <= 32 or not 8 <= len(self.password) <= 63:
            raise ValueError('invalid SSID or password length')
        if not re.fullmatch(r'(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}', self.bssid):
            raise ValueError('invalid AP address')
        owner, client = ipaddress.IPv4Address(self.owner_ip), ipaddress.IPv4Address(self.client_ip)
        if (owner == client or owner not in ipaddress.IPv4Network('192.168.49.0/24')
                or client not in ipaddress.IPv4Network('192.168.49.0/24')):
            raise ValueError('invalid test network addresses')
        if self.frequency != 5745 or not 1024 <= self.port <= 65535:
            raise ValueError('unsupported test network endpoint')

    def credentials(self):
        # Reference software-AP path: client address, four Wi-Fi fields plus AP
        # marker, protocol version, control port, AP mode and actual gateway.
        config = '\n'.join((self.ssid, self.bssid, self.password, str(self.frequency), '1'))
        return (f'{self.client_ip}##{config}##version:3##{self.port}##2'
                f'##hicarApIp:{self.owner_ip}').encode('ascii')


class Hotspot:
    def __init__(self, interface, manage_service=True):
        if not re.fullmatch(r'[a-zA-Z0-9_.-]{1,15}', interface):
            raise ValueError('invalid wireless interface')
        self.interface = interface
        self.name = 'carlink-hicar-test-' + secrets.token_hex(4)
        self.previous = None
        self.active = False
        self.created = False
        self.manage_service = manage_service

    def __enter__(self):
        route = json.loads(command('ip', '-j', 'route', 'get', '192.168.77.1'))
        if not route or route[0].get('dev') == self.interface:
            raise RuntimeError('wired management route required')
        # Refuse an existing connection on the private experimental subnet.
        addresses = json.loads(command('ip', '-j', '-4', 'address'))
        for link in addresses:
            for addr in link.get('addr_info', []):
                if ipaddress.IPv4Address(addr['local']) in ipaddress.IPv4Network('192.168.49.0/24'):
                    raise RuntimeError('experimental subnet already in use')
        connections = command('nmcli', '-t', '-f', 'UUID,DEVICE', 'connection', 'show', '--active')
        self.previous = next((line.split(':')[0] for line in connections.splitlines()
                              if line.endswith(':' + self.interface)), None)
        self.active = (self.manage_service and
                       command('systemctl', 'is-active', 'carlink', optional=True) == 'active')
        try:
            if self.active:
                command('systemctl', 'stop', 'carlink')
            command('nmcli', 'device', 'disconnect', self.interface, optional=True)
            command('nmcli', 'connection', 'add', 'type', 'wifi', 'ifname', self.interface,
                    'con-name', self.name, 'autoconnect', 'no', 'ssid', 'CarLink')
            self.created = True
            password = secrets.token_hex(16)
            command('nmcli', 'connection', 'modify', self.name,
                    '802-11-wireless.mode', 'ap', '802-11-wireless.band', 'a',
                    '802-11-wireless.channel', '149', '802-11-wireless-security.key-mgmt', 'wpa-psk',
                    '802-11-wireless-security.proto', 'rsn', '802-11-wireless-security.psk', password,
                    'ipv4.method', 'shared', 'ipv4.addresses', '192.168.49.1/24', 'ipv6.method', 'disabled')
            command('iw', 'reg', 'set', 'CN')
            command('nmcli', 'connection', 'up', self.name, 'ifname', self.interface)
            info = command('iw', 'dev', self.interface, 'info')
            if 'type AP' not in info or '(5745 MHz)' not in info:
                raise RuntimeError('5 GHz AP not ready')
            bssid = Path('/sys/class/net', self.interface, 'address').read_text().strip()
            self.offer = Offer('CarLink', bssid, password)
            return self.offer
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        try:
            if self.created:
                command('nmcli', 'connection', 'delete', self.name, optional=True)
            if self.previous and self.manage_service:
                command('nmcli', 'connection', 'up', 'uuid', self.previous,
                        'ifname', self.interface, optional=True)
        finally:
            if self.active:
                command('systemctl', 'start', 'carlink')

