# Windows shortcuts. Linux: python scripts/build.py --public
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]
elf := "D:/t/rlcd/xtensa-esp32s3-espidf/release/firmware"
port := "COM3"

default:
    @just --list

build:
    ./scripts/build-local.ps1

web:
    ./scripts/build-web.ps1

test:
    ./scripts/test-host.ps1

flash: build
    espflash flash --flash-size 16mb --partition-table config/partitions.csv {{elf}} --port {{port}}

monitor:
    espflash monitor --port {{port}}

flash-monitor: build
    espflash flash --flash-size 16mb --partition-table config/partitions.csv {{elf}} --port {{port}} --monitor
