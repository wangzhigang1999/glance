"""Portable locked build; inject this checkout's absolute partition path."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--public", action="store_true")
    args = parser.parse_args()
    env = os.environ.copy()
    target = Path(env.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    generated = target / "rlcd-config" / "sdkconfig.defaults"
    generated.parent.mkdir(parents=True, exist_ok=True)
    contents = (ROOT / "config/sdkconfig.defaults").read_text(encoding="utf-8")
    partition = (ROOT / "config/partitions.csv").as_posix()
    contents += f'\nCONFIG_PARTITION_TABLE_CUSTOM=y\nCONFIG_PARTITION_TABLE_CUSTOM_FILENAME="{partition}"\n'
    if not generated.exists() or generated.read_text(encoding="utf-8") != contents:
        generated.write_text(contents, encoding="utf-8", newline="\n")
    env["CARGO_TARGET_DIR"] = str(target)
    env["ESP_IDF_SDKCONFIG_DEFAULTS"] = str(generated)
    if args.public:
        env["RLCD_PUBLIC_BUILD"] = "1"
    subprocess.run(["cargo", "build", "--release", "--locked"], cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
