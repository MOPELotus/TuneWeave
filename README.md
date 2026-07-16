# TuneWeave

统一、可扩展的跨平台音乐 API，为不同音乐平台提供一致的访问接口。

TuneWeave 使用 Rust 构建，目标是在保持较小存储体积、较低运行占用和快速启动的同时，把不同音乐平台的检索、歌单、媒体解析和账户能力统一为稳定的 HTTP API。

## 设计方向

- 同一种业务能力使用相同端点和统一输入输出结构。
- 账户请求可通过 `platform` 明确选择登录平台。
- 内容来源与播放来源解耦：歌单来自一个平台时，音频可按策略从其他平台解析。
- 指定或默认平台播放失败后，可按可配置顺序回退到其他平台。
- 平台适配器按能力声明接入，不要求每个平台实现不存在的功能。

## 计划接入顺序

1. 网易云 Basic
2. QQ 音乐 Basic
3. 哔哩哔哩 Basic
4. 网易云全量、QQ 音乐全量、B 站音视频范围内全量
5. 酷狗、咪咕、酷我（酷我接入前重新验证上游接口可用性）

Basic 优先覆盖普通音乐 App 的搜索、展示、播放/VIP 权益、登录、个人音乐库、MV、云盘、播客和底层协议；网易云剩余工作中，云盘完整读写、下载和播放链路优先于播客目录及节目播放。最终全量要求不变。TuneWeave 还将提供可导入任意平台歌单、混合添加任意平台可播放内容的 Uni Playlist。完整顺序、范围和定期上游检查规则见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 本地运行

```console
cargo run -p tuneweave-server --bin tuneweave
```

- `TUNEWEAVE_BIND`：监听地址，默认 `127.0.0.1:7832`。
- `TUNEWEAVE_NETEASE_COOKIE`：可选的网易云登录 Cookie；不会写入响应或日志。

当前可直接调用 `/healthz`、`/v1/platforms`、`/v1/capabilities`、
`/v1/search`、`/v1/tracks/{ref}`、`/v1/albums/{ref}`、
`/v1/artists/{ref}`、`/v1/playlists/{ref}` 及其曲目、歌词、媒体和目录子端点。
认证已提供二维码、账号密码、短信验证码和退出端点；完整契约见
[docs/api-v1.md](docs/api-v1.md)。

## 许可证

TuneWeave 采用 [MIT OR Apache-2.0](LICENSE) 双许可，使用者可任选其一。上游研究来源和固定快照见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
