# Uni Playlist 实施账本

Uni Playlist 是 TuneWeave 自有的跨平台歌单层，使用 `uni:<opaque-id>`，不依赖任何外部平台上游。状态含义：

- `pending`：尚未实现。
- `implemented`：代码及局部测试已完成，仍缺完整 HTTP、持久化或播放链验收。
- `verified`：核心契约、存储/路由和异常边界均已自动化验证；涉及外部 provider 时还需真实网络验证。

当前统计：`pending=3`、`implemented=0`、`verified=8`。

| 能力 | 状态 | 当前实现/缺口 |
| --- | --- | --- |
| `uni:` 资源命名空间 | `verified` | `Platform::Uni`、`ResourceRef` 解析/序列化和平台枚举均已接入；引用 ID 保持不透明，允许 URL-safe ASCII，平台发现独立声明 Uni 能力。 |
| 单文件持久化 | `verified` | 生产绑定 `TUNEWEAVE_DATA_DIR/uni-playlists.json`，与账户凭据分离；内存快照、同目录临时文件、刷盘、跨平台发布及 Windows 中断恢复已实现，重启读取、重复 ID、未知版本和不覆盖损坏文件均有测试。 |
| `POST /v1/uni/playlists` | `verified` | 创建空歌单，生成随机 `uni:pl_...` 引用，统一返回名称、描述、项目数及毫秒时间；长度、空值、未知 JSON/query 与碰撞重试边界已覆盖。 |
| `GET /v1/uni/playlists/{ref}` | `verified` | 从同一存储读取元数据；完整身份往返、错误平台、畸形 ID、不存在资源和未知查询均使用统一响应。 |
| `POST /v1/uni/playlists/imports` | `pending` | 需遍历任意已注册 provider 歌单分页，保留顺序、重复项、类型、来源引用和元数据快照。 |
| `GET /v1/uni/playlists/{ref}/items` | `verified` | 分页返回类型化项目、稳定项目 ID、零基位置和紧凑元数据快照；`limit=1..100/offset`、真实总数、续页、空列表、缺失歌单和未知查询均已测试，重复来源项不会被折叠。 |
| `POST /v1/uni/playlists/{ref}/items` | `verified` | 一次原子追加 1–100 个 `track/mv/video/podcast_episode`，逐项按来源 Provider 和分平台 `accounts` 解析真实快照，解析完成后才单次发布；错误平台/账户、`uni` 嵌套来源、空批次、未知字段、缺失目标和碰撞均有边界测试。2026-07-22 真实二进制 HTTP 成功把网易云歌曲重复两次、MV 和播客节目按四个独立项目写入，顺序 `0..3`、标题/艺人/时长/视频类型及节目独立音频引用均正确，数据库共 2309 字节。 |
| `DELETE /v1/uni/playlists/{ref}/items/{item_id}` | `verified` | 按某一次出现的稳定项目 ID 原子删除并重编号后续位置；同一来源的其他重复项保持独立，未知/畸形项目 ID、缺失歌单和未知查询均有测试，文件存储重启后保持删除结果。2026-07-22 真实 release 二进制从两次重复的网易云歌曲中只删除第一项，另一项仍保留。 |
| `PATCH /v1/uni/playlists/{ref}/items/order` | `verified` | 原子提交当前全部项目 ID 的显式顺序并重编号零基位置；缺项、未知项、重复 ID 和畸形 ID 会整批拒绝且不改数据，重复来源项不折叠，无变化顺序明确返回 `changed=false`，文件存储重启后保持新顺序。2026-07-22 真实 release 二进制将剩余 MV 与歌曲重排为 `0,1`，重启后顺序一致，数据库为 1332 字节。 |
| `/v1/playlists` 统一读取适配 | `pending` | 需让现有歌单详情/项目读取识别 `uni:`，避免客户端维护第二套读取模型。 |
| Uni Playlist 播放与跨平台回退 | `pending` | 需先尝试原始平台，再按 `playback_platform`、分平台账户和严格元数据匹配执行有序回退。 |
