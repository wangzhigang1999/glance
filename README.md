# ESP32-S3-RLCD-4.2 Rust 固件

Waveshare ESP32-S3-RLCD-4.2(N16R8)开发板的 Rust 四页信息终端，基于
**ESP-IDF v5.5.3** + **`esp-idf-svc` 0.52** `std` 路线。

## 小米体重秤 2（新增）

启动联网后默认打开 **System** 页，体重页排在最后；KEY / BOOT 按 System → GitHub → Weight 顺序切换主页面。接收指定小米体重秤
`70:87:9E:41:24:A0` 的 `0x181D` 广播：实时显示公斤数和 MEASURING / STABLE 状态，
15 秒没有新广播或离秤后显示 LAST STABLE READING，不把旧值冒充实时值。

稳定读数按称重会话去重，最近 16 条保存在 NVS 的 `scale/history`，断电后恢复；
没有校准时钟时仍保存体重，时间为 null。只保存稳定结果，不把每个广播写入 Flash。
本功能只识别秤，不识别人，物品称量同样可能形成记录。

BLE 在 Wi-Fi 配网前启动，断网时仍可接收和保存；当前屏幕启动流程仍先完成 Wi-Fi
连接/配网。蓝牙使用被动扫描，与 Wi-Fi 共存，不进行配对或连接。
`GET /api/weight` 返回接收状态、信号年龄、实时读数及历史；接口限可信局域网使用，
与原有设备 API 一样没有登录鉴权，不应映射到公网。配置私密凭据后可通过 WSS 上传 ECS，详见下文。

纯协议回归测试（无需板子）：

```powershell
rustc +stable --test src/scale/protocol.rs -o D:/t/rlcd/scale-protocol-tests.exe
D:/t/rlcd/scale-protocol-tests.exe
```

本机旧 Python 3.11 已被移除，使用 `./build-local.ps1` 构建。脚本仅在当前进程
指定可用的 Python 和 `D:/t/idf55`（ESP-IDF v5.5.3），也支持 `-PythonHome` /
`-IdfHome` 参数。显式 SDK 路径避免全局 Git HTTPS→SSH 改写触发 embuild 重复克隆。

物理屏页面：

1. **System**：时钟、电池、SRAM、PSRAM、Flash、栈和网络状态；
2. **GitHub**：贡献图、通知和最近活动；
3. **Weight**：实时体重与最近稳定称重记录。

SHTC3 温湿度仍会采集并显示在 `/system.html`，不再占用物理屏主页。

板卡硬件细节见项目根目录 [`../docs/`](../docs/)(Waveshare Wiki 离线镜像 + 引脚速查表)。

---

## 文件结构

```
firmware/
├── .cargo/
│   └── config.toml          # 交叉编译配置:xtensa target / ldproxy 链接器 / 镜像 / 路径规避
├── .vscode/                 # VS Code 任务配置(cargo-generate 自动生成,基本不用改)
├── src/
│   └── main.rs              # 三页信息终端入口
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

| 工具                                  | 版本                              | 安装                                |
| ------------------------------------- | --------------------------------- | ----------------------------------- |
| Rust stable                           | 1.92+                             | https://rustup.rs                   |
| **Xtensa Rust 工具链(`esp` channel)** | 1.93.0.0                          | `espup install --std -t esp32s3`    |
| Python                                | **3.11**(不要用 Windows Store 版) | `winget install Python.Python.3.11` |
| espup                                 | 0.17+                             | `cargo install espup`               |
| espflash                              | 4.4+                              | `cargo install espflash`            |
| ldproxy                               | 0.3+                              | `cargo install ldproxy`             |
| cargo-generate                        | 0.23+                             | `cargo install cargo-generate`      |
| just(任务运行器,可选但推荐)           | 1.49+                             | `winget install Casey.Just`         |

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

KEY / BOOT 在 System → GitHub → Weight 三个主页面间切换。

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

| 键         | 行为                                    |
| ---------- | --------------------------------------- |
| `Ctrl + R` | 软复位芯片(触发 re-boot,看完整启动 log) |
| `Ctrl + C` | 退出监视器                              |

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

体重过滤：低于 1 kg 的读数不显示、不保存；1 kg 可正常记录。启动时自动清理 NVS 中低于 1 kg 的历史记录，零点/离秤广播仍用于识别下一次称重。

## ECS 遥测（真实设备）

设备使用 `wss://iot.bupt.site/mqtt`、MQTT 3.1.1、QoS 1，CA 证书链和域名校验开启。
`rlcd-01` client ID 仅供板子使用；不能同时启动同 ID 的电脑测试客户端。
每分钟上传一次温湿度及电量、RSSI、内存状态，传感器无效值使用 null；稳定且至少 1 kg 的体重立即进入上传队列。
启动时补传板子历史中尚未上传的记录。每次启动生成随机 boot_id，seq 递增，重试保留完整原始内容。
status 使用 retained online 和离线遗嘱；telemetry 不 retain。重连退避约 1–60 秒并加抖动。

私密连接配置位于被 Git 忽略的 `data/ecs-iot/rlcd-01.json`，或由构建环境 `RLCD_IOT_CONFIG` 指定。
构建时写入 OUT_DIR 并嵌入固件；不要分享固件二进制，它包含设备凭据。未提供配置时云上传关闭。
`/api/telemetry` 可查看连接、积压和 broker 确认数，不返回密码。

体重待发送消息和已发送标识保存在 NVS，重启继续用相同消息编号重试；未出队的历史以秤的最近 16 条为限。
温湿度 RAM 队列最多 60 条（约一小时），重启会丢失尚未发送的环境数据，满队列时丢弃最近的积压项为新数据腾空间。
PUBACK 仅代表 broker 确认，不代表 SQLite 入库；服务端尚未提供应用层 ACK。

## SRAM / PSRAM 分配

TLS 加密连接的动态内存明确放到 PSRAM；普通 malloc 超过 1KB 优先 PSRAM。
400 条日志使用内联定长字符串，整个环形缓冲及导出快照按大块分配到 PSRAM，避免数百个小字符串长期占用内部 SRAM。
主任务栈从 32KB 调为 16KB（原板实测最低空余约 25KB）；SPI DMA 缓冲和任务栈继续保留在内部 SRAM。
栈剩余量按 ESP-IDF 的字节单位显示，移除原先错误的乘 4；`/api/system` 另有 `heap_largest` 便于观察连续可分配空间。

## 米家温湿度时钟

复用体重秤 BLE 被动扫描，额外仅接收已确认的时钟 A4:C1:38:67:37:31。
MiBeacon v4/v5 AES-CCM 解密和认证使用 ESP-IDF 自带 mbedTLS；启动时运行公开合成向量及篡改/截断检查，失败禁用时钟接收，不影响秤。
绑定密钥从忽略目录 data/bindkeys.json 读取，或由 RLCD_CLOCK_KEYS 指定；构建后嵌入固件，勿分享二进制或备份。
/api/weight 的 scale.clock 提供 configured、authenticated、rejected 与分字段接收时间，不返回密钥。
新温度/湿度/电量通过 mijia_temperature_c、mijia_humidity_pct、mijia_battery_pct 字段上报，沿用 rlcd-01 凭据与 TLS。
按认证 nonce 去重（最多 256 个、一小时），旧值不周期性重发冒充新测量；最多缓存 60 条环境待发消息，断电丢失尚未发送环境读数。
板载 SHTC3 继续保留为诊断来源；ECS 看板优先米家、不混合历史，超过 15 分钟无更新标记过期。
