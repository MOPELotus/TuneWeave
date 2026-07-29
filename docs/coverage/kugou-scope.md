# 酷狗项目范围覆盖账本

协议基线为 `MakcRe/KuGouMusicApi@283f1e97b110726b208a64b486a657c0fc0a6126`，并以当前酷狗平台真实响应校正过时分支。状态沿用其他平台账本：`pending` 尚未实现，`implemented` 已完成代码和离线验证但缺真实成功态，`verified` 已完成统一 HTTP 与真实上游验收。

当前公开音源补充层共 7 个验收单元：`pending=4`、`implemented=0`、`verified=3`，完成度 `3/7 = 42.86%`。登录、账户和写操作属于后续酷狗完整项目范围，不阻塞本层。

| ID | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- |
| KG01 | 公开歌曲搜索与稳定身份 | `verified` | `GET /v1/search?platform=kugou&kind=track` 使用固定 HTTPS Web 搜索端点，匿名返回 `kugou:<album_audio_id>`、标题、歌手、专辑、封面、时长、MV 身份、基础/高品/无损/Hi-Res 哈希和权益诊断；任意 offset 通过受限双页窗口精确切片。Android 搜索现行 `152` 分支不再冒充可用前置链；真实 HTTP 搜索“周杰伦”已成功。 |
| KG02 | 歌曲详情与身份补全 | `verified` | `GET /v1/tracks/kugou:<album_audio_id>` 先以固定 HTTPS Android 网关读取 KRM 元数据，再用其返回的独立 `audio_id` 读取 KMR 媒体规格；两段响应均反查请求身份，不能把 `album_audio_id` 当成 `audio_id`。返回歌手、专辑、发行日期、版本、语言、分类及标准/高品/无损/Hi-Res/母带实际存在的规格；匿名真实 HTTP 已用 `kugou:32100650` 验收，`playable` 在权益链完成前保持未知。 |
| KG03 | 普通与逐字歌词 | `verified` | `GET /v1/tracks/kugou:<album_audio_id>/lyrics` 以歌曲详情中的稳定身份、哈希、时长和艺人标题搜索歌词候选，优先使用平台 proposal，再以内部 `id + accesskey` 分别下载 KRC 与 LRC；KRC 校验文件头后受限 XOR、zlib 和 UTF-8 解码，LRC 单独保留。`format=krc` 和 `word_synced` 的优先级始终高于普通 `plain`，嵌入语言目录按原生类型分离翻译与音译。真实 HTTP 已验证“晴天”的 KRC/LRC 双格式，以及“打上花火”的中文翻译和罗马音；临时 accesskey 不返回、不记录。 |
| KG04 | 公开播放、下载与权益 | `pending` | 按明确音质选择真实 URL，保留实际音质、试听窗口、必要请求头与到期时间；不伪造会员权益。 |
| KG05 | 统一播放回退与 302 | `pending` | 接入 `MediaStream`、下载语义、受信媒体地址 302 和 resolver；完整保留来源、严格匹配及失败轨迹。 |
| KG06 | 公开歌单与 Uni 导入 | `pending` | 公开歌单元数据和完整分页映射为可播放项目，支持服务端与客户端托管 Uni Playlist 来源展开。 |
| KG07 | 移动协议与匿名设备身份 | `pending` | 仅在依赖移动端接口时注册并持久化 GUID/MID/dfid，按 provider 隔离且不接受调用方注入；签名、业务错误和设备失效刷新均须真实验证。 |

后续完整项目范围将在公开音源层收口后扩展登录/会话、多账户、调用方凭证、账户资料与会员状态、完整媒体搜索、个人歌单/喜欢、登录权益播放等验收单元；纯社交、直播互动、装扮和商城不纳入。
