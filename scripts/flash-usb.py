"""USB-only installation; preserve NVS and measurements, reset the old boot-slot selector."""

import argparse
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, default=Path("D:/t/rlcd"))
    parser.add_argument("--port", default="COM3")
    args = parser.parse_args()
    release = args.target_dir / "xtensa-esp32s3-espidf/release"
    outputs = sorted(
        (release / "build").glob("esp-idf-sys-*/output"),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    if not outputs:
        raise SystemExit("Build the firmware first")
    build = outputs[0].parent / "out/build"
    subprocess.run(
        [
            "espflash",
            "flash",
            "--port",
            args.port,
            "--flash-size",
            "16mb",
            "--bootloader",
            str(build / "bootloader/bootloader.bin"),
            "--partition-table",
            str(ROOT / "config/partitions.csv"),
            "--erase-parts",
            "otadata",
            str(release / "firmware"),
        ],
        check=True,
    )


if __name__ == "__main__":
    main()
