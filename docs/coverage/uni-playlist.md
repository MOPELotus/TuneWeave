# Uni Playlist 实施账本

Uni Playlist 是 TuneWeave 自有的跨平台歌单层，使用 `uni:<opaque-id>`，不依赖任何外部平台上游。状态含义：

- `pending`：尚未实现。
- `implemented`：代码及局部测试已完成，仍缺完整 HTTP、持久化或播放链验收。
- `verified`：核心契约、存储/路由和异常边界均已自动化验证；涉及外部 provider 时还需真实网络验证。

当前统计：`pending=0`、`implemented=0`、`verified=11`。

| 能力 | 状态 | 当前实现/缺口 |
| --- | --- | --- |
| `uni:` 资源命名空间 | `verified` | `Platform::Uni`、`ResourceRef` 解析/序列化和平台枚举均已接入；引用 ID 保持不透明，允许 URL-safe ASCII，平台发现独立声明 Uni 能力。 |
| 单文件持久化 | `verified` | 生产绑定 `TUNEWEAVE_DATA_DIR/uni-playlists.json`，与账户凭据分离；内存快照、同目录临时文件、刷盘、跨平台发布及 Windows 中断恢复已实现，重启读取、重复 ID、未知版本和不覆盖损坏文件均有测试。 |
| `POST /v1/uni/playlists` | `verified` | 创建空歌单，生成随机 `uni:pl_...` 引用，统一返回名称、描述、项目数及毫秒时间；长度、空值、未知 JSON/query 与碰撞重试边界已覆盖。 |
| `GET /v1/uni/playlists/{ref}` | `verified` | 从同一存储读取元数据；完整身份往返、错误平台、畸形 ID、不存在资源和未知查询均使用统一响应。 |
| `POST /v1/uni/playlists/imports` | `verified` | 一次接受 1–100 个有序来源，以 `ref+type` 或 `platform+type+id` 定位公开、账户可见或本地 Uni 歌单；`account` 逐来源可选，普通 `playlist` 为默认类型，provider 可扩展 `season/favorite_folder/favorite_tracks` 等可播放集合类型。逐来源完整翻页后按“来源顺序→来源内位置”合并，保留重复项、类型、来源引用、快照、来源索引和歌单级来源摘要，所有来源成功后才原子创建目标记录。已验证网易云与 QQ 公开来源、QQ 账户来源、本地 Uni，以及 B 站 `season/favorite_folder` 视频来源的跨来源合并、完整分页及重启恢复；B 站真实双来源按 617+98 原子写入 715 个视频项目。非可播放的用户目录只用于选择具体歌单，不做无界展开。 |
| `GET /v1/uni/playlists/{ref}/items` | `verified` | 分页返回类型化项目、稳定项目 ID、零基位置和紧凑元数据快照；`limit=1..100/offset`、真实总数、续页、空列表、缺失歌单和未知查询均已测试，重复来源项不会被折叠。 |
| `POST /v1/uni/playlists/{ref}/items` | `verified` | 一次原子追加 1–100 个 `track/mv/video/podcast_episode/radio_station`，逐项按来源 Provider 和分平台 `accounts` 解析真实快照，解析完成后才发布；错误平台/账户、`uni` 嵌套来源、空批次、未知字段、缺失目标和碰撞均有边界测试。歌曲重复项、MV、播客节目及广播电台的类型化快照和独立播放身份均已验收。 |
| `DELETE /v1/uni/playlists/{ref}/items/{item_id}` | `verified` | 按某一次出现的稳定项目 ID 原子删除并重编号后续位置；同一来源的其他重复项保持独立，未知/畸形项目 ID、缺失歌单和未知查询均有测试，文件存储重启后保持删除结果。 |
| `PATCH /v1/uni/playlists/{ref}/items/order` | `verified` | 原子提交当前全部项目 ID 的显式顺序并重编号零基位置；缺项、未知项、重复 ID 和畸形 ID 会整批拒绝且不改数据，重复来源项不折叠，无变化顺序明确返回 `changed=false`，文件存储重启后保持新顺序。 |
| `/v1/playlists` 统一读取适配 | `verified` | `GET /v1/playlists/{ref}` 已把本地元数据映射为现有 `Playlist`，`GET .../items` 以同一 `PlaylistPlayableEntry` 分页返回外部或 Uni 的歌曲、MV/视频音频、播客节目和广播电台，Uni 项保留稳定 `item_id`；`GET .../tracks` 对混合 Uni 内容仅筛选歌曲并返回筛选后的真实分页总数。外部 provider 的账户选择和分页不变，本地 `uni:` 明确拒绝无意义的 `account`；混合项目、重复歌曲、兼容视图、重启恢复与错误边界均已验证。 |
| Uni Playlist 播放与跨平台回退 | `verified` | `GET /v1/playlists/{ref}/items/{item_id}/stream` 以稳定项目 ID 播放，所有类型统一返回 `UniPlaylistItemStream` 内的 `MediaStream`，并提供 `/redirect`。歌曲使用持久化快照作为严格来源身份；播客先解析承载音频；MV/视频在原生视频流与其他平台严格匹配音频之间按 `playback_platform/fallback_platforms/fallback/unblock` 的精确顺序切换；广播刷新原平台直播 URL。`accounts` 支持列表或 JSON 对象并与兼容 `account` 的首目标语义隔离，全部尝试、账户、候选、分数及失败状态均保留；原平台播放、跨平台命中、降级音质、广播、302、重复项目和重启恢复均已验证。 |
