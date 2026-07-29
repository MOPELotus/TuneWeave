# 酷狗项目范围覆盖账本

协议基线为 `MakcRe/KuGouMusicApi@283f1e97b110726b208a64b486a657c0fc0a6126`，并以当前酷狗平台真实响应校正过时分支。状态沿用其他平台账本：`pending` 尚未实现，`implemented` 已完成代码和离线验证但缺真实成功态，`verified` 已完成统一 HTTP 与真实上游验收。

当前公开音源补充层共 7 个验收单元：`pending=2`、`implemented=0`、`verified=5`，完成度 `5/7 = 71.43%`。登录、账户和写操作属于后续酷狗完整项目范围，不阻塞本层。

| ID | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- |
| KG01 | 公开歌曲搜索与稳定身份 | `verified` | `GET /v1/search?platform=kugou&kind=track` 使用固定 HTTPS Web 搜索端点，匿名返回 `kugou:<album_audio_id>`、标题、歌手、专辑、封面、时长、MV 身份、基础/高品/无损/Hi-Res 哈希和权益诊断；任意 offset 通过受限双页窗口精确切片。Android 搜索现行 `152` 分支不再冒充可用前置链；真实 HTTP 搜索“周杰伦”已成功。 |
| KG02 | 歌曲详情与身份补全 | `verified` | `GET /v1/tracks/kugou:<album_audio_id>` 先以固定 HTTPS Android 网关读取 KRM 元数据，再用其返回的独立 `audio_id` 读取 KMR 媒体规格；两段响应均反查请求身份，不能把 `album_audio_id` 当成 `audio_id`。返回歌手、专辑、发行日期、版本、语言、分类及标准/高品/无损/Hi-Res/母带实际存在的规格；匿名真实 HTTP 已用 `kugou:32100650` 验收，`playable` 在权益链完成前保持未知。 |
| KG03 | 普通与逐字歌词 | `verified` | `GET /v1/tracks/kugou:<album_audio_id>/lyrics` 以歌曲详情中的稳定身份、哈希、时长和艺人标题搜索歌词候选，优先使用平台 proposal，再以内部 `id + accesskey` 分别下载 KRC 与 LRC；KRC 校验文件头后受限 XOR、zlib 和 UTF-8 解码，LRC 单独保留。`format=krc` 和 `word_synced` 的优先级始终高于普通 `plain`，嵌入语言目录按原生类型分离翻译与音译。真实 HTTP 已验证“晴天”的 KRC/LRC 双格式，以及“打上花火”的中文翻译和罗马音；临时 accesskey 不返回、不记录。 |
| KG04 | 公开播放、下载与权益 | `verified` | 统一 stream/download 先按明确的音质降级顺序选择真实哈希，再经移动端 privilege 与 tracker 两段身份和权益校验；只允许固定 HTTPS 网关和受信 `*.kugou.com` 媒体地址，平台 HTTP 媒体地址仅同主机升级为 HTTPS。会员歌曲匿名播放只返回明确的试听窗口，下载保持 `available=false`；公开歌曲返回完整下载 URL。真实 HTTP 已验证“晴天”0–60 秒、960116 字节试听与“两只老虎”77 秒、1248581 字节完整下载，媒体 HEAD 状态和长度均一致。 |
| KG05 | 统一播放回退与 302 | `verified` | 酷狗已进入统一 resolver、解灰来源顺序、歌曲/Uni 播放和无缓存 302；Uni 安全快照只保存跨平台匹配字段，取流时以稳定 `album_audio_id` 重取 provider 私有媒体规格。统一下载跳转只在获得完整文件时使用专用地址或全曲播放地址，试听分支返回 `permission_denied` 而不发 302。真实 HTTP 已验证 `netease:186016` 严格匹配到 `kugou:32100650`（分数 1.0、完整来源轨迹及 60 秒试听），公开歌曲 JSON、stream/download 302、会员歌曲下载拒绝，以及持久化 Uni 项 `kugou:100063739` 的播放和 302。 |
| KG06 | 公开歌单与 Uni 导入 | `pending` | 公开歌单元数据和完整分页映射为可播放项目，支持服务端与客户端托管 Uni Playlist 来源展开。 |
| KG07 | 移动协议与匿名设备身份 | `pending` | 仅在依赖移动端接口时注册并持久化 GUID/MID/dfid，按 provider 隔离且不接受调用方注入；签名、业务错误和设备失效刷新均须真实验证。 |

后续完整项目范围将在公开音源层收口后扩展登录/会话、多账户、调用方凭证、账户资料与会员状态、完整媒体搜索、个人歌单/喜欢、登录权益播放等验收单元；纯社交、直播互动、装扮和商城不纳入。
