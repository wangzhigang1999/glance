# ESP32-S3-RLCD-4.2 Rust 固件

Waveshare ESP32-S3-RLCD-4.2(N16R8)开发板的 Rust 固件骨架,基于 **ESP-IDF v5.5.3** +
**`esp-idf-svc` 0.52** `std` 路线。

板卡硬件细节见项目根目录 [`../docs/`](../docs/)(Waveshare Wiki 离线镜像 + 引脚速查表)。

---

## 文件结构

```
firmware/
├── .cargo/
│   └── config.toml          # 交叉编译配置:xtensa target / ldproxy 链接器 / 镜像 / 路径规避
├── .vscode/                 # VS Code 任务配置(cargo-generate 自动生成,基本不用改)
├── src/
│   └── main.rs              # 入口,目前是电池 ADC 读取 demo
├── build.rs                 # embuild 构建脚本,触发 ESP-IDF 下载 + C 编译 + bindgen
├── Cargo.toml               # Rust 依赖清单(log / esp-idf-svc / anyhow)
├── justfile                 # 开发命令快捷方式(just flash / just monitor / 等)
├── rust-toolchain.toml      # 固定使用 "esp" 工具链(espup 安装的 xtensa fork)
├── sdkconfig.defaults       # ESP-IDF 编译期配置:PSRAM / 16MB Flash / CPU 240MHz / USB 日志
├── .gitignore
└── README.md                # 本文件
```

运行期产生(**不提交**,已在 `.gitignore`):

- `.embuild/` — 克隆下来的 ESP-IDF + 工具链(~6GB)
- `D:\t\rlcd\` — cargo 构建产物(**target-dir 重定向**,短路径规避 Windows 路径长度限制)
- `Cargo.lock`

---

## 关键配置文件讲解

### `.cargo/config.toml`

```toml
[build]
target = "xtensa-esp32s3-espidf"
target-dir = "D:/t/rlcd"           # 避免 Windows MAX_PATH 限制

[target.'cfg(target_os = "espidf")']
linker = "ldproxy"                 # 包装 xtensa-gcc,桥接 rustc ↔ ESP-IDF
runner = "espflash flash --monitor"  # `cargo run` 自动烧录并打开监视器

[unstable]
build-std = ["std", "panic_abort"]  # 为 xtensa 目标重新编译标准库

[env]
MCU = "esp32s3"
ESP_IDF_VERSION = "v5.5.3"
ESP_IDF_TOOLS_INSTALL_DIR = "workspace"     # ESP-IDF 安装到项目本地,不污染全局
IDF_GITHUB_ASSETS = "dl.espressif.cn/github_assets"  # 国内镜像,规避 GitHub 拉包慢
CARGO_WORKSPACE_DIR = { value = "", relative = true } # 搭配 target-dir 必需
LIBCLANG_PATH = "...\\libclang.dll"          # bindgen 生成 FFI 所需
```

### `sdkconfig.defaults`

板子 N16R8 型号配套配置:

- **8 MB Octal PSRAM @ 80 MHz** —— 不开 PSRAM Rust std 栈容易爆
- **16 MB Flash DIO 模式**
- **CPU 240 MHz**
- **USB Serial/JTAG 作为日志输出** —— 直接 Type-C 看 log,不需要外挂 UART

### `Cargo.toml`

依赖刻意保持最小:

```toml
log = "0.4"
esp-idf-svc = "0.52.1"
anyhow = "1.0"
```

**⚠ Windows 专属痛点**:加太多 deps(比如全套 `embassy-*`)会让链接行超过 **32KB 命令行上限**,报 `os error 206`。真需要用 async time driver 请考虑 WSL2 或 no_std 路线。

---

## 前置依赖

只有第一次装,装完长期复用。

| 工具 | 版本 | 安装 |
| --- | --- | --- |
| Rust stable | 1.92+ | https://rustup.rs |
| **Xtensa Rust 工具链(`esp` channel)** | 1.93.0.0 | `espup install --std -t esp32s3` |
| Python | **3.11**(不要用 Windows Store 版) | `winget install Python.Python.3.11` |
| espup | 0.17+ | `cargo install espup` |
| espflash | 4.4+ | `cargo install espflash` |
| ldproxy | 0.3+ | `cargo install ldproxy` |
| cargo-generate | 0.23+ | `cargo install cargo-generate` |
| just(任务运行器,可选但推荐) | 1.49+ | `winget install Casey.Just` |

## Shell 环境(一次性)

用户级 PowerShell profile 已配好,文件在
`%USERPROFILE%\Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1`。

新开 PowerShell 窗口自动带:

- `PATH` 前置 Python 3.11、esp-clang(xtensa 工具链 libclang)
- 函数 `prox-on` / `prox-off` —— 一键挂/摘 Clash 代理(127.0.0.1:7890)
- 函数 `esp-flash` / `esp-monitor` —— 独立于 just 的备用入口

---

## 日常开发

### 一把流(推荐)

```powershell
cd D:\codes\esp32-s3-rlcd\firmware
just
```

`just` 默认跑 `flash-monitor`:**编译 → 烧录 → 监视**。按 `Ctrl+C` 退出监视器。

### 分步命令

```powershell
just build            # 只编译,输出到 D:\t\rlcd\...\firmware
just flash            # 编译 + 烧(不开监视器)
just monitor          # 只开 COM3 监视器
just flash-monitor    # 编译 + 烧 + 监视(等同 just)
just size             # 打印 firmware bin 大小
just doctor           # 工具链自检
just clean            # 清构建产物
just update-deps      # 挂代理拉新依赖
just --list           # 看所有任务
```

### 不用 just 的等价命令

```powershell
cargo build --release
espflash flash D:/t/rlcd/xtensa-esp32s3-espidf/release/firmware --port COM3
espflash monitor --port COM3
```

### 监视器快捷键

打开 `espflash monitor` 后:

| 键 | 行为 |
| --- | --- |
| `Ctrl + R` | 软复位芯片(触发 re-boot,看完整启动 log) |
| `Ctrl + C` | 退出监视器 |

---

## 常见问题

### 烧录报 `Failed to open serial port COM3`

监视器窗口还开着,占用了 COM3。关掉它(`Ctrl+C` 或直接关窗),再 `just flash`。

### 第一次 `cargo build` 极慢

正常。第一次要:
1. 克隆 ESP-IDF + 所有 submodule(~1.5GB)
2. 下载 xtensa-gcc / cmake / ninja(~500MB)
3. bindgen 生成 5000+ 条 FFI
4. 编译 ESP-IDF 的 C 代码(几百个 `.c`)

**务必开代理** `prox-on`,全程 30-60 分钟。后续增量编译只需几秒。

### `error: linking with 'ldproxy' failed: (os error 206)`

Windows 命令行 32KB 上限。把 `Cargo.toml` 里的 embassy 全家桶或其他大依赖砍掉,
或者迁移到 WSL2。

### `Too long output directory ... Shorten your project path to no more than 10 characters`

`target-dir` 没重定向到短路径。检查 `.cargo/config.toml` 里 `target-dir = "D:/t/rlcd"` 是否在。

### `Failed to locate python`

Windows Store 里那个 python.exe 是 alias 存根,不能真执行。确保 PowerShell profile
把 `%LOCALAPPDATA%\Programs\Python\Python311` 放在 PATH 前面。

---

## 硬件引脚速查

板子引脚映射见 [`../docs/10-pinout.md`](../docs/10-pinout.md)(I2C / SPI / I2S / 按键 / ADC 全套)。

---

## 许可

板上示例代码遵循项目许可。第三方 crate(`esp-idf-svc` 等)遵循其各自许可。
