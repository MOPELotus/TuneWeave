# Uni Playlist 实施账本

Uni Playlist 是 TuneWeave 自有的跨平台歌单层，使用 `uni:<opaque-id>`，不依赖任何外部平台上游。状态含义：

- `pending`：尚未实现。
- `implemented`：代码及局部测试已完成，仍缺完整 HTTP、持久化或播放链验收。
- `verified`：核心契约、存储/路由和异常边界均已自动化验证；涉及外部 provider 时还需真实网络验证。

当前统计：`pending=1`、`implemented=0`、`verified=10`。

| 能力 | 状态 | 当前实现/缺口 |
| --- | --- | --- |
| `uni:` 资源命名空间 | `verified` | `Platform::Uni`、`ResourceRef` 解析/序列化和平台枚举均已接入；引用 ID 保持不透明，允许 URL-safe ASCII，平台发现独立声明 Uni 能力。 |
| 单文件持久化 | `verified` | 生产绑定 `TUNEWEAVE_DATA_DIR/uni-playlists.json`，与账户凭据分离；内存快照、同目录临时文件、刷盘、跨平台发布及 Windows 中断恢复已实现，重启读取、重复 ID、未知版本和不覆盖损坏文件均有测试。 |
| `POST /v1/uni/playlists` | `verified` | 创建空歌单，生成随机 `uni:pl_...` 引用，统一返回名称、描述、项目数及毫秒时间；长度、空值、未知 JSON/query 与碰撞重试边界已覆盖。 |
| `GET /v1/uni/playlists/{ref}` | `verified` | 从同一存储读取元数据；完整身份往返、错误平台、畸形 ID、不存在资源和未知查询均使用统一响应。 |
| `POST /v1/uni/playlists/imports` | `verified` | 一次接受 1–100 个有序来源，以 `ref+type` 或 `platform+type+id` 定位公开、账户可见或本地 Uni 歌单；`account` 逐来源可选，普通 `playlist` 为默认类型，provider 可扩展 `season/favorite_folder` 等集合类型。逐来源完整翻页后按“来源顺序→来源内位置”合并，保留重复项、类型、来源引用、快照、来源索引和歌单级来源摘要，所有来源成功后才单次创建目标文件记录。自动化测试以网易云公开来源和带独立账户的 QQ 两页来源完成跨 provider 合并，并再次合并本地 Uni。2026-07-22 真实 release 二进制匿名导入网易云“热歌榜”200 项与“飙升榜”100 项，共 300 项、188431 字节；重启后总数、持久化来源摘要及第二来源边界均与原歌单一致。 |
| `GET /v1/uni/playlists/{ref}/items` | `verified` | 分页返回类型化项目、稳定项目 ID、零基位置和紧凑元数据快照；`limit=1..100/offset`、真实总数、续页、空列表、缺失歌单和未知查询均已测试，重复来源项不会被折叠。 |
| `POST /v1/uni/playlists/{ref}/items` | `verified` | 一次原子追加 1–100 个 `track/mv/video/podcast_episode/radio_station`，逐项按来源 Provider 和分平台 `accounts` 解析真实快照，解析完成后才单次发布；错误平台/账户、`uni` 嵌套来源、空批次、未知字段、缺失目标和碰撞均有边界测试。2026-07-22 真实二进制 HTTP 成功把网易云歌曲重复两次、MV 和播客节目按四个独立项目写入，顺序 `0..3`、标题/艺人/时长/视频类型及节目独立音频引用均正确，数据库共 2309 字节；随后真实 release 二进制又将 `netease:175` 解析为可直接播放的“河北音乐广播”电台项目。 |
| `DELETE /v1/uni/playlists/{ref}/items/{item_id}` | `verified` | 按某一次出现的稳定项目 ID 原子删除并重编号后续位置；同一来源的其他重复项保持独立，未知/畸形项目 ID、缺失歌单和未知查询均有测试，文件存储重启后保持删除结果。2026-07-22 真实 release 二进制从两次重复的网易云歌曲中只删除第一项，另一项仍保留。 |
| `PATCH /v1/uni/playlists/{ref}/items/order` | `verified` | 原子提交当前全部项目 ID 的显式顺序并重编号零基位置；缺项、未知项、重复 ID 和畸形 ID 会整批拒绝且不改数据，重复来源项不折叠，无变化顺序明确返回 `changed=false`，文件存储重启后保持新顺序。2026-07-22 真实 release 二进制将剩余 MV 与歌曲重排为 `0,1`，重启后顺序一致，数据库为 1332 字节。 |
| `/v1/playlists` 统一读取适配 | `verified` | `GET /v1/playlists/{ref}` 已把本地元数据映射为现有 `Playlist`，`GET .../items` 以同一 `PlaylistPlayableEntry` 分页返回外部或 Uni 的歌曲、MV/视频音频、播客节目和广播电台，Uni 项保留稳定 `item_id`；`GET .../tracks` 对混合 Uni 内容仅筛选歌曲并返回筛选后的真实分页总数。外部 provider 的既有账户选择和分页不变，本地 `uni:` 明确拒绝无意义的 `account`。详情、混合项目、重复歌曲、歌曲兼容视图、外部来源与错误边界均有 HTTP 测试。2026-07-22 真实 release 二进制匿名解析并混合写入两次网易云歌曲、MV、播客节目和广播电台共 5 项；统一项目视图依次返回 `track,track,mv,podcast_episode,radio_station` 及全部稳定 ID，歌曲兼容视图返回真实 `total=2`，重启后 5 项完整恢复，数据库为 2963 字节。 |
| Uni Playlist 播放与跨平台回退 | `pending` | 需先尝试原始平台，再按 `playback_platform`、分平台账户和严格元数据匹配执行有序回退。 |
