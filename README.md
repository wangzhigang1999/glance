# Glance · ESP32-S3-RLCD-4.2

Waveshare ESP32-S3-RLCD-4.2（N16R8）的 Rust 信息终端，使用 ESP-IDF v5.5.3。
三页界面：System → GitHub → Weight；支持小米体重秤 BLE 接收、温湿度采集和 MQTT 上报。

## 目录

```text
.cargo/               跨平台 Rust/ESP-IDF 构建配置
.github/workflows/    主机测试与可烧录固件 CI
config/               板卡 sdkconfig 和 Flash 分区表
docs/                 功能说明、开发指南和烧录说明
scripts/              构建、测试、打包脚本
src/                  Rust 固件（hw / net / scale / ui / config）
tests/host/           无需开发板的回归测试
web/                  设备本地管理页与样式
Cargo.toml / Cargo.lock   依赖及锁定版本
build.rs              Cargo 构建入口与私密配置注入
```

`data/` 为本机私密配置，`target/`、`dist/` 为构建产物，均不提交。
Cargo 自动发现的 `build.rs`、`rust-toolchain.toml`、`rustfmt.toml` 保留在根目录。

## 本地开发

Windows（脚本提供本机默认路径，可用参数覆盖）：

```powershell
./scripts/test-host.ps1
./scripts/build-web.ps1   # 修改网页后重新生成本地 CSS，需要 Node/npm
./scripts/build-local.ps1
just flash               # 默认 COM3，可用 just --set port COM5 flash 覆盖
```

Linux：安装 `espup` 的 esp32s3 工具链（1.93.0.0）、`ldproxy`、Python 3.11、CMake/Ninja，
加载 espup 的环境导出文件，然后执行：

```sh
python scripts/build.py --public
```

统一入口会生成当前机器的分区绝对路径，并执行 `cargo build --release --locked`。
Windows 的短构建路径和 libclang 定位由脚本处理，不再写入共享 Cargo 配置。

## 下载 CI 固件

在 GitHub **Actions → Build flashable firmware → 成功运行 → Artifacts** 下载
`glance-esp32s3-<commit>`。push、PR 和手动运行均会构建，产物保留 30 天。

产物包含 `firmware.elf`、`app.bin`、`bootloader.bin`、`partitions.csv`、`factory.bin`、
版本信息、SHA-256 校验和及烧录指南。详见 [烧录说明](docs/flashing.md)。

**CI 是不含凭据的通用版本**：不嵌入 MQTT 密码或米家时钟绑定密钥，因此不具备你家设备的
云上报/加密时钟接收配置。需要这些功能时，继续在本地使用被忽略的 `data/` 配置编译。
普通 Wi-Fi/GitHub 配置和称重历史保存在 NVS；使用指南中的升级命令可保留它们。
不要将本地个性化 ELF/BIN 上传到公开仓库或 Actions。

更多功能、接口和运行限制见 [详细参考](docs/reference.md)。
