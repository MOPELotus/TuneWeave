# Uni Playlist 实施账本

Uni Playlist 是 TuneWeave 自有的跨平台歌单层，使用 `uni:<opaque-id>`，不依赖任何外部平台上游。状态含义：

- `pending`：尚未实现。
- `implemented`：代码及局部测试已完成，仍缺完整 HTTP、持久化或播放链验收。
- `verified`：核心契约、存储/路由和异常边界均已自动化验证；涉及外部 provider 时还需真实网络验证。

当前统计：`pending=8`、`implemented=0`、`verified=11`。

当前已实现的是服务端托管模式：底层支持多个独立 Uni Playlist，但共同保存于一个 `uni-playlists.json`，且尚无服务端歌单目录。客户端托管、无状态 materialize/播放、显式迁移和服务端存储改造已完成设计并进入正式路线，但按照阶段顺序等待 B 站当前项目范围主线收口后实施；完整边界见 [Uni Playlist 客户端托管与存储设计](../uni-playlist-ownership.md)。

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
| `tuneweave_uni_playlist_v1` 客户端交换文档 | `pending` | 规划版本化歌单、稳定项目 ID、顺序、类型、来源和快照契约；JSON 只用于导入、导出、备份和交换，不规定客户端内部存储。凭证、Cookie、token、设备身份、临时媒体 URL、签名和任意请求头不得进入文档。 |
| 无状态平台来源展开 | `pending` | 规划 `POST /v1/uni/materialize/imports`：完整分页展开一个或多个可播放集合，保留来源与内部顺序及重复项，把结果返回客户端但不创建 `uni:<id>`、不写服务端存储；大型结果需要受控分页、流式或压缩传输。 |
| 无状态资源标准化 | `pending` | 规划 `POST /v1/uni/materialize/items`：批量解析歌曲、MV、视频、播客节目和广播的真实元数据并返回标准化项目；限制批量、响应大小、超时和上游分页。 |
| 客户端托管项目播放 | `pending` | 规划 `POST /v1/uni/items/stream`：只提交当前项目和播放控制即可执行原平台播放及严格回退，服务端不要求完整歌单且不持久化项目；可选短期内存票据承接 GET/302，票据不得含可篡改目标 URL。 |
| 服务端多歌单目录与元数据管理 | `pending` | 规划分页 `GET /v1/uni/playlists` 及明确的 `PATCH/DELETE /v1/uni/playlists/{ref}`；稳定列出多个服务端歌单而不内联项目，修改和删除互不影响其他歌单，外部来源失效不得触发自动删除。 |
| Server 导出与 Client 文档导入 | `pending` | 规划完整导出和原子 `import-document`；保留顺序、重复项、稳定项目 ID 和快照，默认生成新服务端 ID，冲突不覆盖。迁移只做显式复制，不提供 `both` 自动同步。 |
| 服务端持久化分片或嵌入式数据库 | `pending` | 在按歌单拆分文件与嵌入式数据库之间按体积、可移植性、事务和维护成本选型，使单歌单修改不再克隆并重写全局数据库；不得依赖独立外部数据库服务。 |
| 旧单文件安全迁移 | `pending` | 将现有 `<TUNEWEAVE_DATA_DIR>/uni-playlists.json` 一次性迁移到新存储，保留多歌单、顺序、重复项和项目身份；失败不得覆盖原文件，Windows 中断后必须可恢复。 |
