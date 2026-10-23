"""Read embedded Windows icon/version/manifest resources without executing the EXE."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import struct
import xml.etree.ElementTree as ET


def verify(path: Path, version: str) -> dict:
    if os.name != "nt":
        raise RuntimeError("Windows is required for data-only resource inspection")
    expected = tuple(int(part) for part in version.split(".")) + (0,)
    if len(expected) != 4:
        raise ValueError("Expected a three-part package version")
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError("Not a Windows executable")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", data, pe + 4)[0] != 0x8664:
        raise ValueError("Expected an x64 PE executable")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32]
    kernel.LoadLibraryExW.restype = ctypes.c_void_p
    kernel.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = ctypes.c_void_p
    kernel.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.SizeofResource.restype = ctypes.c_uint32
    kernel.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    kernel.LoadResource.restype = ctypes.c_void_p
    kernel.LockResource.argtypes = [ctypes.c_void_p]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [ctypes.c_void_p]
    kernel.FreeLibrary.restype = ctypes.c_int
    # AS_DATAFILE | AS_IMAGE_RESOURCE maps bytes without loading executable code.
    module = kernel.LoadLibraryExW(str(path.resolve()), None, 0x22)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        def resource(name: int, kind: int) -> bytes:
            record = kernel.FindResourceW(module, ctypes.c_void_p(name), ctypes.c_void_p(kind))
            if not record:
                raise RuntimeError(f"Missing resource type={kind}, id={name}")
            size = kernel.SizeofResource(module, record)
            handle = kernel.LoadResource(module, record)
            pointer = kernel.LockResource(handle)
            if not pointer or not size:
                raise RuntimeError(f"Empty resource type={kind}, id={name}")
            return ctypes.string_at(pointer, size)

        group = resource(1, 14)  # RT_GROUP_ICON
        reserved, icon_type, count = struct.unpack_from("<HHH", group)
        if reserved != 0 or icon_type != 1 or len(group) != 6 + count * 14:
            raise ValueError("Invalid embedded icon group")
        sizes = []
        for index in range(count):
            width, height, _, _, _, _, byte_count, icon_id = struct.unpack_from("<BBBBHHIH", group, 6 + index * 14)
            width, height = width or 256, height or 256
            icon = resource(icon_id, 3)  # RT_ICON
            if len(icon) != byte_count:
                raise ValueError("Embedded icon size does not match its group entry")
            if icon.startswith(b"\x89PNG\r\n\x1a\n"):
                actual = struct.unpack_from(">II", icon, 16)
            else:
                dib_width, dib_height = struct.unpack_from("<ii", icon, 4)
                actual = (dib_width, abs(dib_height) // 2)
            if actual != (width, height):
                raise ValueError("Embedded icon dimensions do not match its entry")
            sizes.append(width)
        if sorted(sizes) != [16, 20, 24, 32, 40, 48, 64, 128, 256]:
            raise ValueError(f"Missing expected icon frames: {sizes}")

        info = resource(1, 16)  # RT_VERSION
        cursor = 6
        while info[cursor:cursor + 2] != b"\0\0":
            if cursor + 2 >= len(info):
                raise ValueError("Unterminated version-resource key")
            cursor += 2
        if info[6:cursor].decode("utf-16le") != "VS_VERSION_INFO":
            raise ValueError("Invalid version-resource key")
        value_at = (cursor + 2 + 3) & ~3
        fixed = struct.unpack_from("<13I", info, value_at)
        if fixed[0] != 0xFEEF04BD:
            raise ValueError("Missing VS_FIXEDFILEINFO")
        file_version = (fixed[2] >> 16, fixed[2] & 0xFFFF, fixed[3] >> 16, fixed[3] & 0xFFFF)
        product_version = (fixed[4] >> 16, fixed[4] & 0xFFFF, fixed[5] >> 16, fixed[5] & 0xFFFF)
        if file_version != expected or product_version != expected:
            raise ValueError(f"Version mismatch: {file_version}, {product_version}")

        manifest = ET.fromstring(resource(1, 24))  # RT_MANIFEST
        elements = list(manifest.iter())
        levels = [node.get("level") for node in elements if node.tag.endswith("requestedExecutionLevel")]
        dpi = [node.text for node in elements if node.tag.endswith("dpiAwareness")]
        identities = [node.get("version") for node in elements if node.tag.endswith("assemblyIdentity") and node.get("name") == "NetSpeed"]
        if levels != ["asInvoker"] or "PerMonitorV2" not in dpi or ".".join(map(str, expected)) not in identities:
            raise ValueError("Required versioned asInvoker/PerMonitorV2 manifest is missing")
        return {"path": str(path), "file_version": file_version, "product_version": product_version, "icon_sizes": sizes, "privilege": "asInvoker", "dpi": "PerMonitorV2", "execution": "none; data-only mapping"}
    finally:
        kernel.FreeLibrary(module)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.exe, args.version), indent=2))
