#!/usr/bin/env python3
"""
VantaOS ISO Builder — replaces xorriso with pure-Python pycdlib.
Usage: python tools/build_iso.py
"""
import io, os, sys, json, subprocess, urllib.request
from pathlib import Path

ROOT    = Path(__file__).parent.parent
KERNEL  = ROOT / "zig-out" / "bin" / "vanta"
LIMINE  = ROOT / "limine-bin"
ISO_OUT = ROOT / "vanta.iso"

LIMINE_BRANCH = "v8.x-binary"
LIMINE_API    = f"https://api.github.com/repos/limine-bootloader/limine/contents/?ref={LIMINE_BRANCH}"
LIMINE_NEED   = ["limine-bios.sys","limine-bios-cd.bin","limine-uefi-cd.bin","BOOTX64.EFI","BOOTIA32.EFI","limine.exe"]
HDR           = {"User-Agent": "VantaOS-build/1.0"}


# ── Download Limine ─────────────────────────────────────────────

def download_limine():
    if LIMINE.exists() and all((LIMINE / f).exists() for f in LIMINE_NEED[:4]):
        print("[limine] Cached")
        return
    LIMINE.mkdir(exist_ok=True)
    print(f"[limine] Downloading from GitHub ({LIMINE_BRANCH})...")
    req = urllib.request.Request(LIMINE_API, headers=HDR)
    with urllib.request.urlopen(req, timeout=30) as r:
        files = {f["name"]: f["download_url"] for f in json.loads(r.read())}
    for name in LIMINE_NEED:
        dst = LIMINE / name
        if dst.exists():
            continue
        url = files.get(name)
        if not url:
            print(f"  skip {name} (not in release)")
            continue
        print(f"  {name}...")
        req2 = urllib.request.Request(url, headers=HDR)
        with urllib.request.urlopen(req2, timeout=60) as r:
            dst.write_bytes(r.read())
    print("[limine] Done")


# ── Build ISO ───────────────────────────────────────────────────

def build_iso():
    try:
        import pycdlib
    except ImportError:
        sys.exit("ERROR: pip install pycdlib")

    if not KERNEL.exists():
        sys.exit(f"ERROR: kernel not at {KERNEL} — run: python-zig build")

    print("[iso]   Building...")

    iso = pycdlib.PyCdlib()
    iso.new(interchange_level=4, joliet=3, vol_ident="VANTAOS")

    # ── Helper: add directory in both ISO + Joliet namespaces ──
    def mkdir(path):
        iso.add_directory(path, joliet_path=path)

    # ── Helper: add file in both namespaces ──
    def add(local: Path, path: str):
        data = local.read_bytes()
        iso.add_fp(
            fp=io.BytesIO(data),
            length=len(data),
            iso_path=path,
            joliet_path=path,
        )

    # ── Directory tree ──
    mkdir("/BOOT")
    mkdir("/BOOT/LIMINE")
    mkdir("/EFI")
    mkdir("/EFI/BOOT")

    # ── Static files (add all BEFORE El Torito setup) ──
    add(ROOT / "limine.conf",               "/BOOT/LIMINE/LIMINE.CFG")
    add(KERNEL,                             "/BOOT/VANTA")
    add(LIMINE / "limine-bios.sys",         "/BOOT/LIMINE/LIMINE-BIOS.SYS")
    add(LIMINE / "limine-bios-cd.bin",      "/BOOT/LIMINE/LIMINE-BIOS-CD.BIN")
    add(LIMINE / "limine-uefi-cd.bin",      "/BOOT/LIMINE/LIMINE-UEFI-CD.BIN")
    add(LIMINE / "BOOTX64.EFI",             "/EFI/BOOT/BOOTX64.EFI")
    add(LIMINE / "BOOTIA32.EFI",            "/EFI/BOOT/BOOTIA32.EFI")

    # ── El Torito boot records (MUST be after files are added) ──
    # BIOS boot: no-emulation, boot-info-table (Limine requirement)
    iso.add_eltorito(
        "/BOOT/LIMINE/LIMINE-BIOS-CD.BIN",
        bootable=True,
        boot_load_size=4,
        boot_info_table=True,
        media_name="noemul",
    )
    # UEFI boot: second El Torito entry
    iso.add_eltorito(
        "/BOOT/LIMINE/LIMINE-UEFI-CD.BIN",
        bootable=True,
        boot_load_size=0,
        boot_info_table=False,
        media_name="noemul",
        efi=True,
    )

    iso.write(str(ISO_OUT))
    iso.close()

    kb = ISO_OUT.stat().st_size // 1024
    print(f"[iso]   {ISO_OUT.name} ({kb} KB)")


# ── BIOS install (limine.exe) ────────────────────────────────────

def bios_install():
    exe = LIMINE / "limine.exe"
    if not exe.exists():
        print("[bios]  limine.exe missing — UEFI-only (fine for QEMU)")
        return
    print("[bios]  Installing BIOS boot stages...")
    r = subprocess.run([str(exe), "bios-install", str(ISO_OUT)],
                       capture_output=True, text=True, timeout=30)
    if r.returncode == 0:
        print("[bios]  OK")
    else:
        print(f"[bios]  Warning: {r.stderr.strip()}")


if __name__ == "__main__":
    print("=" * 42)
    print("  VantaOS ISO Builder")
    print("=" * 42)
    download_limine()
    build_iso()
    bios_install()
    print(f"  -> {ISO_OUT}")
    print("=" * 42)
