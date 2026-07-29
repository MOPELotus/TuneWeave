# TuneWeave

统一、可扩展的跨平台音乐 API，为不同音乐平台提供一致的访问接口。

TuneWeave 使用 Rust 构建，目标是在保持较小存储体积、较低运行占用和快速启动的同时，把不同音乐平台的检索、歌单、媒体解析和账户能力统一为稳定的 HTTP API。

## 设计方向

- 同一种业务能力使用相同端点和统一输入输出结构。
- 账户请求可通过 `platform + account` 选择服务器持久化的多账户登录态，也可通过 `X-TuneWeave-Credential` 按请求携带调用方自管凭证。QQ、网易云与 B 站登录均支持 `credential_mode=server|client|both`，分别对应仅保存、仅返回或保存并返回同一凭证代际；各平台已实现的账户与业务端点按[调用方托管凭证契约](docs/credential-ownership.md)逐项接通。
- 内容来源与播放来源解耦：歌单来自一个平台时，音频可按策略从其他平台解析。
- 指定或默认平台播放失败后，可按可配置顺序回退到其他平台。
- 平台适配器按能力声明接入，不要求每个平台实现不存在的功能。

## 计划接入顺序

1. 收口网易云、QQ 音乐和 B 站项目范围当前主线。
2. 实施 Uni Playlist 客户端托管、服务端多歌单目录和存储改造。
3. 接入酷狗、咪咕和酷我的公开音源补充层。
4. 补齐后台日志与测试版可观测性，执行全端点真实发布验收。
5. 实施 B 站延期的 GeeTest、密码与短信登录链。
6. 依次完成酷狗、咪咕和酷我的完整项目范围。
7. 六个平台稳定后实施验证码服务本地 Rust 版本。
8. 只在出现明确需求时，从平台功能扩展候选池选择能力实施。

“项目范围”是平台在 TuneWeave 产品定位内计划完成的全部正式能力，不是精简版或通往“全量覆盖”的过渡阶段。参考项目中超出检索、整理、播放和必要账户体验的接口仅进入扩展候选池，不参与完成率。前三个平台收口后，Uni Playlist 将先补齐 `server/client` 所有权、无状态处理、显式复制和可扩展持久化，再由酷狗、咪咕和酷我的公开搜索、元数据、歌词与播放回退增强音源覆盖；此后完成后台日志与全端点真实验收，才达到首个 pre-release 门槛。具体范围见[路线图](docs/implementation-plan.md)，Uni 设计见[客户端托管与存储规划](docs/uni-playlist-ownership.md)，当前覆盖状态见 [`docs/coverage`](docs/coverage/)，发布门槛见[测试版验收标准](docs/pre-release-acceptance.md)。

## 本地运行

```console
cargo run -p tuneweave-server --bin tuneweave
```

- `TUNEWEAVE_BIND`：监听地址，默认 `127.0.0.1:7832`。
- `TUNEWEAVE_DATA_DIR`：私有数据目录，默认 `.local/data`；成功登录的平台凭据按 `platform/account` 隔离保存并在重启时恢复。
- `TUNEWEAVE_NETEASE_COOKIE`：可选的网易云 `default` 账户启动 Cookie；不会写入响应或日志。服务器托管模式通过登录端点取得的账户凭据进入上述私有数据目录；调用方自管凭证只在支持的请求作用域内使用，不会写入该目录。
- `TUNEWEAVE_NETEASE_PROXY`：可选的服务端 HTTP(S) 正向代理 URL；仅在启动配置中读取，API 调用方不能覆盖。
- `TUNEWEAVE_NETEASE_REAL_IP`：可选的服务端固定 IPv4 请求身份，同时写入网易云协议请求的 `X-Real-IP` 与 `X-Forwarded-For`。
- `TUNEWEAVE_NETEASE_RANDOM_CN_IP`：设为 `true/yes/on/1` 时，启动网易云 provider 时生成一个中国 IPv4 请求身份，并像参考实现的 `global.cnIp` 一样由该实例的所有协议请求复用；短信验证码发送、校验与登录还会在同一 10 分钟事务窗口内固定匿名设备会话；不能与固定真实 IP 同时启用。
- `TUNEWEAVE_QQ_PROXY`：可选的 QQ 音乐服务端 HTTP(S) 正向代理 URL；仅从启动环境读取，API 调用方不能覆盖。QQ Android 设备、QIMEI 和匿名会话自动原子保存到私有数据目录的 `qq-device.json`，服务重启后复用。
- `TUNEWEAVE_BILIBILI_PROXY`：可选的 B 站服务端 HTTP(S) 正向代理 URL；仅从启动环境读取，API 调用方不能覆盖。

默认数据目录已由 Git 忽略。账户文件只保存 provider 后续请求所需的会话凭据，不保存密码或验证码；Unix 创建权限为目录 `0700`、文件 `0600`，Windows 继承所选私有目录的 ACL。当前文件后端不执行静态加密，因此不要把该目录放进同步盘、公开目录、镜像或备份仓库；生产部署应显式把 `TUNEWEAVE_DATA_DIR` 指向仅服务账户可读写的位置。

Uni Playlist 的 Server 模式在私有数据目录 `uni-playlists/` 中按歌单保存版本化记录；修改一个歌单只原子发布对应文件，进程重启后由记录重建目录和类型化项目。当前尚未发布稳定版本，存储只保证全新部署使用这一目录格式，不提供开发期旧文件兼容。导入可用 `ref+type` 或 `platform+type+id` 合并多个公开/账户可见平台集合，账户别名按来源可选且彼此隔离。客户端可通过统一 `/v1/playlists/{ref}`、`/items` 和 `/tracks` 读取平台或 Uni 歌单，并通过稳定 `item_id` 的 `/stream` 以分平台账户执行原平台播放、指定平台播放及严格跨平台回退。

服务端多歌单目录已可通过 `GET /v1/uni/playlists` 分页读取，不再要求调用方自行记住全部 `uni:<id>`；`PATCH /v1/uni/playlists/{ref}` 可独立修改名称或描述，`DELETE` 可原子删除整份歌单及其项目。Client 托管使用强类型 `tuneweave_uni_playlist_v1` 交换文档；服务端歌单可通过 `/export` 取得完整客户端副本，客户端文档也可通过 `/import-document` 原子复制回 Server。`/v1/uni/materialize/imports`、`/materialize/items` 与 `/v1/uni/items/stream` 已组成不持久化的来源展开、资源标准化和播放回退链。无外部数据库依赖的按歌单拆分存储已经投入生产入口；超大导出传输仍在本阶段继续收口。两种所有权模式只通过显式导入/导出复制，不提供自动 `both` 同步。详细边界见 [Uni Playlist 客户端托管与存储设计](docs/uni-playlist-ownership.md)。

当前可直接调用 `/healthz`、`/v1/platforms`、`/v1/capabilities`、
`/v1/search`、`/v1/tracks/{ref}`、`/v1/albums/{ref}`、
`/v1/artists/{ref}`、`/v1/users/{ref}`、`/v1/account/profile`、
`/v1/playlists/{ref}` 及其曲目、歌词、媒体和目录子端点。
认证已提供二维码、账号密码、短信验证码和退出端点；完整契约见
[docs/api-v1.md](docs/api-v1.md)。

## 许可证

TuneWeave 采用 [MIT OR Apache-2.0](LICENSE) 双许可，使用者可任选其一。上游研究来源和固定快照见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
