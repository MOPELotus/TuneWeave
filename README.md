# TuneWeave

统一、可扩展的跨平台音乐 API，为不同音乐平台提供一致的访问接口。

TuneWeave 使用 Rust 构建，目标是在保持较小存储体积、较低运行占用和快速启动的同时，把不同音乐平台的检索、歌单、媒体解析和账户能力统一为稳定的 HTTP API。

## 设计方向

- 同一种业务能力使用相同端点和统一输入输出结构。
- 账户请求通过 `platform` 选择登录平台、通过 `account` 选择该平台内的持久化账户别名；每个平台可同时保存多个账户。
- 内容来源与播放来源解耦：歌单来自一个平台时，音频可按策略从其他平台解析。
- 指定或默认平台播放失败后，可按可配置顺序回退到其他平台。
- 平台适配器按能力声明接入，不要求每个平台实现不存在的功能。

## 计划接入顺序

1. 网易云 Basic
2. Uni Playlist
3. QQ 音乐 Basic
4. 哔哩哔哩 Basic
5. 网易云全量、QQ 音乐全量、B 站音视频范围内全量
6. 酷狗、咪咕、酷我（酷我接入前重新验证上游接口可用性）

Basic 优先覆盖普通音乐 App 的搜索、展示、播放/VIP 权益、登录、个人音乐库、MV、云盘、播客和底层协议；网易云剩余工作中，云盘完整读写、下载和播放链路优先于播客目录及节目播放。网易云 Basic 与可导入任意平台歌单、混合添加并播放任意平台资源的 Uni Playlist 已收口；QQ Basic 已接入持久 Android 设备/QIMEI/会话及歌曲、歌手、专辑、歌单、MV、歌词统一搜索，继续完成用户和播客类搜索后进入歌曲详情/歌词、播放/VIP。最终全量要求不变，完整顺序、范围和定期上游检查规则见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 本地运行

```console
cargo run -p tuneweave-server --bin tuneweave
```

- `TUNEWEAVE_BIND`：监听地址，默认 `127.0.0.1:7832`。
- `TUNEWEAVE_DATA_DIR`：私有数据目录，默认 `.local/data`；成功登录的平台凭据按 `platform/account` 隔离保存并在重启时恢复。
- `TUNEWEAVE_NETEASE_COOKIE`：可选的网易云 `default` 账户启动 Cookie；不会写入响应或日志。通过登录端点取得的账户凭据则进入上述私有数据目录。
- `TUNEWEAVE_NETEASE_PROXY`：可选的服务端 HTTP(S) 正向代理 URL；仅在启动配置中读取，API 调用方不能覆盖。
- `TUNEWEAVE_NETEASE_REAL_IP`：可选的服务端固定 IPv4 请求身份，同时写入网易云协议请求的 `X-Real-IP` 与 `X-Forwarded-For`。
- `TUNEWEAVE_NETEASE_RANDOM_CN_IP`：设为 `true/yes/on/1` 时，启动网易云 provider 时生成一个中国 IPv4 请求身份，并像参考实现的 `global.cnIp` 一样由该实例的所有协议请求复用；短信验证码发送、校验与登录还会在同一 10 分钟事务窗口内固定匿名设备会话；不能与固定真实 IP 同时启用。
- `TUNEWEAVE_QQ_PROXY`：可选的 QQ 音乐服务端 HTTP(S) 正向代理 URL；仅从启动环境读取，API 调用方不能覆盖。QQ Android 设备、QIMEI 和匿名会话自动原子保存到私有数据目录的 `qq-device.json`，服务重启后复用。

默认数据目录已由 Git 忽略。账户文件只保存 provider 后续请求所需的会话凭据，不保存密码或验证码；Unix 创建权限为目录 `0700`、文件 `0600`，Windows 继承所选私有目录的 ACL。当前文件后端不执行静态加密，因此不要把该目录放进同步盘、公开目录、镜像或备份仓库；生产部署应显式把 `TUNEWEAVE_DATA_DIR` 指向仅服务账户可读写的位置。

Uni Playlist 使用同一私有数据目录下的 `uni-playlists.json` 单文件数据库，与平台账户凭据分离；创建、批量导入和编辑都先同步临时文件再发布，进程重启后会恢复歌单元数据及类型化项目。导入可用 `ref+type` 或 `platform+type+id` 合并多个公开/账户可见平台集合，账户别名按来源可选且彼此隔离。客户端可继续通过统一 `/v1/playlists/{ref}`、`/items` 和 `/tracks` 读取平台或 Uni 歌单，并通过稳定 `item_id` 的 `/stream` 以分平台账户执行原平台播放、指定平台播放及严格跨平台回退，无需维护第二套协议。

当前可直接调用 `/healthz`、`/v1/platforms`、`/v1/capabilities`、
`/v1/search`、`/v1/tracks/{ref}`、`/v1/albums/{ref}`、
`/v1/artists/{ref}`、`/v1/playlists/{ref}` 及其曲目、歌词、媒体和目录子端点。
认证已提供二维码、账号密码、短信验证码和退出端点；完整契约见
[docs/api-v1.md](docs/api-v1.md)。

## 许可证

TuneWeave 采用 [MIT OR Apache-2.0](LICENSE) 双许可，使用者可任选其一。上游研究来源和固定快照见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
