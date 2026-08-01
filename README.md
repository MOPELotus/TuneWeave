# TuneWeave

TuneWeave 是使用 Rust 构建的跨平台音乐 HTTP API。它以统一的资源引用、请求参数和响应模型连接网易云音乐、QQ 音乐、B 站、酷狗、咪咕、酷我与汽水音乐，并支持多账户、Uni Playlist 和播放失败后的跨平台资源回退。

## 快速开始

下载适合当前系统的预编译包并运行：

```console
./tuneweave
```

Windows 使用：

```console
tuneweave.exe
```

也可以从源码启动：

```console
cargo run --release -p tuneweave-server --bin tuneweave
```

服务默认监听 `127.0.0.1:7832`。确认服务可用：

```console
curl http://127.0.0.1:7832/healthz
curl http://127.0.0.1:7832/v1/platforms
curl http://127.0.0.1:7832/v1/capabilities
```

## 开发者文档

- [安装与配置](docs/getting-started.md)
- [HTTP API v1](docs/api-v1.md)
- [登录与调用方托管凭证](docs/authentication.md)
- [Uni Playlist](docs/uni-playlist.md)
- [完整路由目录](docs/routes.json)

所有业务响应使用统一 JSON 包络。资源引用带有平台前缀，例如 `netease:123456`、`qq:0039MnYb0qxYhV` 和 `bilibili:bvid:BV1xx411c7mD`。调用方可通过 `platform` 选择内容或账户平台，通过 `account` 选择服务器托管账户，也可使用 `X-TuneWeave-Credential` 携带由调用方保存的登录凭证。

接口可用性应以运行实例返回的 `/v1/capabilities` 为准。媒体端点返回平台提供的 URL 和必要请求头；TuneWeave 不代理媒体内容。

## 许可证

TuneWeave 采用 [MIT OR Apache-2.0](LICENSE) 双许可证。第三方研究来源见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
