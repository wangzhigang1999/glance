# Glance · ESP32-S3-RLCD-4.2

Waveshare ESP32-S3-RLCD-4.2（N16R8）的 Rust 信息终端，使用 ESP-IDF v5.5.3。
三页界面：System → GitHub → Weight；支持小米体重秤 BLE 接收、温湿度采集和 MQTT 上报。
KEY 短按翻页；BOOT 短按切换屏幕正向／倒置（180°），方向断电保存，网页镜屏保持正向。

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

Rust 工具链固定为 ESP Rust 1.98.1.0（本地名称 `esp198`），ESP-IDF 保持 v5.5.3：

```sh
espup install --name esp198 --toolchain-version 1.98.1.0 --targets esp32s3 --std
```

1.98.1.0 是上游预发布工具链。本固件在 1.97.0.0 上编译浮点 JSON 解析时触发
LLVM `PCREL_WRAPPER` 内部错误；相同最小复现已在 1.98.1.0 编译通过。
参见 [上游问题 #277](https://github.com/esp-rs/rust/issues/277)。

Windows（脚本提供本机默认路径，可用参数覆盖）：

```powershell
./scripts/test-host.ps1
./scripts/build-web.ps1   # 修改网页后重新生成本地 CSS，需要 Node/npm
./scripts/build-local.ps1
just flash               # 默认 COM3，可用 just --set port COM5 flash 覆盖
```

Linux：安装上述 esp32s3 工具链、`ldproxy`、Python 3.11、CMake/Ninja，
加载 espup 的环境导出文件，然后执行：

```sh
python scripts/build.py --public
```

统一入口会生成当前机器的分区绝对路径，并执行 `cargo build --release --locked`。
Windows 的短构建路径和 libclang 定位由脚本处理，不再写入共享 Cargo 配置。

## 下载 CI 固件

推送与 `Cargo.toml` 版本一致的标签（例如 `v0.2.1`），CI 会在固件构建和主机测试
都成功后自动创建 GitHub Release，附上 ZIP 烧录包、各镜像、校验和及烧录说明。
`v0.2.2-rc.1` 这类标签会发布为预发布版；普通分支推送仍只生成 Actions artifact。

```sh
git tag -a v0.2.1 -m "Glance v0.2.1"
git push origin v0.2.1
```

发布过程先创建草稿、上传全部附件，再公开。失败后可重跑同一次工作流；已公开版本
不会被覆盖，需要修改固件时请升版本并使用新标签。

在 GitHub **Actions → Build flashable firmware → 成功运行 → Artifacts** 下载
`glance-esp32s3-<commit>`。push、PR 和手动运行均会构建，产物保留 30 天。

产物包含 `firmware.elf`、`app.bin`、`bootloader.bin`、`partitions.csv`、`factory.bin`、
版本信息、SHA-256 校验和及烧录指南。详见 [烧录说明](docs/flashing.md)。

**CI 是不含凭据的通用版本**：不嵌入 MQTT 密码或米家时钟绑定密钥，因此不具备你家设备的
云上报/加密时钟接收配置。需要这些功能时，继续在本地使用被忽略的 `data/` 配置编译。
普通 Wi-Fi/GitHub 配置和称重历史保存在 NVS；使用指南中的升级命令可保留它们。
不要将本地个性化 ELF/BIN 上传到公开仓库或 Actions。

更多功能、接口和运行限制见 [详细参考](docs/reference.md)。
