# ESP32-S3-RLCD-4.2 开发命令
# 用法:`just` 默认跑 flash-and-monitor;`just <task>` 指定任务
# 列表:`just --list`

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# ELF 输出位置(.cargo/config.toml 里的 target-dir 短路径规避)
elf := "D:/t/rlcd/xtensa-esp32s3-espidf/release/firmware"
port := "COM4"

# 默认:编译 + 烧 + 监视,一把流
default: flash-monitor

# 只编译
build:
    cargo build --release

# 编译 + 烧
flash: build
    espflash flash {{elf}} --port {{port}}

# 只监视(Ctrl+C 退出)
monitor:
    espflash monitor --port {{port}}

# 编译 + 烧 + 立即进监视器(最常用)
flash-monitor: build
    espflash flash {{elf}} --port {{port}} --monitor

# 不烧,直接监视 + 触发硬件复位
reset-monitor:
    espflash monitor --port {{port}}

# 查看 firmware 体积明细(ELF 段)
size: build
    espflash save-image --chip esp32s3 --merge {{elf}} D:/t/rlcd/firmware.bin
    echo "bin:" ; ls -l D:/t/rlcd/firmware.bin

# 清掉构建产物(target 在短路径,注意是 D:\t\rlcd)
clean:
    cargo clean
    rm -rf D:/t/rlcd/xtensa-esp32s3-espidf

# 拉新依赖时用(挂代理,走完恢复)
update-deps:
    $env:HTTP_PROXY = "http://127.0.0.1:7890"; $env:HTTPS_PROXY = "http://127.0.0.1:7890"; cargo update

# 一键检查工具链状态
doctor:
    @echo "== rustc =="
    rustc --version
    @echo "== cargo =="
    cargo --version
    @echo "== Python =="
    python --version
    @echo "== espflash =="
    espflash --version
    @echo "== esp toolchain =="
    rustup toolchain list
    @echo "== COM ports =="
    Get-PnpDevice -Class Ports -Status OK | Select-Object FriendlyName
