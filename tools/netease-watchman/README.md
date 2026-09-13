# 网易云 Watchman SDK 适配服务

此可选服务为易盾 V2 请求提供官方 SDK 业务 Token。Rust 服务不执行 JavaScript；只有需要 V2 的功能依赖此适配服务，新版喜欢使用 V3。

适配服务独立运行，不嵌入 TuneWeave 可执行文件。下载预编译程序的用户也需要从本仓库取得完整的 `tools/netease-watchman` 目录（包括 `package-lock.json`）。需要 Node.js 22 或更新版本。在本目录运行：

```sh
npm ci
npx playwright install chromium
npm start
```

启动 TuneWeave 前设置 `TUNEWEAVE_NETEASE_WATCHMAN_URL=http://127.0.0.1:17863/token`。可通过 `TUNEWEAVE_WATCHMAN_PORT` 修改适配服务端口。仅监听环回地址，不接受平台 Cookie、Authorization、Origin 或请求体；无需也不得配置网易云账户凭据。

PowerShell 在启动 TuneWeave 的终端执行：

```powershell
$env:TUNEWEAVE_NETEASE_WATCHMAN_URL = 'http://127.0.0.1:17863/token'
```

Linux/macOS 在启动 TuneWeave 的终端执行：

```sh
export TUNEWEAVE_NETEASE_WATCHMAN_URL=http://127.0.0.1:17863/token
```

Linux 如缺少 Chromium 系统依赖，可使用 `npx playwright install --with-deps chromium`。适配服务和 TuneWeave 必须能通过同一环回网络访问；容器部署需要共享网络命名空间，不能将这里的环回地址当成另一容器的地址。安装浏览器仅在部署时执行，`npm test` 不下载浏览器或访问官方服务。

每次 `POST /token` 创建独立 Chromium 上下文，加载官方 `acstatic-dun.126.net/tool.min.js`，调用 Watchman 初始化、采集及业务 Token 接口，随后关闭上下文。最多同时处理两个请求，40 秒后关闭超时上下文；失败返回 502，繁忙返回 429。没有 Token 缓存、指纹伪装、SDK 源码副本或远端写操作。

成功契约为 `{ "code": 200, "registered": true, "token": "..." }`。Rust 拒绝旧 `result.conf`、未注册、空值和非法头字符。未配置适配服务时 V2 明确返回 `PlatformUnavailable`；不将配置串当作 Token。获取 Token 成功不代表平台一定接受某项账户写操作。

运行 `npm test` 验证 HTTP 边界。浏览器上下文隔离和安装方法以 [Playwright 文档](https://playwright.dev/docs/browser-contexts) 为依据。Token 与浏览器状态不落盘，不写日志。

从仓库根目录验证 Rust 到实际 SDK 的只读取 Token 链路（先启动适配服务并设置上面的环境变量）：

```sh
cargo test -p tuneweave-provider-netease live_v2_sdk_adapter_returns_fresh_business_token -- --ignored --exact client::tests::live_v2_sdk_adapter_returns_fresh_business_token
```

CI 在 Windows、Linux、macOS 的干净 checkout 中执行 `npm ci` 和 `npm test`，覆盖服务与隔离生命周期；真实 SDK 验证按上面的命令单独运行。取 Token 不发送评论，也不改变账户数据。
