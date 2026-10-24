#!/usr/bin/env python3
"""Verify native engine coverage and ELF exports without executing packaged code."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zipfile

MACHINES = {"arm64-v8a": 183, "x86_64": 62}
EXPORT = "Java_computer_extend_android_NativeEngine_initialize"
MAX_LIBRARY = 128 * 1024 * 1024


def configured():
    config = json.loads((Path(__file__).resolve().parents[1] / "app.json").read_text())
    for plugin in config["expo"]["plugins"]:
        if isinstance(plugin, list) and plugin[0] == "expo-build-properties":
            return architectures(",".join(plugin[1]["android"]["buildArchs"]))
    raise ValueError("Missing configured Android ABIs")


def architectures(value):
    abis = value.split(",")
    if not abis or len(set(abis)) != len(abis) or any(abi not in MACHINES for abi in abis):
        raise ValueError("Expected unique supported ABIs: arm64-v8a, x86_64")
    return abis


def verify_elf(data, abi):
    def unpack(fmt, offset):
        size = struct.calcsize(fmt)
        if offset < 0 or offset + size > len(data):
            raise ValueError("Truncated ELF structure")
        return struct.unpack_from(fmt, data, offset)

    if len(data) > MAX_LIBRARY or data[:7] != b"\x7fELF\x02\x01\x01":
        raise ValueError("Engine must be bounded ELF64 little-endian")
    header = unpack("<HHIQQQIHHHHHH", 16)
    kind, machine, version, _, phoff, shoff, _, ehsize, phsize, phcount, shsize, shcount, _ = header
    if kind != 3 or machine != MACHINES[abi] or version != 1 or ehsize != 64:
        raise ValueError(f"Wrong ELF shared-library architecture for {abi}: machine={machine}")
    if phsize != 56 or not phcount or shsize != 64 or not shcount:
        raise ValueError("Missing ELF program/section tables")
    loads = 0
    for index in range(phcount):
        kind, _, offset, virtual, _, _, _, align = unpack("<IIQQQQQQ", phoff + index * phsize)
        if kind == 1:
            loads += 1
            if align < 16384 or offset % 16384 != virtual % 16384:
                raise ValueError("Engine LOAD segment lacks 16 KB page compatibility")
    if not loads:
        raise ValueError("Engine has no LOAD segments")
    sections = [unpack("<IIQQQQIIQQ", shoff + index * shsize) for index in range(shcount)]
    for section in sections:
        if section[1] != 11:  # SHT_DYNSYM: a string anywhere in the file is insufficient.
            continue
        offset, size, link, entry = section[4], section[5], section[6], section[9]
        if entry != 24 or size % entry or link >= len(sections) or offset + size > len(data):
            raise ValueError("Invalid dynamic symbol table")
        strings = sections[link]
        if strings[1] != 3 or strings[4] + strings[5] > len(data):
            raise ValueError("Invalid dynamic string table")
        names = data[strings[4]:strings[4] + strings[5]]
        for cursor in range(offset, offset + size, entry):
            name, info, visibility, defined, _, _ = unpack("<IBBHQQ", cursor)
            if name >= len(names):
                raise ValueError("Invalid dynamic symbol name")
            end = names.find(b"\0", name)
            if end < 0:
                raise ValueError("Unterminated dynamic symbol name")
            if names[name:end] == EXPORT.encode() and info >> 4 in (1, 2) and defined != 0 and visibility & 3 == 0:
                return {"abi": abi, "elf_machine": machine, "jni_export": EXPORT,
                        "page_alignment": 16384, "sha256": hashlib.sha256(data).hexdigest()}
    raise ValueError("Engine JNI initialize export is missing or not public/defined")


def verify_apk(path, expected):
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError("Duplicate APK entries")
        actual = {name.split("/")[1] for name in names if name.startswith("lib/") and name.endswith(".so")}
        if actual != set(expected):
            raise ValueError(f"Packaged ABI mismatch: expected={expected}, actual={sorted(actual)}")
        results = []
        for abi in expected:
            name = f"lib/{abi}/libextend_android_engine.so"
            if name not in names:
                raise ValueError(f"Missing engine for packaged ABI {abi}")
            if archive.getinfo(name).file_size > MAX_LIBRARY:
                raise ValueError("Engine exceeds bounded verifier size")
            results.append(verify_elf(archive.read(name), abi))
        return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", nargs="?", type=Path)
    parser.add_argument("--expected")
    parser.add_argument("--jni-dir", type=Path)
    parser.add_argument("--configured", action="store_true")
    args = parser.parse_args()
    try:
        expected = architectures(args.expected) if args.expected else configured()
        if args.configured:
            print(",".join(expected))
            return
        if args.jni_dir:
            results = [verify_elf((args.jni_dir / abi / "libextend_android_engine.so").read_bytes(), abi) for abi in expected]
        elif args.apk:
            results = verify_apk(args.apk, expected)
        else:
            parser.error("Supply APK or --jni-dir")
        print(json.dumps({"status": "passed", "libraries": results}, indent=2))
    except (OSError, ValueError, zipfile.BadZipFile, KeyError) as error:
        parser.exit(1, f"Android ABI verification failed: {error}\n")


if __name__ == "__main__":
    main()
