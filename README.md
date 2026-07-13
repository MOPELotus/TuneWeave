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

1. 网易云音乐
2. QQ 音乐
3. 哔哩哔哩
4. 酷狗音乐
5. 咪咕音乐
6. 酷我音乐（接入前重新验证上游接口可用性）

> 当前处于架构与首个平台适配阶段，公开 API 会随首批实现一并记录在 `docs/` 中。

## 本地运行

```console
cargo run -p tuneweave-server --bin tuneweave
```

- `TUNEWEAVE_BIND`：监听地址，默认 `127.0.0.1:7832`。
- `TUNEWEAVE_NETEASE_COOKIE`：可选的网易云登录 Cookie；不会写入响应或日志。

当前可直接调用 `/healthz`、`/v1/platforms`、`/v1/capabilities`、
`/v1/search`、`/v1/tracks/{ref}`、`/v1/tracks/{ref}/lyrics`、
`/v1/playlists/{ref}` 和 `/v1/playlists/{ref}/tracks`。完整契约见
[docs/api-v1.md](docs/api-v1.md)。

## 许可证

TuneWeave 采用 [MIT OR Apache-2.0](LICENSE) 双许可，使用者可任选其一。上游研究来源和固定快照见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
