# 设备管理界面

React + TypeScript + Tailwind CSS 4 + shadcn/ui（Radix）管理页，统一中文导航和浅色主题。
屏幕、家居、设备状态、日志、设置共用组件。手机使用底部导航，桌面使用侧栏。
保留浅色与灰绿配色、现有布局；文案只写功能名、数据和必要操作提示，不加口号、重复副标题、装饰标签或页脚。

## 构建与开发

```powershell
./scripts/build-web.ps1
npm --prefix web test
./scripts/build-local.ps1
./scripts/flash-usb.ps1 --port COM3
```

Node 24 / npm，依赖锁定在 `web/package-lock.json`。开发运行 `npm --prefix web run dev`，
默认通过本机 Vite 代理读取 `192.168.1.20` 的真实设备接口。可设置 `DEVICE_URL` 指向其他板子。
开发代理只监听 `127.0.0.1`，会把写请求的 Origin 改成目标设备地址；生产固件仍校验同源请求。

`web/src` 为源代码；`npm run build` 先类型检查，再构建、gzip 压缩。
`web/dist/*.gz` 由前端构建生成，供 Rust 的 `include_bytes!` 使用；整个 dist 目录不提交。
CI 从锁文件重新构建这些资源。静态资源压缩总预算 240 KiB，超出会阻止前端构建。
使用本地系统字体和 Lucide SVG，无 CDN、外部字体、SSR 或板端 Node.js。

## 固件接口

`src/net/web_ui.rs` 统一提供 `/`、`/settings`、`/system.html`、`/logs.html`、`/home`。
`/app.js` 和 `/app.css` 与 HTML 一起以 gzip 返回，浏览器解压；板子按 4 KiB 分块发送 Flash 数据。
所有资源 `Cache-Control: no-cache`，避免升级后读取旧缓存。直接刷新旧路径仍能打开对应页面。

沿用现有 JSON/BMP API。屏幕按上一帧结束后轮询，离开页面取消请求；日志短轮询，最多保留 1200 行，
处理设备重启序列号归零和缓存覆盖。后台标签页暂停刷新。
设置只发送修改过的允许字段，不回写脱敏 token，也不覆盖其他数据源配置；未保存离开时提示。
危险操作使用可取消的 Radix 对话框。固件仅通过 USB 烧录，网页不包含远程更新入口。

配网热点的 `prov_form.html` / `prov_done.html` 共用视觉样式，但保持小型原生 HTML 表单，
在无网络和 captive portal 浏览器中也能提交 `/save`。
