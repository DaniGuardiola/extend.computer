#!/usr/bin/env python3
"""Reversible maximum-brightness guard for a local Mac and USB Android displays.

Save an exclusive recovery snapshot before changing anything. Wrap a complete
campaign, rather than each scene, so its brightness policy stays constant.
"""
import argparse
import contextlib
import ctypes
import json
from pathlib import Path
import signal
import subprocess
import time


def run(argv):
    return subprocess.check_output(argv, text=True, timeout=30).strip()


def android(serial, action, key, value=None):
    args = ["adb", "-s", serial, "shell", "settings", action, "system", key]
    if value is not None:
        args.append(str(value))
    return run(args)


def mac_display_id():
    lib = ctypes.CDLL("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
    displays = (ctypes.c_uint32 * 64)()
    count = ctypes.c_uint32()
    get = lib.CGGetOnlineDisplayList
    get.argtypes = [ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32), ctypes.POINTER(ctypes.c_uint32)]
    if get(64, displays, ctypes.byref(count)) != 0:
        raise RuntimeError("Mac display inventory unavailable")
    builtin = lib.CGDisplayIsBuiltin
    builtin.argtypes = [ctypes.c_uint32]
    ids = [display for display in displays[:count.value] if builtin(display)]
    if len(ids) != 1:
        raise RuntimeError("Exactly one built-in Mac benchmark display required")
    return ids[0]


def mac_brightness(value=None, display_id=None):
    if display_id is None:
        display_id = mac_display_id()
    lib = ctypes.CDLL("/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices")
    if value is None:
        number = ctypes.c_float()
        fn = lib.DisplayServicesGetBrightness
        fn.argtypes = [ctypes.c_uint32, ctypes.POINTER(ctypes.c_float)]
        result = fn(display_id, ctypes.byref(number))
        if result != 0:
            raise RuntimeError("Mac brightness cannot be read")
        return number.value
    fn = lib.DisplayServicesSetBrightness
    fn.argtypes = [ctypes.c_uint32, ctypes.c_float]
    if fn(display_id, value) != 0:
        raise RuntimeError("Mac brightness cannot be changed")


def mac_auto(value=None, display_id=None):
    if display_id is None:
        display_id = mac_display_id()
    lib = ctypes.CDLL("/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices")
    supported = lib.DisplayServicesHasAmbientLightCompensation
    supported.argtypes = [ctypes.c_uint32]
    supported.restype = ctypes.c_bool
    if not supported(display_id):
        raise RuntimeError("Mac automatic brightness control unavailable")
    get = lib.DisplayServicesAmbientLightCompensationEnabled
    get.argtypes = [ctypes.c_uint32, ctypes.POINTER(ctypes.c_bool)]
    get.restype = ctypes.c_int
    if value is not None:
        put = lib.DisplayServicesEnableAmbientLightCompensation
        put.argtypes = [ctypes.c_uint32, ctypes.c_bool]
        put.restype = ctypes.c_int
        if put(display_id, bool(value)) != 0:
            raise RuntimeError("Mac automatic brightness change failed")
    for attempt in range(20):
        current = ctypes.c_bool()
        if get(display_id, ctypes.byref(current)) != 0:
            raise RuntimeError("Mac automatic brightness cannot be read")
        if value is None or current.value == bool(value):
            return int(current.value)
        time.sleep(.1)
    raise RuntimeError("Mac automatic brightness change was not confirmed")


def snapshot(serials, include_mac):
    original = {"schema_version": 1, "policy": "fixed-maximum-v1", "android": [], "mac": None}
    for serial in serials:
        values = {key: android(serial, "get", key) for key in ("screen_brightness", "screen_brightness_mode")}
        if values["screen_brightness_mode"] not in ("0", "1") or not values["screen_brightness"].isdigit():
            raise RuntimeError("Android brightness settings unavailable")
        original["android"].append({"serial": serial, **values})
    if include_mac:
        display_id = mac_display_id()
        original["mac"] = {"display_id": display_id, "brightness": mac_brightness(display_id=display_id),
                           "auto": mac_auto(display_id=display_id)}
    return original


def apply(original):
    if original["mac"]:
        mac_auto(0, original["mac"]["display_id"])
        mac_brightness(1.0, original["mac"]["display_id"])
        if mac_brightness(display_id=original["mac"]["display_id"]) < .99:
            raise RuntimeError("Mac maximum brightness was not confirmed")
    for device in original["android"]:
        serial = device["serial"]
        android(serial, "put", "screen_brightness_mode", 0)
        android(serial, "put", "screen_brightness", 255)
        # Settings observers can finish applying the automatic-mode change
        # after the shell write returns. Wait for acknowledgement before capture.
        for attempt in range(30):
            mode = android(serial, "get", "screen_brightness_mode")
            level = android(serial, "get", "screen_brightness")
            if mode == "0" and level == "255":
                break
            # A pending automatic-brightness observer can overwrite the first
            # manual level. Reassert only within this bounded setup phase.
            if mode != "0":
                android(serial, "put", "screen_brightness_mode", 0)
            android(serial, "put", "screen_brightness", 255)
            time.sleep(.1)
        else:
            raise RuntimeError(f"Android maximum brightness was not confirmed: mode={mode}, level={level}")


def restore(original):
    errors = []
    for device in original["android"]:
        try:
            for key in ("screen_brightness", "screen_brightness_mode"):
                android(device["serial"], "put", key, device[key])
        except Exception as error:
            errors.append(str(error))
    if original["mac"]:
        try:
            mac_brightness(original["mac"]["brightness"], original["mac"]["display_id"])
            mac_auto(original["mac"]["auto"], original["mac"]["display_id"])
        except Exception as error:
            errors.append(str(error))
    if errors:
        raise RuntimeError("Brightness restoration incomplete: " + "; ".join(errors))


@contextlib.contextmanager
def maximum(serials, include_mac, recovery):
    original = snapshot(serials, include_mac)
    recovery = Path(recovery)
    recovery.parent.mkdir(parents=True, exist_ok=True)
    with recovery.open("x") as file:
        json.dump(original, file, indent=2)
    try:
        apply(original)
        yield original
    finally:
        restore(original)
        recovery.with_suffix(".restored.json").write_text(json.dumps({"restored": True}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--android", action="append", default=[])
    parser.add_argument("--mac", action="store_true")
    parser.add_argument("--snapshot", required=True, type=Path)
    parser.add_argument("--restore", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.restore:
        restore(json.loads(args.snapshot.read_text()))
        return
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("Supply a campaign command after --, or use --restore")
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    with maximum(args.android, args.mac, args.snapshot):
        result = subprocess.run(command, check=False)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
