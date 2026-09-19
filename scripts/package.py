"""Package only a clean public build. Local personalized binaries stay private."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    if os.environ.get("RLCD_PUBLIC_BUILD") != "1":
        raise SystemExit("Packaging requires RLCD_PUBLIC_BUILD=1 and a fresh public build.")
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    release = target / "xtensa-esp32s3-espidf/release"
    for name in ("iot_config.json", "clock_keys.json"):
        configs = list((release / "build").glob(f"firmware-*/out/{name}"))
        if not configs or any(json.loads(p.read_text()) != {} for p in configs):
            raise SystemExit(f"Expected empty public build outputs: {name}")
    dest = ROOT / "dist"
    if dest.exists() and any(dest.iterdir()):
        raise SystemExit("dist must be empty; use a fresh workspace for packaging.")
    dest.mkdir(exist_ok=True)
    bootloaders = list((release / "build").glob("esp-idf-sys-*/out/build/bootloader/bootloader.bin"))
    if len(bootloaders) != 1:
        raise SystemExit("Expected one bootloader in clean build output.")
    shutil.copy2(release / "firmware", dest / "firmware.elf")
    shutil.copy2(bootloaders[0], dest / "bootloader.bin")
    shutil.copy2(ROOT / "config/partitions.csv", dest / "partitions.csv")
    common = ["espflash", "save-image", "--chip", "esp32s3", "--flash-size", "16mb",
              "--bootloader", str(dest / "bootloader.bin"), "--partition-table", str(dest / "partitions.csv")]
    subprocess.run(common + [str(dest / "firmware.elf"), str(dest / "app.bin")], check=True)
    subprocess.run(common + ["--merge", "--skip-padding", str(dest / "firmware.elf"), str(dest / "factory.bin")], check=True)
    shutil.copy2(ROOT / "docs/flashing.md", dest / "FLASHING.md")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    (dest / "build-info.json").write_text(json.dumps({"revision": revision, "chip": "esp32s3",
        "flash_mb": 16, "public_build": True, "esp_idf": "v5.5.3"}, indent=2) + "\n")
    files = sorted(p for p in dest.iterdir() if p.is_file())
    (dest / "SHA256SUMS").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in files))


if __name__ == "__main__":
    main()
