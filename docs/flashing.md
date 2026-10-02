# CI 固件烧录

适用：ESP32-S3-RLCD-4.2，16MB Flash / 8MB Octal PSRAM。
本产物不含 MQTT 密码和米家绑定密钥；刷到个性化设备后，这两项功能会因缺少配置而停用。
Wi-Fi 配网、GitHub 设置和称重历史仍使用设备 NVS。

## 已有同分区固件：保留数据升级（推荐）

安装 espflash 4.4.0，在解压目录运行，端口按实际替换：

```sh
espflash flash --port COM3 --flash-size 16mb --bootloader bootloader.bin --partition-table partitions.csv --erase-parts otadata firmware.elf
```

此命令按段写入，不覆盖 NVS；仅清除 otadata 启动选择以启动 factory 应用。
不要使用 erase-flash，也不要擦除 nvs 或 lake。固件只通过 USB 更新；远程上传接口和页面已移除。
旧分区地址保留以兼容已安装设备，空闲更新槽不再使用；不搬动设备配置和历史数据。

## 空白设备：完整镜像

```sh
esptool --chip esp32s3 --port COM3 write-flash 0x0 factory.bin
```

**factory.bin 是从 0x0 起的合并镜像，会覆盖 NVS 区域，清除已有配网、设置和称重历史。**
仅在首次烧录或明确需要重新初始化时使用，不能当作保留数据升级。
本地个性化构建：`./scripts/build-local.ps1`，然后 `./scripts/flash-usb.ps1 --port COM3`。

先核对 `build-info.json` 的提交版本及 `SHA256SUMS`，构建通过不等于该 CI 镜像已上板验证。
