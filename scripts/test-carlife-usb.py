"""Host-side AOA1 and CarLife handshake test. Requires pyusb + libusb-package.

Run the temporary `carlink carlife usb` process on the board first. This tool
selects only the unique CarLink lab serial and never changes installed drivers.
"""
import json
import pathlib
import runpy
import socket
import time

import libusb_package
import usb.core
import usb.util

SERIAL = "carlink-h3-lab"


def select(product):
    matches = []
    for device in libusb_package.find(find_all=True, idVendor=0x18d1, idProduct=product):
        if device.serial_number == SERIAL:
            matches.append(device)
    if len(matches) > 1:
        raise RuntimeError("More than one CarLink lab device; disconnect duplicates")
    return matches[0] if matches else None


def protocol(device):
    data = bytes(device.ctrl_transfer(0xc0, 51, 0, 0, 2, timeout=2000))
    assert data == b"\x01\x00", "AOA1 version mismatch: " + data.hex()


def accessory():
    # A freshly bound gadget may take a moment to appear in Windows.
    deadline = time.monotonic() + 20
    device = None
    last_error = "device not found"
    while time.monotonic() < deadline:
        try:
            device = select(0x4ee7)
            if device is not None or select(0x2d00) is not None:
                break
        except (usb.core.USBError, NotImplementedError) as error:
            last_error = str(error)
        time.sleep(.25)
    else:
        raise RuntimeError("Initial driver/enumeration deadline: " + last_error)
    if device is not None:
        protocol(device)
        for index, text in enumerate(("CarLink HU Lab", "CarLife HU", "CarLink USB test", "1.0", "", "CarLink-HU-Lab")):
            data = text.encode() + b"\0"
            count = device.ctrl_transfer(0x40, 52, 0, index, data, timeout=2000)
            assert count == len(data), "short AOA identity transfer"
        device.ctrl_transfer(0x40, 53, 0, 0, b"", timeout=2000)
        usb.util.dispose_resources(device)
    deadline = time.monotonic() + 20
    last_error = "device not found"
    while time.monotonic() < deadline:
        try:
            device = select(0x2d00)
            if device is not None:
                protocol(device)  # VERSION query must also work after transition.
                device.ctrl_transfer(0x40, 53, 0, 0, b"", timeout=2000)  # idempotent START
                print(json.dumps({"event": "aoa_passed", "identity": "18d1:2d00"}))
                return device
        except (usb.core.USBError, NotImplementedError) as error:
            # Windows can publish the USB node before WinUSB is ready to open it.
            last_error = str(error)
        time.sleep(.25)
    raise RuntimeError("Accessory driver/enumeration deadline: " + last_error)


class BulkStream:
    def __init__(self, device):
        self.device, self.pending, self.timeout = device, bytearray(), 3
        self.interface = device.get_active_configuration()[(0, 0)]
        endpoints = list(self.interface)
        self.input = next(ep for ep in endpoints if usb.util.endpoint_direction(ep.bEndpointAddress) == usb.util.ENDPOINT_IN)
        self.output = next(ep for ep in endpoints if usb.util.endpoint_direction(ep.bEndpointAddress) == usb.util.ENDPOINT_OUT)
        usb.util.claim_interface(device, self.interface.bInterfaceNumber)

    def settimeout(self, seconds):
        self.timeout = seconds

    def sendall(self, data):
        count = self.device.write(self.output.bEndpointAddress, data, timeout=max(1, int(self.timeout * 1000)))
        if count != len(data):
            raise RuntimeError("short USB write; session will not retry a partial message")

    def recv(self, count):
        if not self.pending:
            try:
                data = self.device.read(self.input.bEndpointAddress, 512, timeout=max(1, int(self.timeout * 1000)))
            except usb.core.USBTimeoutError as error:
                raise socket.timeout(str(error)) from error
            self.pending.extend(data)
        data = bytes(self.pending[:count])
        del self.pending[:count]
        return data

    def close(self):
        usb.util.release_interface(self.device, self.interface.bInterfaceNumber)
        usb.util.dispose_resources(self.device)


if __name__ == "__main__":
    fixture = runpy.run_path(str(pathlib.Path(__file__).with_name("test-carlife-lab.py")))
    stream = BulkStream(accessory())
    try:
        fixture["handshake"](stream)
        print(json.dumps({"event": "usb_carlife_handshake", "status": "passed"}))
    finally:
        stream.close()
