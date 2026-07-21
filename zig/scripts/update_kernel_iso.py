#!/usr/bin/env python3
"""
Update the kernel binary inside vanta.iso using pycdlib.
Usage: python3 scripts/update_kernel_iso.py
"""
import pycdlib
import sys
import os

ISO_PATH = "vanta.iso"
NEW_KERNEL = "zig-out/bin/vanta"
ISO_KERNEL_PATH = "/boot/vanta"

def main():
    if not os.path.exists(NEW_KERNEL):
        print(f"ERROR: Kernel not found: {NEW_KERNEL}")
        sys.exit(1)

    new_size = os.path.getsize(NEW_KERNEL)
    print(f"Opening {ISO_PATH}...")
    iso = pycdlib.PyCdlib()
    iso.open(ISO_PATH)

    # Check old kernel size
    try:
        rec = iso.get_record(iso_path=ISO_KERNEL_PATH)
        print(f"Old kernel size: {rec.data_length}")
    except Exception as e:
        print(f"WARNING: could not get old record: {e}")

    print(f"New kernel size: {new_size}")

    # Detect namespaces in use
    has_rr  = iso.has_rock_ridge()
    has_jol = iso.has_joliet()
    print(f"Rock Ridge: {has_rr}, Joliet: {has_jol}")

    # Remove old kernel
    print("Removing old kernel entry...")
    rm_kwargs = {"iso_path": ISO_KERNEL_PATH}
    if has_rr:
        rm_kwargs["rr_name"] = "vanta"
    if has_jol:
        rm_kwargs["joliet_path"] = "/boot/vanta"
    iso.rm_file(**rm_kwargs)

    # Add new kernel — keep file handle open through write() because
    # pycdlib holds a reference to the fp until the ISO is written.
    out_path = ISO_PATH + ".new"
    kernel_fp = open(NEW_KERNEL, "rb")
    try:
        print("Adding new kernel...")
        add_kwargs = {
            "fp": kernel_fp,
            "length": new_size,
            "iso_path": ISO_KERNEL_PATH,
        }
        if has_rr:
            add_kwargs["rr_name"] = "vanta"
        if has_jol:
            add_kwargs["joliet_path"] = "/boot/vanta"
        iso.add_fp(**add_kwargs)

        print(f"Writing {out_path}...")
        iso.write(out_path)
    finally:
        kernel_fp.close()
    iso.close()

    # Replace original
    os.replace(out_path, ISO_PATH)
    final_size = os.path.getsize(ISO_PATH)
    print(f"Done. {ISO_PATH} updated ({final_size} bytes)")

if __name__ == "__main__":
    main()
