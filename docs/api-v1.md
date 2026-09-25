# TuneWeave HTTP API v1

## 基础约定

- 基址默认为 `http://127.0.0.1:7832`。
- 业务 API 使用 `/v1` 前缀，存活检查为 `/healthz`。
- 请求与响应使用 UTF-8 JSON；媒体内容端点除外。
- 时间使用 RFC 3339，带 `_at_ms` 的字段使用 Unix 毫秒；时长为毫秒，大小为字节，码率为 bit/s。
- 平台原始 ID 按字符串处理。资源引用写成 `<platform>:<id>`，例如 `netease:123456`、`qq:0039MnYb0qxYhV`、`bilibili:bvid:BV1xx411c7mD`。
- 未知查询字段或 JSON 字段通常返回 `400 invalid_request`，调用方不应依赖未记录的宽松解析。

完整的 `method + path` 目录见 [`routes.json`](routes.json)。运行实例的实际平台与能力通过以下端点发现：

```http
GET /v1/platforms
GET /v1/capabilities
GET /v1/capabilities?platform=netease
```

显式上游会话撤销使用 `POST /v1/auth/session/revoke`，汽水与酷我声明 `session_revocation` 能力。支持服务器账户、调用方凭据以及同次登录的 `both`；成功结果区分请求后确认失效、原已失效和本地无别名，不以本地删除代替上游确认。完整请求、状态及未确认错误说明见[会话管理](authentication.md#会话管理)。

## 听歌历史

`GET /v1/account/history` 返回平台的听歌历史模型（歌曲、播放次数和可选评分），与下面的最近歌曲/专辑/歌单记录是独立能力。`period` 默认为 `all_time`，可用 `all`、`all_time` 或 `week`；`limit` 默认为 30，范围 1–100；`offset` 默认为 0。需要已选择的服务器账号或 `X-TuneWeave-Credential`。

目前网易云和酷狗 Standard/Concept 原生账号支持 `listening_history`，其他平台返回 `422 capability_not_supported`。网易云返回其周榜/累计历史结果；酷狗返回选中原生账号的播放历史。酷狗按时间游标读取并在本地应用周窗口和 offset/limit，删除操作记录不作为播放项。Concept 仅支持本人主设备 `device_type=1`；同一歌曲按最新 `op_time` 合并，同时间保留上游首个动作，删除动作不会显示为播放项。`last_played_at` 使用 UTC RFC 3339 秒精度；分页 `extensions.complete_read` 表示酷狗是否读完全部上游记录。酷狗最多完整扫描 1,000 条唯一记录；达到上限时 `total` 为 `null`、`complete_read=false`，结果只代表已读取的部分，不能当作完整历史。Concept 的来源标记为 `backend=concept_youth_history`，不同于 Standard 原生历史。

## 当前账号的最近记录

alpha.11 新增 `GET /v1/account/history/tracks`、`/v1/account/history/albums`、`/v1/account/history/playlists`，分别对应 `recent_track_history`、`recent_album_history`、`recent_playlist_history`。目前网易云支持，其他平台返回 `422 capability_not_supported`。

查询参数为 `platform`、`account`、`limit`（默认 100，1–100）、`offset`（仅 0）。可使用 `X-TuneWeave-Credential`，但不能同时指定 `account`。响应记录包含规范化 `track` / `album` / `playlist`、毫秒精度 RFC 3339 `played_at`、可选 `device` 和 `extensions`；使用标准列表包络与 `meta.pagination`。

这些是最近记录，使用独立的 `recent_*_history` 能力；旧 `/v1/account/history` 的 `listening_history` 含义不变。网易云只提供最近 N 条窗口，因此 `next_offset=null`、`has_more=false`、`extensions.continuation_supported=false`；`total` 不代表可翻页读取的数量。字段语义、完整记录示例和异常处理见 [最近记录契约](recent-history.md)。

## 听歌打卡 / scrobble

`POST /v1/tracks/{reference}/scrobble` 提交一次实际收听记录。目前仅网易云支持，能力名为 `scrobble_write`，可通过 `/v1/capabilities?platform=netease` 查询。

```http
POST /v1/tracks/netease:123456/scrobble
Content-Type: application/json
X-TuneWeave-Credential: twc1_<opaque-base64url>

{
  "played_ms": 90123,
  "duration_ms": 210456,
  "bitrate": 320000,
  "quality": "high"
}
```

| 字段 | 必填 | 含义 |
| --- | --- | --- |
| 路径 `reference` | 是 | 实际播放曲目的平台引用，如 `netease:123456`；跨平台回退后使用实际播放来源的引用。 |
| `played_ms` | 是 | 实际收听毫秒数，正整数；排除暂停和跳转跳过的部分。 |
| `duration_ms` | 是 | 曲目总毫秒数，正整数；必须不小于 `played_ms`，上限为 4,294,967,295,000。 |
| `bitrate` | 是 | 实际音频码率，单位 **bit/s**，范围 1–4,294,967,295；320 kbps 填 `320000`。 |
| `quality` | 是 | 实际音质：`standard`、`higher`、`high`、`lossless`、`hires`、`surround`、`spatial`、`dolby`、`master`、`vivid`。不能填 `auto` 或 `low`。 |
| `account` | 否 | TuneWeave 托管账号别名，省略时使用 `default`；使用调用方凭据头时不得同时指定非空别名。 |

不要把请求音质误当作实际音质：资源降级或回退后应填实际播放结果。毫秒和 bit/s 到平台日志单位的转换由 TuneWeave 处理，调用方无需截断为整数秒或 kbps。重复播放应分别上报，不能累加成超过曲目总长的一次记录。

成功返回现有 `ApiResponse` 信封，其中 `data` 为：

```json
{
  "track_ref": "netease:123456",
  "accepted": true,
  "played_ms": 90123,
  "duration_ms": 210456,
  "bitrate": 320000,
  "quality": "high"
}
```

`accepted: true` 表示网易云已确认接收开始及结束两个阶段的日志，不保证听歌榜、打卡计数或历史记录立即更新。TuneWeave 不决定提交阈值、不自动统计收听、不调度重复上报。

失败沿用规范化 `error`：参数错误为 `400 invalid_request`，未登录或会话失效为 `401 authentication_required`，权限拒绝为 `403 permission_denied`，限流为 `429 rate_limited`，不支持的平台为 `422 capability_not_supported`，上游异常为 `502 upstream_error`。上传失败时 `error.details` 包含 `stage`（`start` / `complete`）、`start_accepted`、`delivery_may_have_occurred` 和经过筛选的 `upstream` 状态。错误为 `retryable: false`，禁止把超时或部分成功当成未送达直接重试；该接口不提供跨请求幂等保证。返回或传输结果不确定时应由客户端保留该状态，避免重复统计同一次播放。

Minecraft 客户端直接与 TuneWeave 通信，凭据按[登录与凭证](authentication.md)保存在客户端或 TuneWeave 中，不经过 Minecraft 服务器。凭据只放 `X-TuneWeave-Credential` 请求头，不放请求体、URL 或曲目 reference；平台日志协议、加密和账号鉴权由 TuneWeave 内部处理。

## 请求关联

调用方可以发送 `X-Request-ID`。值必须为 1–64 个 ASCII 字符，以字母或数字开头，其余字符只能是字母、数字、`-`、`_`、`.` 或 `:`。服务端会在响应头和 JSON 的 `meta.request_id` 中返回最终值。

## 平台、账户与播放来源

| 输入 | 说明 |
| --- | --- |
| `platform` | 内容目录或账户所属平台；部分搜索端点接受 `all` |
| `account` | 目标平台的服务器托管账户别名，默认 `default` |
| `X-TuneWeave-Credential` | 可重复请求头，每项携带一个平台的调用方托管凭证 |
| `playback_platform` | 首选播放来源，不改变内容原始引用 |
| `fallback` | 是否在播放失败后继续尝试其他平台，默认 `true` |
| `fallback_platforms` | 逗号分隔的有序回退平台列表 |
| `unblock` | 是否启用平台受限音源解锁阶段，默认 `true`；设为 `false` 可只请求原始平台或显式回退 |
| `source` | 解锁阶段的首选公开音源；可与 `playback_platform`、`fallback_platforms` 组合 |
| `accounts` | JSON 请求中按平台键控的账户别名对象 |

路径中的资源引用决定内容平台。账户别名按平台隔离。同一平台不能同时使用显式 `account` 和调用方凭证。登录及凭证格式见[登录与凭证](authentication.md)。

## 响应包络

成功：

```json
{
  "ok": true,
  "data": {},
  "meta": {
    "request_id": "tw-...",
    "platform": "netease",
    "account": "default",
    "cached": false
  }
}
```

失败：

```json
{
  "ok": false,
  "error": {
    "code": "authentication_required",
    "message": "authentication is required",
    "platform": "qq",
    "retryable": false,
    "details": {}
  },
  "meta": {
    "request_id": "tw-..."
  }
}
```

| HTTP | `error.code` | 说明 |
| ---: | --- | --- |
| 400 | `invalid_request` | 参数、引用或请求体无效 |
| 401 | `authentication_required` | 缺少所需登录态 |
| 403 | `permission_denied` | 账户存在但权限或权益不足 |
| 404 | `resource_not_found` | 内容或账户别名不存在 |
| 409 | `conflict` | 资源或状态冲突 |
| 422 | `capability_not_supported` | 目标平台不支持该操作 |
| 429 | `rate_limited` | TuneWeave 或平台限流 |
| 502 | `upstream_error` | 平台返回异常响应 |
| 503 | `platform_unavailable` | Provider 或平台暂时不可用 |
| 504 | `upstream_timeout` | 平台请求超时 |

当 `retryable=true` 时，调用方可以采用有上限的指数退避；不要无界重试登录、写操作或验证码请求。

## 分页

多数列表端点使用 `limit` 和 `offset`。响应分页信息位于 `meta.pagination`：

```json
{
  "limit": 30,
  "offset": 0,
  "total": 245,
  "next_offset": 30,
  "has_more": true,
  "extensions": {}
}
```

部分端点使用 `page`、`next_page` 或 `next_cursor`。调用方应按该端点返回的继续值请求下一页，不要自行推导平台游标。无法确定总数时 `total` 为 `null`。

## 常用实体

### Track

```json
{
  "ref": "netease:123456",
  "platform": "netease",
  "id": "123456",
  "name": "反方向的钟",
  "aliases": [],
  "artists": [
    { "ref": "netease:6452", "name": "周杰伦" }
  ],
  "album": {
    "ref": "netease:18905",
    "name": "Jay",
    "cover_url": "https://..."
  },
  "duration_ms": 258000,
  "isrc": null,
  "mv_ref": null,
  "playable": true,
  "available_qualities": ["standard", "higher", "lossless"],
  "extensions": {}
}
```

`extensions` 保存无法统一但后续请求可能需要的平台字段。客户端可以读取已知字段，但应忽略未知扩展。

### Lyrics

```json
{
  "track_ref": "netease:123456",
  "plain": "[00:00.00]...",
  "translated": null,
  "romanized": null,
  "word_synced": "[0,1000](0,300,0)逐...",
  "singing_annotations": null,
  "singing_annotations_timestamp": null,
  "format": "yrc",
  "contributors": [],
  "extensions": {}
}
```

请求 `word_synced=true`（别名 `qrc`）可请求逐字歌词；`translated=true`（别名 `trans`）和 `romanized=true`（别名 `roma`）分别请求独立译文、罗马音轨。它们是尽力获取偏好，不承诺每个平台都支持或每首歌都有对应轨道，也不是响应过滤器：上游若在同一响应中附带未请求的轨道，Provider 仍可能返回它。上游没有提供的轨道保持 `null`；Provider 对不支持的选项可以拒绝请求。是否支持及平台特有行为以各平台说明为准，不因其他平台实现某个选项而要求所有平台齐平。

`word_synced` 存在时客户端应优先显示它，再回退到 `plain`；`translated` 和 `romanized` 是彼此独立的轨道，不会由 TuneWeave 自动生成或相互替代。`singing_annotations`（别名 `singingAnnotations`、`annotations`）和 `song_type` 也是平台特有选项，使用前应查对应平台说明。

### MediaStream

```json
{
  "url": "https://...",
  "backup_urls": [],
  "headers": {},
  "expires_at": null,
  "format": "flac",
  "codec": "flac",
  "bitrate": 999000,
  "size": 32100000,
  "duration_ms": 258000,
  "requested_quality": "lossless",
  "actual_quality": "lossless",
  "trial": null,
  "origin_track": "netease:123456",
  "resolved_track": "qq:0039MnYb0qxYhV",
  "resolved_platform": "qq",
  "match_score": 0.97,
  "attempts": []
}
```

媒体 URL 可能短期有效。客户端应遵守 `expires_at`，并在下载或播放请求中带上 `headers`。302 跳转无法携带这些请求头；需要请求头时使用 JSON 端点。

## 搜索与目录

酷狗统一搜索支持 `type=track`、`type=album`、`type=artist`、`type=playlist` 和 `type=mv`，例如 `GET /v1/search?platform=kugou&type=album&q=周杰伦&offset=17&limit=25`。新增目录搜索按上游每页 20 条读取，`limit` 为 1–100，一次最多读取 6 个搜索页；分页 `total` 是上游报告的可搜索结果数。末页、越界空页、重复结果及跨页总数变化会分别处理，不以缺失字段或上游失败返回空成功。

酷狗 MV 搜索保留官方结果顺序，也可能包含用户上传内容；不能将全部结果视为官方发行的 MV。每个非空搜索页再读取一次批量视频详情，补全官方封面与精确视频时长；最多 6 个搜索请求及 6 个详情请求。原搜索秒数保留在 `extensions.search_duration_seconds`，关联歌曲的 `audio_id`、`album_audio_id` 及不同来源的哈希独立保留。搜索页超过官方可检索范围时，上游错误码 149 映射为 `invalid_request`，不会伪造 `total=0` 的成功结果。

专辑保留每位歌手的独立引用；歌单引用使用全局 `gid`，旧数字 ID 保留在 `extensions.special_id`，创建者的用户 ID 保留在 `extensions.owner_id`。歌手的 `video_count` 不替代 `mv_count`，未知数量与收藏状态保持未知。当前仅支持匿名、默认 `variant`，拒绝账户别名、调用方凭证及尚未支持的筛选选项；账号搜索和更多媒体目录另行接入。

汽水统一搜索支持 `type=track`、`type=album`、`type=playlist` 和 `type=artist`，例如 `GET /v1/search?platform=soda&type=album&q=周杰伦&offset=17&limit=25`。专辑、歌单和歌手保留各自的结果类型、资源引用与元数据；`limit` 为 1–100，统一分页可跨越上游每页 20 条的边界。未指定账户时使用原有匿名 Android 入口，不读取默认登录凭据；显式 `account=personal`、`account=default` 或调用方凭据使用官方 PC 账户搜索。四种类型均只接受默认 `variant`，不接受外部 `search_id`、高亮、选择器或视频筛选。缺失的曲目总数保持未知，响应中的分页 `total` 不从单页数量推测。

账户搜索先独立核验 UID，再在同一次登录与搜索 ID 下读取所需分页；后续请求使用已接受的 Cookie 更新。上游游标步长可能大于实际条目数，因此从游标 0 开始，跟随后续游标并累计有效条目后再应用统一 offset；继续页的游标必须推进，稀疏页及有推进游标的空页可继续。每页最多 20 项，单次最多 128 页、每页 1 MiB、累计 16 MiB，账户核验及分页总期限 45 秒；offset 加 limit 不得超过 2560，超出预算不返回部分结果。退出、重新登录、换号、错误后页、坏条目、凭据反射、空响应或异常状态均不返回部分结果，失败也不回退匿名。成功时调用方获得最终更新凭据；失败、超时或取消清除待发调用方更新。响应和分页元数据标记 `backend=official_pc_account_search`、`authenticated=true`、`source_user_id`，相关 HTTP 响应为 `no-store`。这些字段说明请求来源，不保证搜索个性化或歌曲可播；播放仍需要独立授权。公开实测已覆盖 PC 专辑、歌手与歌单分页，歌曲 PC 入口匿名返回空响应，真实账户下四种搜索及权益仍待最终验收。

咪咕统一搜索支持 `type=track`、`type=album`、`type=playlist` 和 `type=artist`，例如 `GET /v1/search?platform=migu&type=album&q=周杰伦&offset=17&limit=25`。目录搜索的 `limit` 为 1–100，按每个上游页 20 条跨页读取，一次最多 6 页；结果保留原顺序和位置，分页 `total` 保持未知。缺失的曲目、专辑或 MV 数量不会补成 0。歌单的创建者名称保留在 `creator`，实际用户身份如有提供则在 `extensions.owner_id`，不会伪装成歌手引用；公开搜索也不推断当前账户是否收藏。仅支持默认 `variant`，拒绝未支持的筛选选项；歌曲搜索可在显式账户核验后读取公共候选，其他目录搜索仍拒绝账户别名和调用方凭证。歌手详情与完整歌曲／专辑目录使用下述独立入口。咪咕现已接入普通密码与短信登录、凭证导入、账户资料和会话管理，见[登录与凭证](authentication.md)；已接入图形验证、密码二次短信、会员资料、自建／收藏歌单目录、账户歌单及喜欢内容、喜欢／取消喜欢、歌单收藏／取消收藏、本人普通歌单创建／改名／删除、标签顺序调整与曲目增删，以及账户歌单和喜欢集合的 Uni 导入；账户歌曲播放与可用性见下文；独立账户下载已接入普通非加密歌曲的授权候选链，支持范围与真实兼容性待验收的边界见[账户媒体说明](authentication.md#咪咕账户歌曲与播放授权)；其他个人库及剩余验证分支仍在开发。

咪咕相似歌手使用 `GET /v1/artists/migu:{singerId}/similar?limit=20`，只读取匿名公开目录，不接受账户或调用方凭据。先核对歌手身份及该资源的相似歌手模块策略，再完整读取和验证上游列表后应用 1–100 的本地 limit；上游顺序保留，返回的 `artist_ref` 对应被查询歌手。若官方策略关闭该模块或任一身份、响应、分页数据无效，则直接返回错误，不输出部分结果，也不切换到其他后端。

咪咕专辑搜索同时返回普通专辑 `type=album` 与数字专辑 `type=digital_album`，SDK 对应 `SearchItem::Album` 和新增的 `SearchItem::DigitalAlbum`。两类项目共同占据原始搜索位置，不能先过滤一类再用其数量计算下一页。普通专辑引用使用 `albumId`，数字专辑引用使用 `contentId`；数字专辑的 `extensions.item_id` 是销售条目 ID，不能替代详情请求中的引用。

| 方法 | 端点 | 咪咕目录语义 |
| --- | --- | --- |
| `GET` | `/v1/albums/migu:{albumId}` | 普通专辑详情 |
| `GET` | `/v1/albums/migu:{albumId}/tracks` | 普通专辑完整读取后分页 |
| `GET` | `/v1/digital-albums/migu:{contentId}` | 数字专辑详情 |
| `GET` | `/v1/digital-albums/migu:{contentId}/tracks` | 数字专辑曲目，新增入口及 `digital_album_tracks` 能力 |

这四条咪咕入口使用公开来源，拒绝账户别名和调用方凭证。曲目请求的 `limit` 为 1–100；读取前核对详情身份，再核对全部曲目与总数，最后应用 `offset`。普通专辑按上游页号读到明确结束，最多 64 页，每页最多 1,000 项，集合最多 10,000 项；数字专辑要求单次响应返回完整列表，因此最多 1,000 项。即使只取一个窗口也可能需要多次请求。响应分页扩展带 `complete_snapshot`、`collection_type`、`collection_id`；完整性矛盾、重复页或超出请求预算返回错误，不返回残缺列表。顺序和重复歌曲保留；数字专辑中每首歌原有的普通专辑关联也保留，不改写成数字专辑 ID。缺失数量和未证明的价格、已购买或可购买状态保持未知，目录读取不代表播放权益验证。

| 方法 | 端点 | 用途 |
| --- | --- | --- |
| `GET` | `/v1/search` | 统一搜索；常用参数 `q`、`type`、`platform`、分页 |
| `GET` | `/v1/search/general` | 平台综合搜索结果 |
| `GET` | `/v1/search/default` | 默认搜索词 |
| `GET` | `/v1/search/trending` | 热搜目录 |
| `GET` | `/v1/search/suggestions` | 搜索建议 |
| `GET` | `/v1/search/multimatch` | 高置信多类型匹配 |
| `GET/POST` | `/v1/search/match` | 通过标签、时长和 MD5 匹配本地歌曲 |
| `GET` | `/v1/charts` | 音乐榜单目录 |
| `GET` | `/v1/recommendations/*` | 推荐内容 |
| `GET` | `/v1/radio/*` | 广播目录、详情与播放队列 |
| `GET` | `/v1/podcasts/*` | 播客目录、详情与节目 |

示例：

```http
GET /v1/search?q=海阔天空&type=track&platform=all&limit=20&offset=0
```

搜索 `data` 使用 `type` 判别资源种类。多平台结果保留各自资源引用，不会把不同平台 ID 合并为同一个 ID。

酷狗 `GET /v1/search/default?platform=kugou` 返回官方桌面 Web 首页当前轮播的第一条 `main_title`，作为可直接搜索的歌曲关键词；副标题只放在扩展字段，不把广告链接或封面提升成资源。该请求固定使用匿名 Web 入口，不读取或刷新默认账户凭据。空轮播返回 `resource_not_found`，不会伪造备用词。

酷我搜索建议使用 `GET /v1/search/suggestions?platform=kuwo&client=web&q=周杰伦`；热搜使用 `GET /v1/search/trending?platform=kuwo&detail=full`，也接受 `detail=brief`。两者均使用匿名 Web 来源，拒绝显式账户和调用方凭据，不读取默认账户。建议只支持 `client=web`，查询去除首尾空白后限 1–128 个 UTF-16 单元且不得含控制字符。关键词保留上游顺序和重复项；建议中的资源类型、完整资源和展示字段保持未知。热搜两种视图均只返回关键词和原始位置排名，`metadata_scope=keywords_only`，不补造热度、图标或描述。每次读取限 45 秒、256 KiB、100 项；明确空数组可返回空结果，业务错误、缺失数据或损坏条目返回错误。

咪咕搜索热词使用 `GET /v1/search/trending?platform=migu&detail=full`，也接受 `detail=brief`。它读取官方 PC 搜索框热词接口，不接受账户或调用方凭据，不读取默认会话。热词按过滤后上游顺序编号；“猜你想搜”标题和年度听歌报告入口不作为可搜索词，重复关键词保留。`rank_scope=searchable_response_order`、`metadata_scope=keywords_only`；不从原始 `value` 推断资源身份、热度或跳转地址。响应限 256 KiB/200 项，单次总时限 45 秒；缺少数组、业务错误或损坏尾项均返回错误，不输出部分列表。

汽水搜索建议使用 `GET /v1/search/suggestions?platform=soda&client=pc&q=周杰伦`，或显式选择 `client=mobile`。汽水当前仅支持这两个正式客户端，`client=web` 返回不支持能力错误。两者按官方顺序返回关键词和已知资源类型；仅含 ID 的提示不补造成完整资源，`resource` 保持空。省略账户和调用方凭证时使用匿名来源，不读取已存储的默认会话。Mobile 使用 Android 建议接口，匿名响应带 `backend=official_android_sug`、`authenticated=false`。

PC 与 Mobile 均支持显式账户或调用方凭证：先核验所选 UID，再使用该账户请求，失败不回退匿名。账户响应分别带 `backend=official_pc_account_sug` 或 `official_android_account_sug`、`authenticated=true` 和 `source_user_id`，表示已核验的请求来源，不保证建议个性化。两种客户端的查询均限 1–1024 字节、无控制字符；响应最多 128 KiB、64 项，匿名单次请求限 20 秒，账户核验与读取共限 45 秒。空结果须有有效上游状态，业务错误、异常格式或超限不会变成空列表。Web 建议仍未接入；没有官方 Web producer 证据，不复用 PC 接口。账户完整搜索见前述统一搜索入口，真实账户验收待完成。

酷我 FM 使用 `GET /v1/radio/taxonomy?platform=kuwo` 读取分类和地区，`GET /v1/radio/stations?platform=kuwo&region_id=7&limit=20&offset=0` 读取地区电台，也可改用 `category_id`；两种筛选不能同时指定。省略筛选读取热门电台。`limit` 为 1–100，不支持游标；先校验单次上游完整数组，再按 `offset` 分页，`total` 只表示该次响应内的电台数。分类中的收藏、主播、历史、地区和热门等导航项目独立放在 `extensions.native_navigation`，不当作内容分类。

FM 详情使用 `GET /v1/radio/stations/kuwo:fm:{channel_id}`。引用与普通歌曲 ID 独立，支持通过 Uni 的 `kind=radio_station` 添加和播放解析。返回的 `stream_url` 只表示核对过电台身份的官方直播地址元数据，`extensions.stream_validation=official_url_metadata_only`；不代表已实际播放直播。入口均为匿名来源，拒绝显式账户和调用方凭证，不读取默认账户；单次操作限 45 秒、每响应 2 MiB。节目时刻表不作为有独立媒体的播放队列。响应保留官方停止维护公告及观察日期，遗留目录可读取不等于直播服务持续可用。

酷我主播专辑详情使用 `GET /v1/podcasts/kuwo:anchor:{album_id}`，SDK 为 `KuwoClient::podcast("anchor:{album_id}", None)`。读取原生主播专辑的名称、简介、封面、作者、节目数、收藏数和播放数，严格核对专辑 ID、主播标记及原生内容类型。作者引用来自平台的艺人 ID，不是登录账户 UID。入口为匿名元数据，拒绝显式账户和调用方凭证；单次操作限 30 秒、响应限 2 MiB。`paid`、`purchased`、`subscribed` 保持未知，原生付费策略仅作为扩展字段返回。不将官网推荐卡片的单曲 `rid` 当作专辑 ID，也不将第三方小程序内容映射为普通主播专辑。

主播分类使用 `GET /v1/podcasts/categories?platform=kuwo&kind=all`，读取官方分类页列出的全部分类项，扩展字段保留 `native_type` 和经校验的官方导航链接。Kuwo 不提供已证实的 `non_hot` 分类请求，因此 `kind=non_hot` 返回不支持。分类热门节目集使用 `GET /v1/podcasts/category-recommendations?platform=kuwo`：对官方 `cat` 分类按原生热度排序读取第一页，每类至多 10 个主播专辑；每个分组的 `total` 给出上游总量，`complete=false` 表示仍有后续页。按类别读取热度或最近更新顺序使用 `GET /v1/podcasts?platform=kuwo&catalog=category_hot&category_id=31&limit=10&offset=0` 或 `catalog=category_newest`；后者对应官方“最近更新”排序，不代表节目发布日期排序。`category_id` 必须是官方 `cat` 分类，`offset` 必须是 `limit` 的整数倍；若上游成功返回空页但 `has_more=true`，应继续请求 `next_offset`，不要因当前页为空而停止。分页扩展保留每页实际返回数。`link` 与 `xshow_index` 导航项不伪装成播客分类节目。三个入口只读匿名数据，拒绝显式账户和调用方凭据，每个上游响应限 2 MiB、单请求限 10 秒；任一上游页损坏时不返回部分分组。

节目列表使用 `GET /v1/podcasts/kuwo:anchor:{album_id}/episodes?limit=30&offset=0&ascending=true`，SDK 为 `KuwoClient::podcast_episodes`。支持 1–100 条分页；`ascending=true` 正序，默认倒序。节目引用为 `kuwo:episode:{music_id}`，保留所属专辑、期号、时长、作者及发布日期；严格校验上游页码、顺序和每条节目的专辑归属。越过末页返回空页，不重复上游回送的最后一期。此入口同样仅支持匿名来源，单次限 30 秒，压缩响应和展开内容各限 4 MiB。列表中的 `audio` 及账户权益保持未知；百城声音子目录见下方艺人目录入口。

单期详情使用 `GET /v1/episodes/kuwo:episode:{music_id}`，核实主播标记、原生内容类型及所属专辑后，在 `audio.ref` 返回普通歌曲引用 `kuwo:{music_id}`。专辑简介不会冒充本期简介，音频引用本身也不代表已获播放授权。`GET /v1/episodes/kuwo:episode:{music_id}/stream` 以及 Uni 的 `kind=podcast_episode` 导入、播放解析复用公共媒体流程，每次播放重新向平台请求授权，不在 Uni 快照中存储临时音频 URL。SDK 可调用 `KuwoClient::podcast_episode` 和 `podcast_episode_stream`；详情总预算 45 秒，SDK 详情与播放授权合计 60 秒。当前仅匿名普通 MP3 授权，显式账户及特殊音效不在该入口支持范围；真实账户和实际音频播放仍待最终验收。

咪咕榜单目录使用 `GET /v1/charts?platform=migu&view=summary`；三个 `view` 均保留官网完整分组，包括国风热歌榜等返回条目。引用形如 `migu:chart:27553319`。预览分别保留名次、歌曲引用及已提供的排名变化，不推算历史排名；目录响应版本 `source_data_version` 不当作榜单更新时间或不可变快照，也不从预览数量推断歌曲总数。

`GET /v1/charts/migu:chart:27553319/tracks?offset=18&limit=4` 读取咪咕当前期榜单，亦接受不带 `chart:` 的榜单编号。`limit` 为 1–100；单次响应最多 2 MiB、1000 条、总等待最多 45 秒。先核对完整列表、总数、续页标记、榜单及当前期编号、重复条目的身份一致性，再应用分页；超限、身份冲突或未知续页协议均不返回部分结果，未知榜单返回 `resource_not_found`。越界保留已核实总数。 数量矛盾返回 `upstream_error`，错误详情中的 `reason=chart_count_mismatch`、`declared_track_count` 和 `received_track_count` 分别说明原因、标称数量和实际数量，不用成功响应掩盖缺失条目。

咪咕榜单保留单次官方响应中重复出现的歌曲及各自位置，分页总数包含这些条目，另以 `upstream_unique_track_count` 返回不同曲目数；同一曲目引用绑定不同 `songId/copyrightId` 时拒绝。咪咕榜单歌曲引用使用 `contentId`，内嵌歌曲资料的 `contentId/songId/copyrightId` 必须与榜单行一致。`chart_rank` 从 1 开始，`chart_position` 从 0 开始；`include_tags=true` 时附加已提供的 `chart_rank_change`，缺失不补零，关闭标签也不会跳过上游完整性校验。分页中的 `update_label` 仅保留更新文字，`period_scope=current`、`consistency_scope=single_complete_response` 标明读取范围。两条入口只接受匿名来源，不读取默认账户，拒绝显式账户及调用方凭证；目录音质和版权标记不代表账户播放权益。

咪咕历史榜单沿用歌曲入口，例如 `GET /v1/charts/migu:chart:27553319/tracks?period_kind=day&period_date=2026-01-01&limit=10`，周榜使用 `period_kind=week`。`period_date` 必须为有效公历 `YYYY-MM-DD`；未指定期间或显式 `current` 保持当前榜，当前榜不能附日期，日榜/周榜不能省略日期。不自动选择今天、上一周或调整星期；无该期数据返回 `resource_not_found`。入口拒绝未知参数、重复字段和矛盾组合，保留已有分页及标签别名。

SDK 的 `ChartTrackListRequest.period` 为 `ChartPeriod::Current`、`Day { date }`、`Week { date }` 或 `Id { id }`，旧请求反序列化时默认当前期。咪咕支持日/周日期，酷狗支持从期数目录获得的 ID；两者提供 `chart_historical_tracks`。QQ、酷我以及默认歌单委托实现尚不支持显式历史期间。未支持的期间种类返回 `capability_not_supported`，不会默默用当前榜或普通歌单代替。

咪咕分页扩展中的 `requested_period` 是调用方选择，`period_column_id` 是上游返回的独立期间编号，`period_binding_scope=request_and_returned_column` 明确这个边界。歌曲扩展同时保留 `chart_requested_period` 与 `chart_period_id`。`update_label` 仍是上游显示文字，可能为当前日期，不充当历史日期回显。返回当前期间来替代历史期间时拒绝；完整数量、重复条目保留、身份、账户隔离与大小/时间预算保持一致。存在上游支持列表时返回 `available_period_kinds` 和 `upstream_rank_types`，缺失则保持未知；`period_start_dates` 是官网选择器使用的起始日期，不保证其后每个日期都有可用榜单，也不定义所有榜单的统一周起点。

酷狗榜单目录使用 `GET /v1/charts?platform=kugou&view=summary`。`overview`、`summary`、`modern` 均读取同一份官方完整目录，并保留请求的 `view`。榜单引用形如 `kugou:chart:8888`；名称、简介、封面、更新说明、可用歌曲数和预览分别返回。目录中的界面播放按钮与跳转链接不用于判断播放权益；缺少数量时保持未知，响应生成时间也不会当作榜单更新时间。

酷我榜单目录使用 `GET /v1/charts?platform=kuwo&view=summary`，三种 `view` 均映射当前官网完整分组目录。引用形如 `kuwo:chart:16`，其中 `16` 是官方 `sourceid`；另一展示编号仅保留为 `extensions.display_id`。目录更新标签保留为 `publication_label`，不推算年份或更新时间戳；没有歌曲数、收藏状态或播放授权字段时保持未知。

`GET /v1/charts/kuwo:chart:16/tracks?offset=18&limit=4` 读取酷我当前榜单歌曲，`limit` 为 1–100，也接受 `kuwo:16`。先读取首页确定总数与发布日期，再读取目标窗口；每页 20 条、最多 7 个目录页、每个响应最多 2 MiB、整体最多 45 秒，包含匿名会话初始化和受控刷新。检查完整物理页、总数、发布日期及重复歌曲；中途失败不返回部分列表。越界请求保留首页总数，不请求无必要的越界页。日期为 `publication_date`，不是历史期号或不可变快照；一致性范围由 `consistency_scope=matching_count_and_publication_date` 标明。

酷我榜单歌曲保留上游位置，`chart_rank` 为从 1 开始的名次，`chart_id` 与 `chart_publication_date` 保留来源。`include_tags=true` 时输出已提供的 `chart_upstream_trend`、`chart_upstream_rank_change`、`chart_upstream_is_new` 原始字段；缺失的新上榜标记不补零，也不据此推算历史排名。`include_tags=false` 省略这些附加字段，仍保留来源和名次。目录中的音质规格与付费标记不表示当前账户具备播放权益。两条酷我入口均为公开目录，拒绝显式账户与调用方凭证，不读取默认账户仓库。

`GET /v1/charts/kugou:chart:8888/tracks?offset=98&limit=5` 先从详情取得实际 `rank_cid`，再固定该期号读取全部歌曲，最后应用统一分页。每个上游页最多 100 首，每次最多 128 页；`limit` 为 1–100。每页核对总数、榜单 ID、期号及实际名次，重复歌曲、跨期内容、矛盾排名、中途失败或预算耗尽均返回错误，不返回残缺列表。目录期号为 0 的榜单也会先查详情，不能据此判为没有歌曲。两条入口都只接受匿名来源，拒绝账户别名或调用方凭证。

榜单歌曲扩展中的 `chart_rank` 为实际名次，`chart_position` 为从 0 开始的位置；`chart_id` 与 `rank_cid` 分别标识榜单和期号。`last_sort`、`last_original_index` 等原始排名字段保留各自含义，不据其中一个 0 值推断“新上榜”或计算涨跌。`include_tags=false` 省略附加榜单标签、曲目备注与推荐理由，仍保留身份、名次和音频目录信息。分页扩展说明期号、完整读取状态及上游页数；固定期号和一致性检查不代表上游承诺不可变快照。

## 资源详情

汽水专辑详情 `/v1/albums/soda:{id}` 与 `/tracks` 支持公开来源、显式账户别名或调用方凭证。账户来源先核验 UID 并完整读取官方 PC 专辑，再应用 `offset` / `limit`（1–100）；保留顺序、重复项及从 0 开始的 `extensions.album_position`。没有显式账户时使用公开分享入口；账户错误不会改为匿名请求。账户读取最多 8 MiB、10,000 首，含身份核验共限 45 秒。

账户专辑的扩展字段 `backend=official_pc_account_album`、`source_user_id`、`complete_read` 和 `source_snapshot_id` 分别说明来源、查询 UID、完整读取状态及绑定登录与目录的不透明版本。`type=album` 可用于 Uni 歌单导入与物化；导入期间版本不一致会失败，不保留部分结果。完整读取不代表上游事务快照，专辑目录也不代表已购买或可播放。真实账户验收待完成。

酷狗专辑详情 `/v1/albums/kugou:{album_id}` 返回官方专辑名称、歌手、简介、封面、发行日期、公司与类型；缺少曲目数时保持未知。`/v1/albums/kugou:{album_id}/tracks` 先核对专辑身份，按上游每页 20 条读取完整目录，再应用 `offset` 和 `limit`（1–100）。每次最多读取 64 页、1,280 首，超过预算或出现跨页总数、碟数、曲目身份或位置冲突会返回错误。两条入口均使用公开匿名来源，不接受账户别名或调用方凭证。

酷狗专辑曲目使用 `album_audio_id` 作为歌曲引用，`audio_id` 和音频哈希分别保留。上游顺序、不同位置的重复曲目、碟号和曲序保持原样；`extensions.album_position` 是从 0 开始的目录位置。分页 `total` 为完整目录数量，分页扩展包含 `complete_snapshot`、`upstream_pages_fetched` 和可用的 `disc_count`；这表示本次已完整读取，不承诺上游事务快照。曲目中的音质、大小和时长是目录资源信息，不能据此判断账户播放或下载权益。

酷狗歌手详情 `/v1/artists/kugou:{author_id}` 返回简介、完整分节传记、可用头像及歌曲/专辑/MV 数量，未知字段保持未知。`/tracks` 支持 `order=hot` 和 `order=time`；`order=platform_default` 使用上游默认的最新顺序。`/albums` 返回按最新顺序排列的专辑；`/overview` 返回歌手资料及热门目录前 10 首，并标明是否还有歌曲。这些入口均使用公开匿名来源，拒绝账户别名和调用方凭证。

`GET /v1/artists/catalog?platform=kugou` 返回官方热门及字母分组歌手目录，保留热门列表与字母目录的各自顺序；同一歌手可同时出现在两个视图。目录支持 `area=all|chinese|western|japanese|korean|other` 和 `type=all|male|female|group`，`genre` 仅接受 `all`；香港／台湾、未知筛选、账户别名和调用方凭证均不支持。歌手 ID 可直接用于上方详情和作品入口；目录仅表示上游分组范围，不宣称全平台完整歌手全集。上游缺失的数量保持未知。

酷狗歌手歌曲页宽为 100，专辑页宽为 30，每类最多遍历 128 页；先核对歌手身份与可用计数，完整读取后再应用 `offset` 和 `limit`（1–100）。分页扩展包含 `artist_id`、`kind`、实际 `order`、`complete_snapshot` 与 `upstream_pages_fetched`。排序回显、总数或作者身份矛盾、重复歌曲/专辑 ID、读取失败及超出预算均返回错误。歌曲保留每位合唱者，`album_audio_id` 相异而 `audio_group_id` 相同的版本独立保留；没有专辑 ID 时不会构造专辑引用。概览也会完整读取热门目录，调用可能需要多次上游请求；不承诺上游事务快照。音频容器和音质字段仍只描述目录资源。

`GET /v1/artists/kugou:{author_id}/top-tracks` 返回已完整校验的热门曲目目录前 10 首，沿用歌手概览的热门顺序；它是 TuneWeave 对该目录的固定投影，不代表酷狗另有独立 Top 10 榜。该入口只接受公开匿名读取，不接受账户或调用方凭证。分页 `total` 是本次返回数，`has_more=false`；扩展字段 `result_scope=hot_catalogue_first_10` 和 `source_catalogue_total` 分别标记投影范围及原目录总数。完整目录校验失败时不返回部分结果。

酷狗歌手视频使用 `GET /v1/artists/kugou:3520/videos?kind=all&offset=29&limit=5`。`all` 和 `mv` 均读取官方歌手页面所展示的同一份 MV 目录，包含官方、现场、饭制和用户上传内容；引用统一使用 `kugou:mv:{video_id}`，分页扩展保留请求的 `kind`。支持匿名 `offset` 分页，`limit` 为 1–100，使用平台默认顺序；拒绝账户别名、调用方凭证、游标和显式排序。

视频目录按官方每页 30 条读取，先核对歌手身份与可用总数，再完整读取请求窗口；每次最多 5 个目录页，并仅为窗口内的项目读取详情（每批最多 20 个不同视频、最多 5 批），加上一次歌手资料请求。分页扩展 `complete_window=true` 表示本次窗口已完整读取，后续可用 `next_offset` 继续；`upstream_pages_fetched` 和 `detail_batches_fetched` 分别记录两类请求数。总数变化、重复 ID、详情缺失或身份/时长冲突会返回错误。真实歌手与上传者来自视频详情，`catalogue_artist_id` 仅表示查询的目录来源，`artist_video_position` 是从 0 开始的目录位置。

汽水歌手详情 `/v1/artists/soda:{id}` 和概览 `/v1/artists/soda:{id}/overview` 在未指定账户时使用官方公开分享页，返回可用的简介、别名、头像和歌曲数。提供 `account=default`、`account=personal` 或 Soda 调用方凭据时，入口改用官方 PC 账户歌手详情接口；详情返回账户请求来源标记后的歌手资料，概览另外返回 `hot_tracks` 预览。歌手 ID 与关联用户 ID 分开处理，关联用户只保留经过校验的 `extensions.linked_user_id`，不会输出上游用户对象中的其他字段。

公开概览中的 `featured_tracks` 是分享页提供的预览；账户概览中的同名字段是 PC `hot_tracks` 预览。`has_more_tracks=true` 表示预览尚未覆盖资料中的歌曲数，不能将概览当作全部作品；已知歌曲数时服务会校验该标记与预览数量一致，缺少歌曲数则保留未知语义。账户返回的 `extensions.source_user_id`、`authenticated`、`backend` 和显式 `account_state` 只描述请求来源及上游明确提供的状态，不代表个性化、收藏、播放或下载权益；缺少的状态字段不会补成 `false`。账户详情的预览最多 100 首，歌手资料中的专辑数保持有界，错误、认证失效或超时不会回退公开入口。账户请求响应使用 `no-store`，调用方凭据成功轮换通过 `x-tuneweave-updated-credential` 返回。

汽水完整歌手作品使用 `/v1/artists/soda:{id}/tracks?order=platform_default` 和 `/v1/artists/soda:{id}/albums`，支持 `limit=1–100` 与 `offset`。歌曲必须明确选择平台默认顺序，当前不提供热门或时间排序。这两条入口先核对官方 PC 歌手身份，再完整读取作品，验证署名、重复 ID 和可用的独立计数，最后切出请求窗口；中途失败或计数矛盾不会返回部分结果。省略的数量保持未知，分页 `total` 是本次完整读取的实际数量。未指定账户时走匿名 PC 目录，不读取默认账户或初始化登录设备；显式 `account=default`、具名账户或调用方凭据则先核验本人 UID，再以所选 Cookie 读取歌手资料和所有作品页。每个成功或失败网络响应均检查登录代际，完整合法页之后才接纳 Cookie 轮换，注销或换号后的迟到结果被拒绝。账户响应中的 `source_user_id` 和 `authenticated` 仅表示请求来源，不证明个性化、收藏状态或播放下载权益。

账户歌手曲目／专辑目录共用 60 秒总期限（含本人核验）、单个目录响应 8 MiB、累计 64 MiB、最多 128 个作品页及 10000 项上限；实际游标推进与可见实体数分别处理。最终窗口之外的已读取作品也须通过身份和敏感信息校验。遇到认证失效、附加验证、计数矛盾、超限、取消或超时均不返回部分目录，不改用匿名来源；失败不交付未完成请求的调用方凭据。目录完整读取不构成上游原子快照。歌手详情与概览的公开入口及其账户限制保持上文所述。

分页扩展包含 `complete_read`、`artist_id`、`reported_total`、`upstream_pages` 和 `backend`。上游扫描游标可能大于可见作品数，且末页仍可非空；它不作为统一偏移或总数返回。每次最多读取 128 页、10,000 项，单页最多 1,000 项/8 MiB，合计响应最多 64 MiB、总时限 60 秒；不承诺上游事务快照，目录信息也不代表播放权限。

| 资源 | 常用端点 |
| --- | --- |
| 歌曲 | `/v1/tracks/{ref}`、`/lyrics`、`/availability`、`/versions`、`/similar` |
| 专辑 | `/v1/albums/{ref}`、`/tracks`、`/stats` |
| 歌手 | `/v1/artists/{ref}`、`/catalog`、`/tracks`、`/albums`、`/digital-albums`、`/videos`、`/stats` |
| 用户 | `/v1/users/{ref}`、`/playlists/created`、`/favorites/*`、`/history` |
| 歌单 | `/v1/playlists`、`/v1/playlists/{ref}`、`/items`、`/tracks` |
| 视频 | `/v1/videos/{ref}`、`/parts`、`/subtitles`、`/playback`、`/stats` |
| 播客节目 | `/v1/episodes/{ref}`、`/lyrics` |

酷狗视频详情使用 `GET /v1/videos/kugou:mv:17737761` 或 `GET /v1/videos/kugou:video:17781851`；两种资源类别访问同一官方视频 ID 空间，前缀与显式 `kind` 必须一致，不据此断言内容是官方 MV。批量入口 `GET/POST /v1/videos/details` 接受 1–100 个引用，按每批最多 20 个不同 ID 请求，保留原请求顺序和重复项。缺失、下线、重复或混入其他 ID 的上游响应不会交付部分成功；资源不存在返回 `resource_not_found`。

视频时长与关联音频时长分开，歌手与上传者分别保留；上传者信息位于 `extensions.uploader`。详情的 `resolutions` 仅根据明确的像素高度生成，缺少尺寸的资源仍保留在 `video.extensions.catalogue_assets`；同高度的不同来源资源不合并。哈希、码率、大小和尺寸是目录信息，不代表已经取得播放或下载权益，也不根据字段名推断实际容器或编码。视频搜索仍仅支持匿名来源，拒绝账户别名、调用方凭证和未支持的筛选；详情另支持下述原生账户会话核验。

酷狗匿名视频播放使用 `GET /v1/videos/kugou:mv:17737761/stream?resolution=1080`，另支持现有 `/stream/redirect` 和 `GET/POST /v1/videos/streams`。批量接受 1–100 个引用，保留顺序、重复项及原始前缀，同一视频只解析一次媒体地址。先读取详情，再核对各资源权限的实际视频 ID、哈希、文件大小及可用码率；获准后才读取媒体地址。优先选择不超过请求高度的最高获准资源，否则选最小的更高资源；已知尺寸优先，全部尺寸未知时保留 `actual_resolution=null`。同高度优先普通资源字段，其次 MKV、其他资源。

仅返回上游直接提供的受信 HTTPS 媒体地址，主、备用地址保持顺序并去重。权限明确拒绝、无目录资源或未取得获准地址时，`available=false` 且不返回播放 URL；资源身份、大小冲突、异常响应和中途失败返回错误，不交付部分批次。`format`、`codec`、`expires_at` 未获明确证据时保持未知，权限数码保留在扩展中，不推断免费或会员权益。

酷狗原生标准版／概念版账户也支持上述视频详情、播放、批量及重定向入口：服务端使用 `account=A`，调用方使用现有凭证请求头并省略账户别名。详情在核验选中会话后读取公开目录；播放则使用该账户的设备、令牌及本次个人资料返回的原始 `vip_type` 请求资源权限和媒体地址，缺失该会员字段时返回 `capability_not_supported`，不会默认补零或根据已购列表推断。会员字段只作为上游请求参数，最终仍以资源权限和地址响应为准。账户资料的 `extensions.vip_type` 保留该原始数码，不代表完整会员包模型。

账户视频批次只在当前请求和当前会话内复用结果；注销、重新登录或并发凭证替换会使迟到响应失效。Uni 的持久条目和临时材料可用独立的播放账户请求原生视频；跨平台回退时，原平台的登录失效或会话冲突不会带回失效凭证更新。Web Cookie 视频、加密媒体及视频独立下载授权仍待支持，真实账户播放验收仍待进行。

咪咕歌手目录使用以下公开入口，不接受服务器账户别名或调用方凭据：

| 方法 | 端点 | 语义 |
| --- | --- | --- |
| GET | `/v1/artists/migu:{singerId}` | 本人歌手资料、可用图片和完整简介 |
| GET | `/v1/artists/migu:{singerId}/overview` | 资料与平台默认顺序的前 10 首预览，明确是否还有歌曲 |
| GET | `/v1/artists/migu:{singerId}/tracks?order=platform_default` | 完整歌曲目录上的统一分页 |
| GET | `/v1/artists/migu:{singerId}/albums` | 普通专辑目录 |
| GET | `/v1/artists/migu:{singerId}/digital-albums` | 数字专辑目录，SDK 对应 `artist_digital_albums` |

歌手分类目录使用 `GET /v1/artists/catalog?platform=migu&area=chinese&type=male&genre=all`。咪咕支持 `area=chinese|western|japanese_korean` 与 `type=male|female|group`；“日韩”是上游明确提供的合并分类，使用 `area=japanese_korean`，不等同于单独日本或韩国分类。缺省的 `all`、港澳台、其他分类及非 `all` 的 `genre` 均在请求前返回 `capability_not_supported`。接口先读取官方分类标签，再读取所选标签的完整目录；热门歌手放在 `featured_artists`，字母目录放在 `artists`，两组各自保留上游顺序。相同歌手可同时出现在两组，目录不承诺跨两组的原子快照；粉丝数仅作为每条来源行的资料，不能用来判断身份或合并重复项。目录里的筛选项说明仅涵盖已核实的区域与男／女／组合子集，未核实的区域、类型或曲风不由通用模型补齐。

咪咕歌曲目录只接受显式 `order=platform_default`，表示官网默认顺序；不承诺热门或时间排序。通用 API 省略 `order` 时仍是既有的 `hot`，咪咕会拒绝尚未支持的排序；网易云与 QQ 的既有排序行为保持不变。SDK 使用 `ArtistTrackOrder::PlatformDefault`。概览的前 10 首也使用平台默认顺序，不是另外认证的热门榜。

歌曲每个上游页最多 50 项，混合专辑页最多 10 项，每种目录最多遍历 128 页。每次先核对歌手身份及可用计数，完整读取后再应用 `limit`（1–100）和 `offset`；即使请求第一页或越界窗口，也会进行完整遍历。重复资源、计数或下一页身份矛盾、未知条目类型、读取失败及预算耗尽都返回错误。内容在读取期间变化可能导致失败，不声称上游提供事务快照。

专辑原始目录包含普通与数字专辑，两条接口都先读取全部混合页再按类型筛选，分别返回 `Album` 与 `DigitalAlbum`。普通 ID 是 `albumId`，数字 ID 是 `contentId`，同样的数字字符串可属于不同类型。分页 `total` 为所选类型的数量，`extensions.upstream_raw_count` 是原始混合数量；歌手 `album_count` 若有值包含两类，`extensions.album_count_includes_digital=true` 说明此含义。专辑歌手显示名没有可靠 ID 时不补造引用；未知价格、购买状态和曲目数保持未知。资料中的视频页计数保留在 `extensions.tab_counts.mv`，不据此推断全部资源都是 MV。

`GET/POST /v1/artists/details` 批量读取歌手描述。默认返回扩展资料、百科、组合成员、主图和相册；调用方可分别通过 `ex_singer`、`wiki_singer`、`group_singer`、`pic`、`photos` 控制，POST 也接受对应的 `include_*` 别名。QQ 主图和相册使用强类型字段，所有图片地址在返回前校验并统一为 HTTPS。

`POST /v1/account/podcasts/{ref}/episodes` 兼容原始音频 body（通过 `cover_image_id` 指定已有封面）和 `multipart/form-data`。multipart 请求使用 `audio`/`songFile` 与可选 `cover`/`imgFile` 文件字段；封面文件和 `cover_image_id` 不能同时提供。音频最大 500 MiB，封面最大 20 MiB，文件名、媒体类型和账户范围会在网络请求前校验。

批量详情端点通常同时提供 GET 和 POST 形式。GET 适合短引用列表；POST 适合结构化批量请求。

## 播放与下载

歌曲播放：

```http
GET /v1/tracks/{ref}/stream?quality=lossless&playback_platform=qq&fallback=true&fallback_platforms=netease,kugou,migu,kuwo,soda
```

常用媒体端点：

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `GET` | `/v1/tracks/{ref}/stream` | 返回统一 `MediaStream` |
| `GET` | `/v1/tracks/{ref}/stream/redirect` | 302 到最终媒体 URL |
| `GET` | `/v1/tracks/{ref}/stream/content` | 由服务端交付支持的媒体内容 |
| `GET` | `/v1/tracks/{ref}/download` | 返回下载元数据 |
| `GET` | `/v1/tracks/{ref}/download/redirect` | 302 到下载 URL |
| `GET` | `/v1/tracks/{ref}/download/content` | 独立校验下载权限后交付媒体附件 |
| `GET/POST` | `/v1/tracks/streams` | 批量解析歌曲流 |
| `GET/POST` | `/v1/videos/streams` | 批量解析视频流 |
| `GET` | `/v1/videos/{ref}/audio-stream` | 选择视频的音频轨 |
| `GET` | `/v1/videos/{ref}/video-stream` | 选择视频轨 |
| `GET` | `/v1/videos/{ref}/playback` | 返回完整播放清单 |

常用 `quality` 值包括 `auto`、`standard`、`higher`、`high`、`lossless`、`hires`、`surround`、`dtsx`、`spatial`、`dolby`、`master` 和 `vinyl`。`vinyl` 与 `dtsx` 当前仅酷我支持，其他平台明确拒绝；`dtsx` 表示酷我独立 DTS:X 档，不等同于 `surround` 或 `spatial`。响应中的 `actual_quality` 是最终取得的档位。

酷我账户伴唱使用 `variant=sing_along`，通过 `/stream/content` 或 `/download/content` 返回双声道 PCM WAV。伴唱保留少量引导人声，须独立核验伴唱及本次播放／下载授权；当前只支持完整歌曲。需要显式服务器账户或调用方凭据，普通 URL 端点不返回原四声道媒体地址。主资源音质选择、格式边界与固定混音规则见[酷我账户音频](authentication.md#酷我普通账户音频与下载)。其他平台尚不支持该变体时明确拒绝，不返回普通歌曲代替。

跨平台回退使用标题、歌手、专辑、时长、ISRC 和版本信息进行匹配。`attempts` 按执行顺序记录每个平台的结果。试听片段只在没有更合适完整资源时作为结果返回。

歌曲播放默认同时启用解锁阶段。对网易云音乐歌曲，服务端先尝试原始音源；原始 URL 缺失、只有试听片段或权益不足时，再按首选平台、手动回退平台和公开音源顺序匹配。默认公开音源顺序为 QQ 音乐、酷狗、酷我、咪咕、汽水。`unblock=false` 才会关闭该附加阶段；`fallback=false` 不会关闭默认解锁，这是两个独立控制项。`source` 可以把某个公开音源提升到解锁阶段首位，但不会改变歌曲的原始引用。

网易云返回的 `m数字.music.126.net` 媒体 URL 会同时提供兼容的 `m数字c.music.126.net` URL 和原始 URL，调用方应按 `url`、`backup_urls` 顺序尝试；重定向端点会发送 `Referrer-Policy: no-referrer`，避免网易 CDN 因来源页拒绝播放。服务端不会接受调用方注入的媒体 URL、代理或请求头。

歌曲下载端点接受相同的 `playback_platform`、`fallback`、`fallback_platforms`、`unblock` 和 `source` 控制项。服务端优先使用原平台提供的完整下载 URL；原生下载不可用时复用统一播放解析链，并将实际来源、匹配结果和尝试记录写入下载响应。显式指定其他 `source` 时直接解析该公开音源；仅取得试听片段时不会将其作为完整下载返回。

汽水歌曲的 `/stream`、`/download` 和 `/stream/content` 支持服务器账户别名与调用方凭证。返回本地音频 URL 时，应同时使用响应数据中的 `headers`；仅授权试听时不会提供完整下载 URL。账户请求使用官方 PC 账户链，失败不会在汽水提供者内部切换为匿名 SEO；统一跨平台回退仍由上述路由参数控制。当前支持直接播放器模型，以及直接模型缺失时的官方二级明文或 CENC 播放器授权。二级 CENC 仅接受明确的 `cenc-aes-ctr`、完整授权和合法密钥标识，内容交付时核对媒体内标识；其他算法或不完整授权明确拒绝，密钥不对外返回。真实账户验收待完成。

汽水明文内容交付会核对实际 AAC／ALAC 容器或原始 FLAC 帧结构；授权字段未声明加密也不会接受带加密标记的文件。损坏、截断或声明编解码与容器不符时返回错误，不交付部分内容；具体格式边界见[登录与凭证](authentication.md)。

`/stream/content` 和 `/download/content` 直接交付支持平台的音频字节，拒绝 `playback_platform`、`fallback`、`fallback_platforms`、`unblock` 和 `source`。下载内容调用独立的 `audio_download_content`，提供者未实现时返回不支持，不复用播放授权。酷我这两个接口支持明确选择的服务器账户和调用方凭据，可处理已核验的原生加密音频；返回实际 MIME、播放 `inline`／下载 `attachment` 和 `private, no-store`。响应头 `X-TuneWeave-Audio-Kind` 标明 `full` 或 `trial`；试听同时返回 `X-TuneWeave-Trial-Start-Ms`、`X-TuneWeave-Trial-End-Ms`，范围位于原曲时间轴。汽水和酷我的内容交付均保留这个窗口。下载内容端点独立拒绝任何带试听标记的内容；无效范围报上游错误。大小、音质及密钥边界见[登录与凭证](authentication.md#酷我普通账户音频与下载)。

播放、下载、二进制内容、重定向、批量及 Uni 解析都可通过 `X-TuneWeave-Updated-Credential` 响应头返回调用方凭证更新。先处理更新，再处理业务结果；调用方模式的重定向建议手动跟随。多平台及媒体请求头的处理规则见[登录与凭证](authentication.md)。

汽水会员查询 `/v1/account/membership?platform=soda` 和 `/v1/users/soda:{id}/membership` 支持默认账户、具名账户或调用方凭据；`front`／`client` 均读取官方 commerce v2 主会员资料。返回明确状态及 UTC `expires_at`，未知字段保持 `null`，并独立核验响应凭证的 UID；会员状态不代替单曲授权。字段与失败边界见[登录与凭证](authentication.md)。

咪咕本人资料查询 `/v1/account/profile?platform=migu&backend=modern` 与 `/v1/users/migu:{uid}?backend=modern` 只返回所选账户已核验的用户 ID、昵称和头像，未提供的等级、统计和社交字段保持 `null`。指定 UID 必须属于当前账户；服务器账户和调用方凭据均遵守同一代际、更新凭证与 `no-store` 规则，不以匿名资料替代失败的账户读取。

酷狗会员查询 `/v1/account/membership?platform=kugou` 和 `/v1/users/kugou:{id}/membership` 同样支持默认、具名及调用方账户；`front`／`client` 根据凭证选择普通版、概念版或 Web 协议。通用状态和到期字段对应主会员，其他产品分别保留在 `membership_details`；普通版 SVIP 等级另标明所属类型。日期保持上游原文且时区未知，不按最长日期推导权益。字段、凭证更新与失败边界见[酷狗会员资料](authentication.md#酷狗会员资料)。

## 歌单与写操作

酷我公开歌单精选目录使用 `GET /v1/playlists?platform=kuwo&catalog=latest|hot`；必须明确选择 `latest`（官网“最新”）或 `hot`（官网“最热”）。`limit` 默认为 20、范围 1–100，`offset` 默认为 0；接口最多读取所需的 6 个官方 20 项页面，并在 45 秒总预算、每页 2 MiB 内校验页号、页大小、总数和不重复的歌单 ID。也可通过 `GET /v1/playlists/tags?platform=kuwo` 读取官方当前标签分组，再使用其中标签的 `id` 调用 `GET /v1/playlists?platform=kuwo&catalog=tag&tag_id=...`；标签 ID 必须来自当前目录，`tag` 目录不接受 `latest`/`hot` 排序参数。以上目录只支持匿名访问，账户别名和调用方凭证会在网络请求前拒绝；未实现该目录的 Provider 保持不支持。返回的是精选目录窗口，不代表平台全部歌单，也不保证跨页或跨调用事务快照。结果引用是普通 `kuwo:{id}` 歌单，可继续使用下方歌单详情、曲目读取和 Uni `playlist` 导入；创建者歌手身份、订阅状态、发布日期和播放权益不会从歌单行推断。

酷狗原生标准版／概念版账户可用于歌曲详情、`/v1/tracks/kugou:{album_audio_id}/stream`、`/download` 及相应重定向入口；播放批量入口也沿用同一账户路径。服务端模式使用 `account=A`，调用方模式使用现有凭证请求头并省略账户别名。Web Cookie 暂不支持这条原生媒体链。歌曲详情提供目录信息；播放／下载会重新核对目录身份与音频哈希，并依次取得用户授权、单曲授权和当前媒体地址，不能用已购记录或会员标记代替实时授权。

账户媒体支持默认 `variant` 的标准、高品质、无损、Hi-Res 和母带目录选择；目录缺少目标音质时按既有音质顺序选择较低资源，响应依据实际格式和码率报告音质。`bitrate` 为 1–320000，暂不接受沉浸类型。上游对单曲或媒体明确拒绝授权时，会在同一选中账户内依次尝试较低目录音质，每种资源重新取得授权；用户级授权失败、登录失效、会话冲突、网络错误或异常媒体响应直接报错。高音质只有试听时优先尝试较低音质的完整播放；如果全部只有试听或明确拒绝，播放保留已验证的最高目录音质试听，下载仍失败。上游返回的身份、时长、格式或试听范围不一致时明确失败；不支持的加密文件不作为普通音频返回。下载必须取得独立的下载授权且覆盖完整曲目，降级过程也始终使用下载授权。需要独立下载授权的平台不会把播放链接自动转换成下载成功，跨平台解析后也会重新向实际播放平台请求下载授权。


酷狗 Web 凭证支持账户歌曲详情、播放、批量播放、播放跳转及 `availability`，沿用同一 `account`／调用方凭证方式。每次先交换并核验所选 Web 登录，再从公共目录核对曲目；官网播放请求使用该账户的最新令牌和设备签名。目录请求不携带登录令牌；媒体响应的 Cookie 不更新登录状态。重新登录、注销或并发轮换会使迟到结果失效，普通错误保留已接受的凭证更新，认证失效及冲突抑制更新。

Web 播放接口没有音质选择参数；普通音质请求按官网返回的实际码率报告 `actual_quality`，不保证所请求的高音质。试听要求明确的试听标记与起止时间，不能作为全曲可用性；部分数字专辑试听仅允许官方客户端，会返回拒绝。缺失格式或有效期保持未知。Web 登录 Cookie 必须能用于官网页面；只限定登录主机的 Cookie 无法用于官网播放。Web 播放不提供独立下载授权，下载入口返回 `capability_not_supported`，不会把播放 URL 作为下载授权；真实 Web 账户权益仍待最后验收。

酷狗原生账户支持 `GET /v1/tracks/kugou:{album_audio_id}/availability?account=A`，调用方模式使用凭证请求头。`bitrate` 接受 1–320000；默认值 `999000` 表示从最高受支持的目录音质开始验证，不代表实际媒体码率。检查沿用实时播放授权和同账户音质降级，`actual_bitrate` / `extensions.actual_quality` 报告实际资源；仅有试听时 `playable=false` 且保留 `extensions.trial`，明确授权拒绝时返回 `playable=false`。登录失效、会话冲突和异常响应仍返回错误。此检查不返回媒体 URL，不代表下载授权；匿名路径暂不支持。

汽水歌词入口 `GET /v1/tracks/soda:{id}/lyrics` 支持普通 LRC、逐字 KRC 和明确标记为 `text` 的原文，以及官方提供的中文译文。KRC 返回 `format=krc`、原始 `word_synced` 和派生的 LRC `plain`；普通 LRC 返回 `format=lrc`、`plain`，`word_synced` 为空。官方明确声明 `type=text` 时返回 `format=text` 与无时间轴的 `plain`，保留空行及正文，只统一 CRLF 换行；不会生成逐字轨或时间标签，也不会把损坏的 LRC/KRC 降为纯文本。

`translated` 对应官方 `translations.cn`，保留译文自己的 LRC 时间标签及小数精度，不覆盖原文。没有译文时为空；罗马音仍为空。匿名和服务器账户／调用方凭证使用相同的歌词输出结构，账户选择沿用现有规则。`translated`、`word_synced` 是显示选项，接口仍返回上游实际提供的轨道；真实账户及纯文本歌词实包仍待最终验收。

`contributors` 只列出当前选用歌词来源中有明确消费证据的贡献者：PC 原文作者，以及 Android 原文作者和实际采用的中文译者；缺少昵称时不生成条目。PC 的 `translation_contributor` 尚无已确认消费结构，罗马音仍未接入。

酷我歌词使用 `GET /v1/tracks/kuwo:{id}/lyrics`。默认返回既有原文 LRC／LRCX；显式请求 `translated=true` 或 `romanized=true` 时，才读取官方辅助歌词轨。逐首核验上游支持标记后分别请求译文和罗马音；只把官方 `[ml:1]` 明确标出的同时间辅助行拆成独立 LRC，不覆盖原文。账号与调用方凭证不适用于此匿名入口；`song_type` 和演唱标注也不支持。`extensions.native_lyric_tracks` 保留每轨支持与读取诊断：明确不支持时 `supported=false, available=false`，标记缺失时 `supported=null` 且省略 `available`，内容读取失败会给出该轨的 `error_code`，不会丢弃原文。

酷狗原生标准版／概念版账户支持 `GET /v1/tracks/kugou:{album_audio_id}/lyrics?account=A`，调用方模式使用凭证请求头。`word_synced`、`translated`、`romanized`（及既有别名）沿用公开歌词的显示选项；返回上游实际提供的普通、逐字、翻译和罗马音轨道，缺失轨道保持未知。先核验账户、读取公共曲目目录和歌词候选，再用所选原生会话获取歌词内容。任一步失败即停止，不用另一个格式掩盖认证或协议错误；上游对 KRC 请求返回普通 LRC 时仍可正常使用。迟到响应受同一账户代际检查约束，响应不缓存并沿用凭证更新规则。暂不支持 `song_type` 或演唱标注；真实账户验收待完成。

酷狗 Web 凭证也支持同一歌词入口：先核验账户和曲目身份，再读取官网返回的普通歌词。歌词读取不要求音频 URL、格式或码率；没有逐字、翻译、罗马音轨道时相应字段为空，即使请求了显示偏好也不会生成这些内容。明确空歌词返回 `resource_not_found`，缺失或错误类型的响应字段返回上游错误。数字专辑试听限制下不返回 Web 歌词；试听类型不明确时也会报错。调用方应继续保留凭证更新，认证失效或会话冲突抑制更新，响应不缓存。

账户范围内的歌曲搜索与跨平台候选匹配仍读取公开目录，分页扩展标明 `catalogue_scope=public`，不会向公开搜索发送账户令牌，也不代表个性化搜索或播放授权。匹配到的歌曲会重新进入所选账户的媒体授权链；不匹配的候选不会继续请求授权。

这些账户请求沿用会话轮换与账户隔离规则：退出／重新登录后到达的旧授权不能交付媒体；调用方保留响应中的最新登录凭证。临时用户／单曲授权仅在一次请求中使用，不写入账户仓库或公开结果。媒体不携带登录请求头，响应不缓存，未得到明确期限时 `expires_at` 保持未知；真实账户及权益验收仍待进行。

酷狗原生标准版/概念版凭证支持 `GET /v1/account/playlists?platform=kugou&account=A`，以及本人 `/v1/users/kugou:{uid}/playlists/created`、`/favorites/playlists`。支持服务端所选账户和调用方凭证；本人用户入口的 UID 必须与所选登录一致。真实账户验收仍待进行。

酷狗 Web 凭证在 `/v1/account/playlists` 读取官网旧版“云收藏”目录，分页扩展标明 `source=legacy_web_collection`。完整解析单次上游响应、保留列表顺序后按 `offset`、`limit`（1–100）分页；`total` 仅为本次返回的目录条数，`storage_used_bytes` 是上游 `totalSize` 的占用字节数。此来源有 2 MiB、4096 个目录项和 45 秒总时限，不宣称跨请求快照一致或对应当前原生云歌单全集。引用使用独立的 `kugou:legacy_web_collection:{uid}:{opaque_id}`，可凭同一 Web 账户通过 `/v1/playlists/{reference}` 从目录回读名称；调用方应原样保留引用。读取前后独立核验所选 Web UID，失败不回退到匿名或原生来源，响应沿用凭证更新和 `no-store` 规则。

旧版 Web 目录没有自建／收藏／我喜欢标记、创建者、曲目数或版本信息，这些字段不推断。该专用引用支持同一 Web 账户下的 `GET /v1/playlists/{reference}/tracks`，以及 Uni `playlist` 来源的持久导入和 Client materialize；不支持其他 Uni 来源分类或写入。每个条目都必须解析出完整的酷狗歌曲引用（`kugou:{album_audio_id}`），缺失或无法确认的 canonical ID 会使整页／整次导入失败，不会静默丢弃未知 occurrence；顺序和重复歌曲均保留。

旧版 type=17 没有远端游标或版本号。曲目详情的 `source_snapshot_id` 是对单次完整、无版本响应的内容指纹，只用于比较元数据与各曲目页是否来自内容一致的读取；任何一页不匹配都会使导入失败，Server 不创建部分歌单，Client 不返回部分 materialize 结果。它不构成上游事务快照，也不代表播放、试听或下载权益。type=16 目录本身不能判定自建／收藏／喜欢分类。现有 Web 会话对旧版 PHP 接口的实际兼容性仍待最后真实账户验收。下述 v8 全量库、`cloudlist` 引用及写入规则仅适用于原生标准版／概念版。

这些入口按 v8 云歌单全量协议每页 30 条读取，最多 128 页；逐页检查版本、计数和重复条目，完整读取后按 `offset`、`limit`（1–100）切片。满页后继续读取，包括确认末尾的空页；显式删除记录不进入结果，但参与物理分页和重复检查。分页 `total` 为本次完整读取后的可见数量；上游 `list_count`、`collect_count`、`album_count` 仅保留在扩展字段，不能相加当作结果总数。上游变更、超限或读取错误不会返回部分库。

账户歌单引用为 `kugou:cloudlist:{uid}:{type}:{listid}`，其中 `type=0` 是自建、`1` 是收藏。此引用支持 `/v1/playlists/{reference}`、`/tracks`、`/items` 和普通歌单 Uni 导入。`extensions` 分别保留库拥有者、原始作者来源、全局集合 ID、`total_ver`/`list_ver`、`count`/`m_count` 与原始排序值。`is_def=1` 的默认收藏和 `is_def=2` 的我喜欢分别标记，不按歌单名称或固定 ID 猜测。元数据状态和曲目计数不代表播放权益。

账户歌单详情和每次曲目窗口都先从本人 v8 库定位本地 `listid` 与 `type`，再通过新版 v3 接口每页 300 条、最多 128 页读取全部条目，最后回读账户库核对版本及来源。v3 的明确 `count` 决定曲目结束位置，不与 v8 的 `count`/`m_count` 强行等同；详情 `track_count` 和曲目分页 `total` 使用完整条目数。完整读取后统一按 `sort` 升序排列并切片；全部缺少 `sort` 时保留上游原序，部分缺少则报错。重复歌曲保留，`file_id` 是歌单条目 ID，歌曲引用使用 `mixsongid` 或有效 `add_mixsongid`；`playlist_position` 为显示位置，`upstream_position` 为原始位置。没有可解析歌曲身份或名称的条目会使完整读取明确失败，不静默丢弃或编造引用。

详情和曲目页带 `source_snapshot_id`，普通 Uni 导入会核对全部页面的内容版本。该标识用于检测变化，不承诺上游事务隔离；每个统一分页请求都需要完整回读，大型歌单和 Uni 导入的请求成本较高。当前严格接受新版 `info`、`count`、`list_ver` 结构，旧版 `songs`/`list_info` 不作为自动回退。这些原生私有成功形态及实际账户权益仍待最后真实账户验收。

读取先验证并接受原生会话轮换，逐页及交付前复核同一会话；换号、注销或并发轮换会拒绝迟到结果。调用方保留响应中的凭证更新，认证失效或冲突不返回旧账户凭证；库响应使用 `no-store`。

酷狗喜欢专用读取使用当前账户自建库中唯一的 `is_def=2` 歌单，保留改名后的真实名称、原始条目和重复歌曲；缺少该歌单或标记冲突会明确失败。当前账户 `/v1/account/favorites/playlist`、`/tracks` 和指定本人 `/v1/users/kugou:{uid}/favorites/playlist`、`/tracks` 均支持。Uni `favorite_tracks` 来源使用 `kugou:{本人UID}`，返回的歌单引用仍为实际 `cloudlist` 引用；普通及喜欢来源都核对完整内容版本。

Standard 原生账户的 `PUT` / `DELETE /v1/account/favorites/tracks/kugou:{歌曲ID}` 支持喜欢和取消喜欢。Concept 不支持通过该入口新增喜欢；取消喜欢仅限“我喜欢”列表中唯一出现的歌曲。普通自建歌单的 `POST` / `DELETE /v1/playlists/{reference}/tracks` 或 `/items` 中，Standard 每次支持 1–100 个不同酷狗歌曲引用；Concept 的 `POST` 添加支持 1–100 个不同歌曲引用，但仅限本人 `type=0/is_def=0` 普通歌单。删除要求歌曲在本人普通歌单或 `is_def=2` 喜欢列表中唯一出现；Concept 暂不支持批量删除。`/items` 仅接受 `kind=track`，收藏库 `type=1` 不支持改曲目。Standard 添加不会重复加入已有歌曲，删除会移除目标歌曲的全部原始条目；Concept 添加已存在一次的歌曲时按无操作处理，已有重复项会拒绝。Concept 新增歌曲在同一请求中发送，回执按歌曲身份匹配；完整读回确认输入顺序、原有条目和版本，部分回执或读回异常会标记未确认且不自动重试。添加所需歌曲身份与资源哈希由公开目录核验取得，随后再次检查源歌单，不能将目录元数据视为账户播放授权。

写操作以完整回读确认目标变化和其他条目的保留顺序，返回内容版本、实际曲目数及 `extensions.changed`、`affected_occurrences`、`write_requests_dispatched`。Standard 每次最多删除 300 个原始条目；超过上限会在写入前拒绝，不会拆批。Concept 添加支持单次 1–100 首，Concept 删除仍只支持单首。写入不保证原子性或上游版本锁。已发请求后的失败返回 `details.write_outcome=unconfirmed`、已发请求数和 `retryable=false`，不自动重试或回滚，应重新读取状态后再决定后续操作。标准版和概念版使用各自的添加曲目端点；这些写入的真实账户成功回包及最终业务状态仍待验收。

酷狗原生账户支持 `POST /v1/playlists` 创建普通歌曲歌单。Standard 必须显式指定 `visibility=public` 或 `private`；Concept 只接受 `visibility=platform_default`，不会保证实际可见性。创建结果要求上游给出新的本地歌单 ID，并完整回读确认身份、名称和空列表状态；Standard 还确认可见性，Concept 会标记 `visibility_guaranteed=false`。不会根据同名歌单猜测创建结果。`PATCH /v1/playlists/{reference}` 支持默认 `variant` 下修改名称、简介和标签，保留未修改字段及当前隐私和排序。简介空字符串和空标签数组表示明确清空；标签项不能包含逗号。当前库缺少必须保留的字段时返回 `capability_not_supported`，不发送可能覆盖用户设置的默认值。`extensions.native_metadata` 保留上游实际提供的简介和标签原文，包括显式空字符串。

`DELETE /v1/playlists/{reference}` 和批量 `DELETE /v1/playlists` 支持删除本人普通自建歌单，批量为 1–100 个不同引用。默认收藏和喜欢等系统歌单不允许删除；整个批次先检查身份及类型，再逐项写入并回读确认。中途失败在上述不确定状态之外返回 `confirmed_refs`、`unconfirmed_refs`、`not_attempted_refs`，批量不保证原子性。目录排序数值可由上游重新编号，但其他歌单的元数据、版本和可判断的相对顺序必须保持。

Concept 账户批量删除普通自建歌单时，2–100 个不同目标使用单次加密批量请求；单目标继续使用单独删除入口。发请求前会核验全部目标属于当前账户且为普通自建歌单，并以同一完整目录版本绑定写入。批量回执必须逐项确认所有目标、账户 UID 与版本，之后还要完整回读确认目标恰好消失、其他歌单资料保持；部分回执、版本漂移或不完整回读不会返回成功，也不会自动重试或回滚。

`PUT` / `DELETE /v1/account/favorites/playlists/kugou:{公开集合ID}` 支持收藏和取消收藏歌单。公开集合 ID 与本人收藏库中的 `cloudlist` ID 分开处理，响应保留请求的 `resource_ref`，并在 `extensions.account_playlist_ref` 返回账户内引用；取消收藏也可直接使用本人 `type=1` 引用。添加先核验公开来源拥有者和集合身份；已收藏或已取消时不重复写入。缺少来源身份或重复收藏记录导致状态无法确定时明确失败，不把本人本地 ID 当成原作者 ID。歌单管理已做离线协议和接口测试，当前仍不支持 Web Cookie；真实账户验收留待最后进行。


酷狗标准版和概念版原生账户支持 `PUT /v1/playlists/kugou:cloudlist:{uid}:0:{list_id}/cover`，使用图片原始二进制请求体和对应 `Content-Type`，可传 `account`、`filename`、`image_size`、`crop_x`、`crop_y` 查询参数；调用方凭据也可使用。仅允许本人普通自建歌单；收藏、喜欢等系统歌单以及 Web 凭据尚不支持此操作。

图片支持 JPEG、PNG、GIF、BMP，类型声明须与实际内容一致，上限 20 MiB，宽高至少 400、至多 8192 像素，总像素不超过 16 Mi。`image_size` 是方形裁剪边长，至少 400；`crop_x`／`crop_y` 为应用图片方向信息后的坐标，省略时为 0。未指定边长时使用居中最大方形，此时不可单独传裁剪坐标。标准版结果缩小到至多 1000×1000，概念版固定输出 400×400；透明像素铺白后编码为 JPEG。

封面操作先核验账户及完整歌单目录，再获取上传授权、上传图片、保存封面并完整回读。只有封面与保存回执一致、歌单身份及其余资料保持时才返回 `PlaylistCoverUpdateResult`，其中 `image.url` 为确认后的封面，`extensions.readback_verified=true`。上传图片不等于歌单修改成功；上传后的失败会带 `uploaded_image_may_remain=true`，`upload_requests_dispatched` 与 `write_requests_dispatched` 分别表示上传和保存是否已发出。保存发出后的失败为 `write_outcome=unconfirmed`，不自动重试；未发出保存时为 `not_attempted`。响应沿用账户凭据轮换及 `no-store` 规则。真实账户上传和回读尚待最终验收。

酷狗原生账户支持 `PUT /v1/account/playlists/order`，请求示例 `{"refs":["kugou:cloudlist:111:0:7","kugou:cloudlist:111:0:1"],"account":"A"}`。每个涉及的类别必须提供该类别的全部歌单且不重复；可排序自建、收藏或两类，类别内部按请求顺序排列，未指定类别保持原状。所有引用必须属于选中账户，公开来源集合 ID 不能替代账户歌单 ID。已观察到目标顺序时不重复写入。

`GET /v1/playlists/{reference}/track-occurrences` 在 provider 支持时读取完整原始条目后分页，`limit` 为 1–100。每条返回不透明 `id`、从 0 开始的 `position` 和可空的 `track`；缺少歌曲资料的条目是否保留取决于平台实现。此 ID 只用于所属账户歌单内的条目管理，不是可播放歌曲引用。分页扩展 `source_snapshot_id` 表示本次原始列表状态；遍历不同分页时应保持此值一致。目前酷狗 Standard/Concept 支持本人自建和收藏歌单，最多 38,400 个原始条目；咪咕仅支持本人普通自建歌单，最多 10,000 个条目，并要求原始曲目 DTO 完整。其他平台不因共享路由而自动支持该能力。

`PUT /v1/playlists/{reference}/track-occurrences/order` 接受 `{"occurrence_ids":["entry:111:1:7:83","entry:111:1:7:81"],"snapshot_id":"从读取结果的 pagination.extensions.source_snapshot_id 取得","account":"A"}`。必须提供完整条目排列和读取时的快照值；旧快照、重复、遗漏、其他歌单或账户的条目会被拒绝。酷狗 Standard/Concept 支持本人自建和收藏歌单，最多 38,400 个条目；咪咕只支持本人普通自建歌单和最多 10,000 个条目。咪咕按原生 DTO 与位置重排；同一歌曲引用下字段不同的条目可独立移动，但原生 DTO 完全相同的条目无法证明彼此身份，互换会返回 `capability_not_supported`。咪咕每次最多 16 个原生移动，并逐次完整回读确认；发出写入后的未确认结果不可重试。HTTP 请求体上限为 8 MiB，空歌单可提交空排列作为无写入操作。SDK 方法为 `playlist_track_occurrences` 和 `reorder_playlist_occurrences`，对应 `playlist_occurrence_read` / `playlist_occurrence_write` 能力。

现有 `PUT /v1/playlists/{reference}/tracks/order` 接受完整歌曲引用排列；同一歌曲出现多次时，以当前顺序依次匹配其条目 ID。它要求全部条目都有可解析歌曲，遇到未知条目返回 `capability_not_supported`，请使用原始条目接口。排序使用独立加密协议并执行完整回读，核对条目、顺序、版本及已知资料；响应包含新快照及 `extensions.confirmed`、`write_requests_dispatched`。目录排序核对全库资料和类别顺序。上游不提供事务保证；发送后任何未确认结果均标为 `write_outcome=unconfirmed`、不可自动重试，不自动回滚或重发。SDK 的空歌曲排列仅适用于空歌单；现有通用 HTTP `tracks/order` 仍要求非空引用。


咪咕单曲已购记录使用 `GET /v1/account/purchases/tracks?platform=migu&account=A`，也支持默认账户及调用方凭据。SDK 对应 `account_purchased_tracks`，能力为 `account_purchased_tracks`。仅包含单曲已购接口的记录，专辑订阅附带的歌曲不混入；已购专辑见下文。不会与喜欢或收藏库合并。

每次先核验所选 UID，读取完整 `contentId` 列表，按 `limit` / `offset` 补齐公开歌曲资料，再复核完整列表和同一账户身份。保留顺序与重复记录；`limit` 为 1–100，完整列表最多 10,000 项、每个已购响应最多 1 MiB，整次调用期限 45 秒。`total` 是完整记录数；超出范围的窗口返回空页，读取失败不作为空库。两次观察不一致返回 `conflict`，不承诺上游原子快照。

每项的 `extensions.resource_ref` / `content_id` 保留购买身份；公开目录明确找不到歌曲时 `track=null`、`catalogue_resolved=false`，仍保留该记录。其他目录错误继续返回错误。歌曲资料标记 `catalogue_scope=public`，购买记录不授予当前播放、下载或音质权限。分页扩展包含 `source_user_id`、`source_snapshot_id`、`complete_read` 和 `consistency=two_complete_reads`。凭据更新和 `no-store` 沿用账户接口合同；购买歌曲可通过 `type=purchased_tracks` 导入 Uni，使用所选账户 UID 并保留原始顺序及重复项。真实非空购买库及权益仍待最终账户验收。

咪咕已购专辑／专辑订阅记录使用 `GET /v1/account/purchases/albums?platform=migu&account=A`，SDK 为 `account_purchased_albums`。服务端默认／指定账户和调用方凭据均支持。结果中的普通专辑放在 `album`，数字专辑放在可选的 `digital_album`，最多填充其中一个；数字专辑不会改成普通专辑，即使 ID 相同。新增字段为空时省略，因此旧平台购买库的 JSON 结构保持兼容。缺少标题的记录两者均为空，保留 `extensions.content_id`、`resource_ref`、`resource_type` 和 `catalogue_kind`。

该入口读取官方专辑订阅来源，不声称覆盖平台全部历史订单或退款记录。完整遍历两遍，比较有序类型化记录及分页边界后才切片；保留重复记录，未知类型、循环分页、缺失终止标记及中途失败均返回错误。每页请求 50 项，每遍最多 128 页／6,400 条，单个购买响应最多 1 MiB，两遍累计最多 16 MiB，整个调用期限 120 秒；调用窗口 `limit` 为 1–100。短页仍以明确的 `hasNextPage` 判断是否继续。购买目录字段来自账户响应，价格、期限、订单号及当前媒体权限没有证据时保持未知；`DigitalAlbum.purchased` 不由这一条记录推断当前状态。已购专辑可通过 `type=purchased_albums` 导入 Uni，单曲使用 `purchased_tracks`；两种来源都使用所选账户本人 UID，见 [Uni Playlist](uni-playlist.md)。真实账户验收仍待完成。

酷狗已购音乐库使用 `GET /v1/account/purchases/tracks?platform=kugou&account=A` 和 `/v1/account/purchases/albums`，调用方凭证模式沿用现有请求头。SDK 对应 `account_purchased_tracks` / `account_purchased_albums`，能力为 `account_purchased_tracks` / `account_purchased_albums`；收藏库接口不作为购买库使用。原生账户的歌曲和专辑记录分别可通过本人 UID 的 `type=purchased_tracks` 与 `type=purchased_albums` 导入 Uni；保留来源顺序及重复项，未解析条目会拒绝，详见 [Uni 账户来源](uni-playlist.md)。

两类入口分别返回 `PurchasedTrack` / `PurchasedAlbum`：`track` / `album` 可为 `null`，同时保留可用的 `name`、`artists`、`cover_url` 及原始商品标识扩展。只使用明确的 `album_audio_id` 或 `album_id` 建立目录引用；缺少身份或标题时保留未解析购买条目，不把 `goods_id`、`good_scid` 或哈希当作可播放引用。不同商品条目可指向同一歌曲或专辑，保持原序，不按目录引用去重。原始商品标识不是支付订单号。

歌曲每页 50 条、专辑每页 15 条，每类每次完整读取最多 128 页（6,400 / 1,920 条）；每个统一请求执行两次完整读取，核对总数、所有条目和已知资料一致后，再应用 `limit`（1–100）和 `offset`。分页总数包括未解析条目，扩展包含 `unresolved_entries`、`source_snapshot_id`、`upstream_pages_fetched` 和 `consistency=two_complete_reads`。此比较不提供上游事务隔离；大库读取成本较高。缺少 `goods` / `total`、短页、重复商品、分页中途变化、明确删除记录或读取失败均返回错误，不输出部分成功或伪造空库。

`extensions.goods_id`、`good_scid`、`album_audio_id`、`album_id` 分开保留；可用的音频时长保留毫秒，哈希仅作为目录资源描述。缺少歌曲数、发行日期和作者 ID 时保持未知；购买记录和原始状态码不直接建立当前播放、下载、会员或音质权限。账户轮换逐页及交付前核验，响应使用 `no-store`，调用方应保留更新后的凭证。当前支持标准版和概念版原生账户；Web Cookie 和购买操作尚未接入。`purchased_tracks` 与 `purchased_albums` Uni 来源已接入。已购成功回包及实际权益仍待最终真实账户验收。

普通平台歌单统一使用 `/v1/playlists`：

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `POST` | `/v1/playlists` | 创建平台歌单 |
| `PATCH` | `/v1/playlists/{ref}` | 修改歌单元数据 |
| `DELETE` | `/v1/playlists/{ref}` | 删除单个歌单 |
| `POST/DELETE` | `/v1/playlists/{ref}/tracks` | 添加或删除歌曲 |
| `PUT` | `/v1/playlists/{ref}/tracks/order` | 提交歌曲顺序 |
| `POST/DELETE` | `/v1/playlists/{ref}/videos` | 添加或删除视频 |
| `PUT` | `/v1/playlists/{ref}/cover` | 更新封面 |

汽水 `POST /v1/playlists` 只支持创建空的普通歌单，`kind=normal`、`visibility=public|private` 必须明确给出；名称为 1–30 个 UTF-16 单元且不能含控制字符。操作使用指定服务器账户或调用方凭据，先核验本人 UID，写入后完整回读本人自建歌单确认新 ID、名称和空曲目数。上游目录不提供可见性字段，因此结果只回显 `requested_visibility` 并标记 `visibility_verified=false`。写入已发出但 ACK 或回读未确认时返回 `write_outcome=unconfirmed`，不可自动重试；响应为 `no-store`。完整账户边界见[登录与凭证](authentication.md)。

汽水 `PUT /v1/playlists/{reference}/visibility` 支持 `{"visibility":"public"}` 或 `{"visibility":"private"}`，使用所选服务器账户（省略时为 `default`）或调用方凭据，不会匿名回退。写入前从完整的本人自建歌单目录确认目标 ID 和 owner UID；随后使用官方 PC `UpdatePlaylistInfo` 仅提交 `playlist_id` 与 `is_private`，以顶层 `status_code=0` 作为成功回执。写后使用同一所选账户完整读取官方 `GetPlaylistDetail` 分页，要求所有页面的 `is_private` 明确一致，并确认它等于请求值且歌单 ID/owner 仍匹配；缺失、null、跨页冲突、状态不符或所有权不符均返回 `write_outcome=unconfirmed`。成功结果标记 `visibility_verified=true`，返回已读回歌单并在 `playlist.extensions.is_private` 提供确认值；单次写入、不可自动重试，响应为 `no-store`。

汽水 `PATCH /v1/playlists/{reference}` 支持默认变体下修改标题和／或简介：标题为 1–30 个 UTF-16 单元且不能含控制字符；显式提供空字符串可清空简介。标签和其他变体返回 `capability_not_supported`。实现先用所选账户或调用方凭据完整读取本人自建歌单，确认 owner UID 后向官方 PC `UpdatePlaylistInfo` 只发送明确提供的 `playlist_id`、`name` 和／或 `description`，不覆盖未提供字段；随后完整回读目录，确认请求字段已更新且未提供字段、曲目数和拥有者均保持原值。该回读不包含隐私字段，也不声称隐私已确认。写入未取得回执或读回不符时返回 `write_outcome=unconfirmed`，不可自动重试；响应为 `no-store`。

汽水 `DELETE /v1/playlists/{reference}` 支持删除单个本人自建歌单，使用所选账户或调用方凭据。删除前从完整的本人自建歌单目录确认目标及 owner UID；随后向官方 PC `MDeletePlaylists` 单次提交该 ID，并要求完整 Created 目录的 ID 集合只少了目标歌单。当前不支持一个 SDK 批次删除多个汽水歌单。ACK 丢失或完整读回未确认时返回 `write_outcome=unconfirmed`，不可自动重试；响应为 `no-store`。

汽水本人普通歌单支持 `POST`／`DELETE /v1/playlists/{reference}/tracks`，以及 `/items` 的 `kind=track`。每次最多提交 100 个不同的汽水歌曲引用，只发送官方 PC `MAppendPlaylistMedia` 或 `DeletePlaylistMedia` 接受的 `playlist_id` 与 `media[{id,type:"track"}]`；添加按请求顺序追加，删除请求引用的全部出现次数。写入前必须从完整 Created 目录和完整歌单曲目中确认所选账户及普通歌单身份；写入后再完整读取曲目和 Created 目录，核对有序条目、目标歌单资料与本人 Created 目录 ID 集合。视频、喜欢／系统歌单和不完整读取不支持；上游 ACK 或完整读回不能确认时返回 `write_outcome=unconfirmed`，不可自动重试。操作使用指定服务器账户或调用方凭据，响应为 `no-store`；真实账户写入验收留待全部自动化开发结束后进行。

汽水本人普通歌单也支持 `PUT /v1/playlists/{reference}/tracks/order`，`refs` 或 `ids` 必须给出当前完整曲目顺序，重复歌曲按出现次数保留。实现沿 Android 手动拖动排序请求提交 `playlistId` 与有序 `media[{id,type:"track",attr:{duration}}]`，只使用所选登录 Cookie、`x-luna-api-version: 2023-01-04` 和 `x-luna-is-login: 1`。写入前验证完整 Created 目录、所选账户 UID、普通自建歌单及输入引用与当前曲目多重集一致；ACK 必须显式成功且返回相同歌单 ID 和完整有序歌曲 ID，随后用同一账户完整读回歌单和 Created 目录 ID 集合确认结果。相同顺序不发送写入；其他变体、喜欢／系统歌单、空排列及不完整读取不支持。写入一旦发送不重试；ACK 或回读不符返回 `write_outcome=unconfirmed`、`retryable=false`。响应为 `no-store`，Android 排序真实账户验收待自动化开发全部结束后进行。

咪咕本人普通自建歌单支持 `PUT /v1/playlists/{ref}/tracks/order`，`refs` 或 `ids` 必须提供 1–10,000 首的完整新顺序，数量及每首资源引用的重复次数必须与当前曲目完全一致。重复引用只表示相同歌曲资源的出现次数，不提供可持久选择的单个 occurrence ID；仅当同一资源的原生歌曲身份、标题和歌手字段完全一致时接受重复项，无法区分的原生记录组合会在写入前拒绝。此歌曲引用排序入口不接收单个 occurrence ID；需要逐条目排序时，使用上文独立的 `PUT /v1/playlists/{reference}/track-occurrences/order`。收藏／系统歌单不支持这两个排序入口。实现最多逐首移动 16 次，每次都要收到原生成功码并完整回读确认；超过预算、读回差异或中途失败不会自动重试或回滚，多步写入不具原子性。首曲变化时上游可能更新派生封面；自定义封面行为仍待最终真实账户验收。支持所选账户和调用方凭证，响应沿用账户凭证更新规则。

账户资料库和收藏操作位于 `/v1/account/*`，包括个人歌单、喜欢歌曲、收藏专辑、播客、视频、广播、历史、会员状态和云盘。写操作需要目标平台凭证，并可能受平台权益、频率和风控限制。

咪咕收藏专辑区分普通专辑与数字专辑：普通专辑使用 `/v1/account/library/albums`，数字专辑使用 `/v1/account/library/digital-albums`。两者均支持 `GET` 列表、`PUT` / `DELETE` 批量收藏操作，以及追加 `/migu:{id}` 的单项 `PUT` / `DELETE`。本人收藏列表也可从 `/v1/users/migu:{uid}/favorites/albums` 或 `/v1/users/migu:{uid}/favorites/digital-albums` 读取。普通引用的 ID 是 `albumId`，数字引用是 `contentId`，不能混用；SDK 分别返回 `Album` / `DigitalAlbum`，对应 `account_albums` / `account_digital_albums` 和各自的订阅写方法。

这两类咪咕目录先完整读取混合收藏，再按类型分页，单次最多 64 个上游页、每页 10 项，`limit` 为 1–100。每项写入后都要完整回读并核对独立收藏状态；批量逐项执行，失败报告已确认、失败和未尝试的引用，不能当作原子事务重发。收藏不表示已购买或获得播放权益，未知价格和购买状态保持未知。账户选择、凭据更新与详细限制见[登录与凭证](authentication.md)。

汽水只读已购数字专辑目录使用 `GET /v1/account/library/digital-albums?platform=soda`，SDK 方法与能力均为 `account_digital_albums`。支持指定或默认服务器账户，也可使用 `X-TuneWeave-Credential` 调用方凭据；匿名请求不回退。实现先核验所选账户 UID，再从官方 PC `GetMyDigitalAlbums` 读取一次完整目录，最多 10,000 项，之后在本地应用 `limit`（1–100）和 `offset`（最多 10,000）；分页包含 `source_user_id`、`complete_snapshot` 和 `source_snapshot_id`。目录只映射官方消费端使用的专辑 ID、名称、艺人、曲目数、封面和发行时间。`DigitalAlbum.purchased=true` 仅表示该条目来自汽水本人已购数字专辑目录，不代表当前播放或下载权益；此目录不是订单明细。

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `GET` | `/v1/account/favorites/playlist` | 当前账户的“喜欢”歌单元数据，包括平台原封面 |
| `GET` | `/v1/account/favorites/tracks` | 当前账户喜欢的歌曲 |
| `GET` | `/v1/users/{ref}/favorites/playlist` | 指定用户可见的“喜欢”歌单元数据 |
| `GET` | `/v1/users/{ref}/favorites/tracks` | 指定用户可见的喜欢歌曲 |
| `GET` | `/v1/account/favorites/tracks/intelligence` | 以喜欢歌单为来源生成心动模式/智能播放队列 |

网易云“喜欢”歌单名称使用平台资料昵称生成 `{username}喜欢的音乐`，昵称不可用时为“我喜欢的音乐”，封面保持平台歌单原值。QQ 使用“我喜欢”目录的原始名称和封面。心动队列要求 `seed` 歌曲引用，可选 `start` 和 `count`；`count` 保留网易云协议语义，不作为 TuneWeave 分页大小，返回队列不伪造 offset 或 continuation。

汽水喜欢列表使用账户目录中的真实集合身份和原始名称，支持当前账户读取、指定本人 UID 读取和 Uni `favorite_tracks` 导入。`PUT` / `DELETE /v1/account/favorites/tracks/soda:{id}` 在写入后完整回读核对状态；失败不报告已确认成功，也不自动重发。服务器别名和调用方凭证均可使用，会话更新通过响应头返回。分页、身份边界及待验收项见[登录与凭证](authentication.md)。

`GET /v1/account/following/artists?platform=...&account=...&limit=...&offset=...` 读取所选账户关注的歌手目录，返回 `Artist[]` 和统一分页元数据。`limit` 默认 25、范围 1–100；`offset` 默认 0。可选择服务器账户，或使用 `X-TuneWeave-Credential` 调用方凭证；两者不能同时指定。此入口依赖 provider 的 `account_following_artists` 能力，不支持的平台返回 `capability_not_supported`。它只表示关注目录读取，不隐含关注写入、好友关系读取或媒体权益；写入需要单独的 `artist_subscription_write` 能力。

汽水单艺人关注使用 `PUT /v1/account/following/artists/soda:{artist_id}?account=A`，取消关注使用同一路径的 `DELETE`；也可省略 `account` 使用默认账户，或改用 `X-TuneWeave-Credential`（两种账户来源不可同时指定）。当前支持单个正十进制艺人 ID；先确认所选账户身份并完整读取关注目录，状态已符合请求时不重复写入，否则提交一次官方 Android 添加/删除请求，再完整分页读回确认。分页游标必须前进且各页总数稳定。写入后的 ACK 或读回不能确认时返回 `write_outcome=unconfirmed`，不可自动重试；响应为 `no-store`。此段说明汽水的单艺人写入语义；关注目录读取按前述 GET 端点及 provider 的能力声明提供。

酷我也支持单艺人关注：对 `kuwo:{artist_id}` 使用 `PUT /v1/account/following/artists/kuwo:{artist_id}?account=A`，取消时改用 `DELETE`。账户选择方式与汽水相同；只接受正十进制艺人 ID，并限定为已选择的酷我原生账户。实现先完整读取该账户的关注目录，已符合目标时不再写；否则按官方原生歌手关注请求只发一次，再完整读取确认目标状态，并要求其他艺人顺序不变。写入后的 HTTP 回执或读回无法确认时返回 `write_outcome=unconfirmed`，不可自动重试；响应为 `no-store`。此能力与关注目录读取、媒体权益分别声明，不支持该写操作的 provider 不会因共享路由而自动支持。

汽水收藏专辑通过 `GET /v1/account/library/albums?platform=soda` 或 `GET /v1/users/soda:{uid}/favorites/albums` 读取，指定 UID 必须属于选中账户。`PUT` / `DELETE /v1/account/library/albums/soda:{id}` 支持单项操作，同方法的 `/v1/account/library/albums` 支持一次 1–100 项的 JSON 批量请求。写后完整回读确认，批量中途失败会报告已确认项、未确认项及未尝试项，不自动重发或宣称原子成功。目录分页、未知曲目数、会话更新和错误字段见[登录与凭证](authentication.md)；账户专辑详情和曲目使用上述资源入口，真实账户验收待完成。

## 云盘分片直传

能力标识为 `account_cloud_upload_transfer`，当前由网易云 provider 实现。原有单次 `/v1/account/cloud/uploads/ticket`、`/complete` 和服务端代理上传保持兼容；需要大文件、413 回退及任务内续传的客户端应使用以下独立流程。音频仍由客户端直传存储服务，TuneWeave 只编排步骤及解析客户端提交的存储响应。

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `POST` | `/v1/account/cloud/uploads/transfers` | 绑定文件及当前账户，分配上传任务 |
| `GET` | `/v1/account/cloud/uploads/transfers/{id}` | 读取已确认偏移与当前步骤 |
| `POST` | `/v1/account/cloud/uploads/transfers/{id}/advance` | 提交当前存储响应，获取下一步 |
| `DELETE` | `/v1/account/cloud/uploads/transfers/{id}` | 取消任务并阻止后续发布步骤 |
| `POST` | `/v1/account/cloud/uploads/transfers/{id}/complete` | 上传确认完成后，提交元信息并发布 |

所有路由支持 `platform` / `account` 查询参数及既有调用方凭据头，返回统一响应包络和 `Cache-Control: no-store`。调用方凭据与服务器账户别名不能混用；每一步须保持原凭据来源。未实现此能力的 provider 返回 `capability_not_supported`。

创建请求示例（`strategy` 默认 `auto`，也可指定 `chunked`）：

```json
{
  "file": {
    "md5": "0123456789abcdef0123456789abcdef",
    "file_size": 9000000,
    "filename": "example.flac",
    "bitrate": 999000,
    "content_type": "audio/flac"
  },
  "strategy": "auto"
}
```

`bitrate` 默认 `999000`；文件 MD5、长度和内容必须由客户端在开始前确认，并在任务期间保持不变。任务内不能替换文件、资源 ID 或账户。

返回的 `data` 包含 `transfer_id`、`state`、`file_size`、已确认的 `offset`、Unix 秒 `expires_at`、`upload_required` 和可空 `step`。`state` 只有 `transferring` / `ready_to_publish`；平台明确表示无需传输时，直接进入后者。

每个 `step` 包含不透明的 `step_id`、`kind`（`upload` / `probe`）、`offset`、`length`、`method`、`url`、`headers`、`retry_delay_ms`、`attempts_remaining`。 `attempts_remaining` 表示连续失败预算（含当前请求），成功的偏移探测不会重置此预算。客户端先等待指定延迟，再按返回信息执行请求：`upload` 只发送原文件 `[offset, offset + length)` 的字节，`probe` 长度为零、无请求体。禁用自动重定向和 HTTP 自动重试，不附加平台 Cookie、TuneWeave 凭据或其他应用授权；上传 URL、授权头和上下文不得写日志或传播到 Minecraft 服务器。客户端无需识别 NOS URL、构造偏移参数或解析厂商 JSON。

客户端将 HTTP 状态、响应头和原始文本响应体交给 `advance`，例如：

```json
{
  "step_id": "returned-step-id",
  "status": 200,
  "headers": {"x-nos-context": "returned-context"},
  "body": "{\"offset\":4194304,\"context\":\"returned-context\"}"
}
```

传输失败且未取得响应时提交 `status: null`，此时 `headers` / `body` 必须为空。响应体上限为 64 KiB，响应头最多 64 项、名称和值合计 16 KiB；HTTP JSON 总体上限为 128 KiB。超限响应应取消任务，不截断后当成成功，也不臆造偏移。旧 `step_id`、越界包络或未知请求字段不会推进当前任务。若 `advance` 的 TuneWeave 响应丢失，先 `GET` 当前状态核对 `step_id`：仍为原 ID 时重交同一份存储回执，已变化时采用返回的新步骤，避免再次执行已经完成的存储请求。若丢失的是存储响应本身，则用 `status: null` 交由 provider 判断恢复方式；不得并行执行同一任务的步骤。

网易云当前 `auto` 策略在文件超过 200 MiB 时使用 4 MiB 分片，整文件请求收到 413 时切换分片；分片继续收到 413 时逐步减小，最低 64 KiB。临时网络错误或 HTTP 408 / 425 / 429 / 5xx 最多重试两次；已有上下文时先查询存储偏移，再继续未确认范围。探测成功不会重置重试预算，实际上传确认后才重置。响应必须明确确认合法偏移，不能按已发送字节数猜测成功；偏移回退、越界、上传未前进、矛盾上下文或无效响应会终止任务。短 `Retry-After` 秒数会转换为等待提示；大于 30 秒的提示终止当前任务，日期形式不做推断。

只有 `state=ready_to_publish` 且 `step=null` 时才能调用该任务的 `/complete`。请求仅允许可选的 `song_name`、`artist`、`album`（不需要覆盖时发送 `{}`）。provider 使用任务绑定的歌曲、资源、文件及账户执行最终发布，成功返回 `CloudUploadResult`。这表示 TuneWeave 已校验客户端报告的存储响应并完成元数据发布，不是服务端独立读取并验证音频内容。

任务保存在当前 provider 实例的内存中，最多 128 个，有效期最长一小时；`expires_at` 是本地任务期限，不保证上游授权令牌同样有效。重启、实例切换或任务过期后不能续传，需要新建任务；多实例部署须保持请求落在同一实例。此接口不提供进程重启后的持久化断点恢复。

取消时客户端应立即停止自己的存储请求并调用 `DELETE`。服务端会阻止尚未发出的后续发布请求，但不承诺撤回已经送达存储或平台的操作、删除已上传的临时对象或撤销已交付的存储授权。服务器账户重新登录/注销、调用方凭据切换会拒绝旧任务；通过 TuneWeave 注销调用方凭据也会取消对应任务。发布开始后不允许重复提交；取消、失败或请求被中断均消费该任务。发布阶段失败的 `details.publish_outcome=unconfirmed` 表示可能已有写入，应先核对云盘状态，不能自动重新发布；传输阶段终止则返回 `details.transfer_consumed=true`、`published=false`。

## 登录与 Uni Playlist

酷狗短信使用 `POST /v1/auth/challenges` 创建事务，支持 `standard` 后端、中国大陆手机号和三种凭据归属。`allow_account_creation` 默认 `false`，仅显式 `true` 允许平台要求的新账号确认。向既有 `/v1/auth/challenges/{transaction_id}/verify` 提交验证码；若返回 `state=account_selection_required`，从 `accounts` 中选择账号，再以 `action=select_account` 和字符串 `user_id`、`code` 继续原事务。短信登录业务码 `20028` 返回 `state=browser_verification_required`，按 `verification` 在官方页面完成人工验证，并以 `action=submit_browser`、原 `verification_id`、再次提供的 `code` 和原始 `dataJson` 字符串 `response` 接续。中间状态不包含凭据，只有独立身份核验与条件保存成功才确认登录。浏览器消息来源校验、期限、次数、冷却及 SDK 用法见[酷狗短信说明](authentication.md#酷狗短信登录与账户选择)。

酷狗 Web 密码入口支持平台要求的二次手机验证：同一密码事务接收 `submit_sms`、`resend_sms`、`select_account`；短信浏览器验证返回 `verification.method=sms_browser`，使用 `submit_sms_browser` 提交 `verification_id`、短信 `code` 和官方 `dataJson` 字符串 `response`，不提交密码。原生密码浏览器仍使用独立的 `submit_browser` 协议；Web 初始图形／交互及绑定验证尚未接通。详见[Web 密码说明](authentication.md#酷狗-web-密码登录)。

咪咕密码登录若要求自动语音验证，返回 `verification.method=voice`；使用同一密码事务提交 `action=submit_voice` 与 `code`，或用 `action=resend_voice` 请求再次呼叫。它与短信验证使用不同的上游协议；期限、冷却和失败后的事务处理见[登录与凭证](authentication.md#密码登录)。

汽水扫码创建即绑定请求中的 `account`；注销或重新登录后，旧扫码／导入不能覆盖当前账户，竞争返回 `conflict`。具体期限、SDK 兼容入口与跨进程保证见[登录事务说明](authentication.md#二维码登录)。

登录端点位于 `/v1/auth/*`，详见[登录与凭证](authentication.md)。

Uni Playlist 端点位于 `/v1/uni/*`，服务端歌单也可通过统一 `/v1/playlists/{uni-ref}` 读取和播放，详见 [Uni Playlist](uni-playlist.md)。

## 平台扩展

无法映射为稳定统一语义的能力位于：

```text
/v1/extensions/{platform}/...
```

扩展端点仍使用统一响应包络、凭证选择和错误模型，但请求与 `data` 可能包含平台专属结构。它们不允许调用方覆盖目标域名、通用代理、Cookie、任意请求头或重定向策略。完整扩展路由请查阅 [`routes.json`](routes.json)。


## 酷我公开目录搜索

`GET /v1/search?platform=kuwo&kind=album|artist|playlist|mv&q=...` 支持专辑、歌手、歌单和全局 MV 搜索，使用既有 `SearchItem` 联合类型；歌曲仍使用 `kind=track`。`limit` 为 1–100，`offset` 从 0 开始，只接受默认搜索方式；不接受账户、调用方凭证、`search_id`、高亮、选择器或视频过滤。

返回项按官方顺序保留。跨页读取会检查总数、页号和结果身份；数据变化、缺页或重复页返回上游错误，不输出部分拼接结果。明确的总数 0 和空列表表示没有结果；专辑接口偶尔返回缺少总数的空对象，此时返回上游错误，不将未知结果当作空集合。

MV 搜索返回 `SearchItem::Video`，`extensions.kind=mv`，引用使用官网 `/mvplay` 的歌曲 ID，与现有视频详情/播放入口一致；不使用另一套 `mvpayinfo.vid` 编号。名称、歌手署名、时长及播放计数来自搜索目录，未知会员、收藏和播放授权保持未知。与歌手 MV 目录相同，保留下线条目及其 `extensions.online=0`，不做隐式过滤；`pagination_scope=upstream_catalogue_positions` 明确 offset 和 total 对应上游目录位置，调用方隐藏下线项时仍应使用返回的 `next_offset`。

MV 每页固定 20 条，先读首页确认总数，再读取所需窗口，最多 7 个目录页（包括首页）。上游越界页可能返回总数 0，因此不会使用越界回包覆盖首页总数。每个 API 响应最多 2 MiB，整个搜索含匿名会话获取/刷新限时 45 秒；取消、超时、总数变化、重复身份和不完整页面均不返回部分结果。只允许已支持的匿名会话拒绝触发一次刷新重试；不因限流、服务错误或坏数据重试。当前排序仅平台默认，不接受 `order`、`duration`、`category_id` 或其他视频过滤。

搜索摘要不保证包含专辑曲目数、完整歌手作品、会员权益或收藏状态；未知字段保持未知。歌单创建者只有昵称时放在 `extensions.creator_name`，不构造用户或歌手身份。歌手和专辑详情及作品目录见下文。


## 酷我歌手详情与作品目录

酷我支持既有 `/v1/artists/{reference}`、`/overview`、`/tracks` 和 `/albums` 入口。歌手详情保留完整简介、别名和明确的歌曲/专辑/MV 数量；概览仅含最多 10 首歌曲，`has_more_tracks` 表示是否还有作品。更多作品使用可分页目录，不把概览当作全集。

歌曲目录需显式指定 `order=platform_default`，例如 `GET /v1/artists/kuwo:336/tracks?order=platform_default&limit=20&offset=20`；不承诺热门或时间排序。歌曲和专辑目录的 `limit` 为 1–100，`offset` 从 0 开始；均为公开资料，不接受账户或调用方凭证。

作品读取先核对歌手身份，再用第一页确认总数后定位请求窗口；越界窗口保留正确总数并返回空列表。每次最多读取第一页及 6 个窗口页，第一页在窗口内时复用；这不表示每次下载全部作品。歌手详情与目录数量矛盾、跨页总数变化、重复结果或作品归属不符时返回上游错误。目录音质信息不代表当前账户的播放或下载权限。

合唱歌曲和合作专辑保留官方署名顺序。上游只提供首位歌手 ID 时，其余署名保留姓名，资源引用保持未知；通过已核实的歌手名称或别名确认目录关联，不补造其他歌手 ID。`extensions.source_artist_id` 表示本次查询的歌手目录。

`GET /v1/artists/catalog?platform=kuwo` 返回酷我官方“百城声音”主播目录；它与歌手分类目录是不同入口。该上游只提供一次完整数组，没有筛选、分页或账户参数；请求只接受默认的 `area=all`、`type=all`、`genre=all`，不接受账户或调用方凭证。目录中的 `id` 是艺人 ID，可直接用于上方详情和作品入口；`channel_id` 是可选关联电台 ID，`0` 保持未知。`source_priority` 与 `source_status` 仅保留上游原值，不解释其排序或可用性；缺少作品数量时保持未知。节目表仍是独立的时刻表资料，不构造可播放节目条目。

酷我歌手分类目录使用 `GET /v1/artists?platform=kuwo&area=chinese&type=male&initial=A&limit=30&offset=0`。它读取官网 Web 歌手目录，`limit` 为 1–100、`offset` 从 0 开始；`area` 与 `type` 支持已验证的组合：全部、华语男／女／组合、欧美男／女／组合、其他（`area=other&type=all`）。日韩、仅地区或仅性别筛选均不支持。`genre` 只接受 `all`，`initial` 为空、单个英文字母或 `#`。上游每页 60 项，任意窗口最多读取 3 页；保持平台默认顺序并检查跨页总数和重复歌手。只支持匿名请求，拒绝账户与调用方凭据；目录不足以表示全平台完整歌手全集。

## 酷我 HiFi 专辑目录

酷我另支持官方 HiFi 专辑目录：`GET /v1/albums?platform=kuwo&catalog=hifi_latest&limit=30&offset=0`，或将 `catalog` 设为 `hifi_hot` 获取最热排序。`catalog` 必填且只接受这两个值；目录不接受 `area`、账户或调用方凭证。条目使用普通酷我专辑 ID，可继续调用下方专辑详情和曲目入口。

上游每个物理页固定 18 项。为避免越界页把总数重置为 0，窗口读取总先取第 1 页确认目录总数，再读取所需页；每次最多 8 个物理页。跨页总数变化、重复专辑或数据不完整会返回错误，不返回部分拼接结果。`extensions.catalogue_hires_status`（存在时）仅为目录标记，不代表账号拥有 HiFi 播放或下载权益。

## 酷我专辑详情与曲目

`GET /v1/albums/kuwo:{id}` 和 `/v1/albums/kuwo:{id}/tracks` 支持公开专辑资料及曲目，拒绝账户和调用方凭证。曲目分页接受 `limit=1–100` 与从 0 开始的 `offset`，保留官方曲序；越界返回空列表和已核实的总数。

每次读取完整专辑列表，核对专辑身份、声明曲数、曲目归属和重复 ID 后才返回请求窗口。`extensions.complete_read` 表示本次上游列表已完整读取，不表示窗口包含所有曲目。响应限 2 MiB、最多 1000 首；超限或不完整的数据返回上游错误。不具备专辑身份的空对象也返回上游错误，不当作有效空专辑。

详情保留完整简介、厂牌、发行日期、语言和明确曲数。合作署名采用上游实际 ID；缺少 ID 的署名仍可显示姓名。目录音质和付费标记不代表账户已获得播放或下载权限。

## 酷我歌手 MV 目录

官方 MV 分类使用 `GET /v1/videos/taxonomy?platform=kuwo&kind=groups`，返回固定的 9 个 Web 分组；例如 `GET /v1/videos?platform=kuwo&catalog=group&group_id=236682731&type=mv&limit=20&offset=0` 读取“华语”目录。此处的 `group_id` 必须是官方分组 ID；每页 20 项，`limit` 为 20、40、60、80 或 100，`offset` 必须是 20 的倍数。分页位置和 `total` 对应上游目录槽位，稀疏页或空页仍需按 `next_offset` 继续；下线条目保留，不将“首播”解释为完整或按时间排序的 MV 集合。两条入口均只接受匿名请求。

`GET /v1/artists/kuwo:{id}/videos?kind=mv` 返回歌手的官方 MV 集合，含官网归入该集合的现场和预告等内容。必须显式指定 `kind=mv`；接受 `limit=1–100`、从 0 开始的 `offset`，使用平台默认顺序，不接受 `cursor`、自定义 `order`、账户或调用方凭证。

读取会核对歌手身份、MV 数量及分页完整性。下线项保留原位置，并通过 `extensions.online=0` 表示目录状态；在线状态也不代表播放许可。未知播放量、时长、收藏状态保持未知。

酷我目录中的 MV 引用与曲目的 `mv_ref` 使用官网 `/mvplay` 所需的歌曲 ID；上游 `mvpayinfo.vid` 属于另一套编号，仅保留在原始 MV 元数据中，不作为这些引用的 ID。

MV 详情使用 `GET /v1/videos/kuwo:{id}?kind=mv`，统计使用 `GET /v1/videos/kuwo:{id}/stats?kind=mv`。详情沿用官网关联资料中的标题、署名、封面、时长和 MV 播放量；歌曲发行日期仅放在 `extensions.source_track_release_date`，不当作 MV 发布日期。分辨率列表保持空，收藏、点赞等未知状态不补造。明确不存在 MV 的歌曲返回 `ResourceNotFound`；不完整或矛盾的数据返回上游错误。

匿名 MV 播放使用 `GET /v1/videos/kuwo:{id}/stream?kind=mv&resolution=1080`，支持既有 `/stream/redirect` 和 `GET/POST /v1/videos/streams`。批量为 1–100 项，保留顺序和重复引用，同一批内每个资源只解析一次；下一次请求重新读取权限。所有输入先校验，过程中出现上游或数据错误时不返回部分批次。

每次先读取当前 MV 权限；明确下线、禁播或未获授权时返回 `available=false`、`url=null`，原因在 `extensions.unavailable_reason`。未知权限返回上游错误。只有平台明确允许且地址接口再次成功后才返回官方 HTTPS MP4 地址；跟踪 Cookie 和签名不会放进媒体请求头。

官网只提供默认视频地址，`resolution` 接受正整数作为偏好记录，不承诺选择对应清晰度。`actual_resolution`、宽高、码率、文件大小和过期时间保持未知；时长来自官网关联资料。播放支持不代表账户播放、下载权限或更多清晰度已经接入。


## 酷我原生账号

`platform=kuwo` 的统一密码登录、会话查询、会话刷新、本地注销和独立上游会话撤销已接入 `server/client/both`，沿用现有路由和调用方凭据格式。原生密码登录支持 `principal_type=username|phone|email`、`password_format=plain`；手机号限中国大陆号码，国家码可省略或为 `86`／`+86`，不会改走短信或注册流程。`modern` 本人资料与音乐会员分别使用独立读取入口。普通歌曲资料及账户音频入口见下文；其他尚未接入账户来源的公开接口仍明确拒绝账户请求。请求、独立核验、条件存储和真实账户验收范围见 [authentication.md](authentication.md#酷我账号与统一-http-登录)。

本人歌单目录支持 `/v1/account/playlists?platform=kuwo`、`/v1/users/kuwo:{uid}/playlists/created`、`/v1/users/kuwo:{uid}/favorites/playlists`，使用所选账户的原生会话，返回 `no-store`。自建与收藏保留各自身份，完整读取后提供统一分页；普通自建及收藏歌单详情和曲目支持显式账户或调用方凭据，并可作为 Uni 导入来源；普通歌单写操作及独立账户媒体授权见下文。范围和读取上限见 [歌单目录说明](authentication.md#酷我本人歌单目录) 及 [自建歌单曲目](authentication.md#酷我本人自建歌单曲目)。

酷我原生账户同时支持当前账户或指定本人 UID 的喜欢歌单与曲目读取（`/v1/account/favorites/playlist`、`/tracks` 及 `/v1/users/kuwo:{uid}/favorites/playlist`、`/tracks`），并支持 Uni `favorite_tracks` 来源。喜欢列表按官方系统类型和真实云 ID 识别，使用默认/显式服务器账户或调用方凭据，响应为 `no-store`；详见 [喜欢歌曲说明](authentication.md#酷我喜欢歌曲与-uni)。


酷我短信登录使用既有 `POST /v1/auth/challenges` 和 `POST /v1/auth/challenges/{transaction_id}/verify`，支持三种凭据归属、中国大陆手机号及 `standard` 原生、`middle` 官方 PC Web 后端。两条流程都必须明确设置 `allow_account_creation=true`；`middle` 还必须先由用户勾选并确认酷我用户协议、隐私政策和儿童隐私政策，然后设置 `accept_platform_policies=true`。后端与许可均绑定到原事务，验证时只提交 5 位数字验证码。参数、60 秒统一重发冷却、事务有效期和并发规则见 [短信认证说明](authentication.md#酷我短信登录与统一-http)。

酷我原生账户支持 `POST /v1/playlists` 创建显式公开的普通空歌单，歌单名沿用上游 UTF-16 加权宽度上限 20；也支持单项/批量 `DELETE /v1/playlists` 删除本人普通自建歌单。写前完整检查所有目标，单次写入后回读确认；未确认的结果标为 `write_outcome=unconfirmed` 且不可自动重试，响应 `no-store`。见 [创建与删除的范围和错误说明](authentication.md#酷我普通歌单创建与删除)。

酷我本人普通自建歌单支持 `PATCH /v1/playlists/kuwo:{pid}` 的名称、简介及标签修改。未提供字段保留，显式空简介／标签可清空；写后核对详情和目录，现有封面及公开／私密状态保持原值。支持默认／指定账户或调用方凭据，响应 `no-store`，未确认写入不得自动重试。见 [歌单资料修改](authentication.md#酷我普通歌单资料修改)。

酷我投稿记录使用 `GET /v1/account/playlist-submissions?platform=kuwo&account=A`，支持默认／指定服务器账户和调用方凭据，以及 `limit=1..100`、非负 `offset`。Provider 为 `account_playlist_submissions`，能力为 `account_playlist_submissions`。结果为独立的投稿记录，`review_status` 区分待审、通过、驳回与未知，当前是否上线的 `published` 保持未知；不混入普通歌单目录或推断媒体权益。两遍完整分页一致后返回窗口，响应及早期错误均为 `no-store`。范围与限制见 [投稿记录](authentication.md#酷我投稿记录与审核状态)。

`DELETE /v1/account/playlist-submissions/kuwo:{pid}?account=personal` 删除所选账户对该歌单的投稿历史记录，要求空请求体。响应分别报告记录移除数量、本人普通歌单是否仍在目录中以及独立观察到的上线状态；不把删除记录当作撤下或普通歌单删除。来源、部分失败与最终验收边界见[删除投稿记录](authentication.md#酷我删除投稿记录)。

酷我显式投稿使用 `POST /v1/playlists/kuwo:{pid}/submission`。JSON `{}` 提交已保存的公开普通歌单，可选 `account` 和 `name`／`description`／`tags`；提供资料字段时先保存并回读，再提交审核。可选非空 `recommendation` 为 1–5 个 Java UTF-16 单元；此分支先独立校验文本，再保存资料和投稿，即使未传资料字段也包含两次业务写入，标题加权长度限制为 7–16。`accepted`、`metadata_updated`、独立观察的 `records`／`published` 分开表达；失败分别报告保存与投稿结果，不自动重试。该接口允许已上线歌单显式编辑后重新投稿，不改变普通 PATCH 的保护。详见 [显式投稿](authentication.md#酷我显式投稿与编辑后重新投稿)。

酷我本人未投稿上线的普通歌单支持 `PUT /v1/playlists/kuwo:{pid}/visibility`，必须指定 `{"visibility":"public"}` 或 `{"visibility":"private"}`，可选 `account`。接口仅改变可见性并回读确认其他资料未变；能力名为 `playlist_visibility_write`，未实现的平台明确拒绝，响应 `no-store`。见 [歌单隐私修改](authentication.md#酷我普通歌单隐私修改)。

酷我对已投稿上线歌单的普通资料／隐私修改及实际新增歌曲会在写入前拒绝，因为官方流程包含再次投稿审核。删除操作若会使云端歌曲条目少于 10 首，同样在写前拒绝以避免隐式下线；其余删除仍须回读核验发布状态未变。投稿接收、审核通过与上线是不同状态，不能互相替代。

酷我本人普通自建歌单的账户详情与曲目读取已包含前后两次完整资料核验，返回已确认的标签。来源摘要可识别仅标签变化，Uni 导入来源扩展包含 `source_tags` 和 `source_metadata_verified`；详情、曲目或目录不一致时不会返回部分成功。该读取不执行写入或投稿下架，仍需最终真实账户验收。

酷我本人普通歌单支持 `POST`／`DELETE /v1/playlists/kuwo:{pid}/tracks`，以及 `kind=track` 的 `/items`。添加仅发送缺失歌曲并保留原有重复项；删除按引用移除全部出现次数。写前后完整读取确认有序内容，不能确认的写入不可自动重试。默认／指定账户或调用方凭据均返回 `no-store`；范围、原生插入顺序、投稿限制和封面变化见 [曲目增删](authentication.md#酷我普通歌单曲目增删)。


酷我喜欢／取消喜欢支持 `PUT`／`DELETE /v1/account/favorites/tracks/kuwo:{rid}`，可使用 `account` 查询参数或调用方凭据。只操作所选账户的真实系统喜欢列表，写前后完整核验；已满足的状态不发送写入，取消喜欢移除全部重复项。返回已确认状态、实际喜欢歌单引用和内容摘要；未确认写入不可自动重试，响应为 `no-store`。见 [喜欢与取消喜欢](authentication.md#酷我喜欢与取消喜欢)。


酷我本人普通歌单及真实喜欢列表支持 `PUT /v1/playlists/kuwo:{pid}/tracks/order`，使用 `refs` 或 `ids` 提供全部歌曲的新顺序，重复项按原出现次数保留。完整写后读取核对实际顺序、歌曲资料及歌单属性；同序请求不发送写入，未确认的写入不可自动重试。支持默认／指定账户与调用方凭据，响应 `no-store`。范围及上限见 [歌曲排序](authentication.md#酷我歌单歌曲排序)。

酷我原生账户支持 `PUT /v1/account/playlists/order`，使用 `refs` 提供全部普通自建歌单的新顺序，不包含喜欢、其他系统或收藏列表。写后通过明确序号核对完整顺序，并保持目录中全部已知资料及非目标系统状态；已满足规范序号时不发送写入。支持默认／指定账户与调用方凭据，响应 `no-store`，未确认写入不可自动重试。见 [自建歌单目录排序](authentication.md#酷我自建歌单目录排序)。

酷我原生账户支持 `PUT`／`DELETE /v1/account/favorites/playlists/kuwo:{pid}` 收藏／取消收藏歌单。支持默认／指定账户与调用方凭据，先后完整读取收藏目录确认目标状态及其他收藏资料和顺序；重复请求已满足状态时不发送写入。响应 `no-store`，发送后未确认的修改不可自动重试。见 [收藏与取消收藏歌单](authentication.md#酷我收藏与取消收藏歌单)。

酷我收藏目录支持独立的 `PUT /v1/account/favorites/playlists/order`，使用全部收藏歌单的 `refs` 或 `ids` 指定目标顺序，能力名为 `playlist_collection_order_write`。该入口与自建目录排序明确区分，支持默认／指定账户和调用方凭据，写后完整核对顺序及资料。重复同序请求不发送写入，未确认修改不可自动重试，响应 `no-store`。见 [收藏歌单目录排序](authentication.md#酷我收藏歌单目录排序)。


酷我本人未投稿上线的普通歌单支持 `PUT /v1/playlists/kuwo:{pid}/cover`，以图片 MIME 和原始图片字节上传，可选 `account`、`filename` 及正方形裁剪参数。图片在本地转换为 700×700 JPEG，上传后保存并完整回读确认；两次写入分别计数，上传成功不等于封面已保存。响应为 `no-store`，未确认的写入不可自动重试。格式、尺寸、裁剪及分阶段错误见 [封面修改](authentication.md#酷我普通歌单封面修改)。


酷我普通账户音频支持既有歌曲 `stream`、`download`、`availability` 及重定向入口，显式使用 `account=default`、服务器别名或调用方凭据。播放与下载独立授权，下载拒绝不会回退到播放地址。支持 AAC 48、MP3 128／320、普通及 Hi-Res FLAC，以及 `master` 对应的独立 `ZPLY` 母带、显式 `spatial` 对应的 `ZPGA201` 至臻全景声、`vinyl` 对应的独立 `VINYL` 黑胶内容和 `dtsx` 对应的独立 DTS:X 压缩媒体交付；未知实际码率保持未知。加密音频通过内容接口交付，独立授权的 128 kbps MP3 试听标注窗口且不可下载。账户响应 `no-store`，真实账户与媒体播放仍待最终验收。质量参数和边界见 [账户音频与下载](authentication.md#酷我普通账户音频与下载)。


## 咪咕账户歌曲与播放

歌曲详情、`type=track` 搜索、歌曲 `stream`／`availability` 及播放重定向支持显式 `account=default`、服务器别名或调用方凭据。详情和搜索在核验账户后读取公共目录，扩展标记 `catalogue_scope=public`，目录音质不代表该账户已获授权。省略账户且没有调用方凭据时仍使用匿名路径，不自动选取已保存的默认账户。

账户播放通过 PC 权益与播放接口独立授权，核对当前歌曲、版权身份、实际格式和试听范围。支持普通 MP3 与 FLAC 音质；显式音质不静默降级，`auto` 仅在明确格式拒绝后尝试同一账户的下一档。试听会保留原曲时间轴的 `trial`，`availability.playable` 只有完整播放获准时才为 `true`。账户下载及下载重定向使用独立原生授权，支持普通完整非加密 MP3／FLAC；先核验 H5 交换所得临时原生身份与选中账户 UID 一致，不以播放 URL 代替下载授权。直接下载 URL／302 仍仅支持已核实的非加密下载子集；`/v1/tracks/migu:{contentId}/download/content` 另支持经独立授权、解密和完整性校验的受限 MG3D MP3／FLAC 内容。MGM、云盘／云端已购的专属下载和未证实格式仍不支持。账户响应 `no-store`，H5／原生账户兼容性、付费权益和实际媒体待最终验收。参数及边界见 [咪咕账户歌曲与播放授权](authentication.md#咪咕账户歌曲与播放授权)。


## 咪咕 MV 目录与播放

MV 搜索使用 `GET /v1/search?platform=migu&type=mv&q=周杰伦`。支持默认相关度、`order=newest`、`order=most_played`，不支持时长或分类过滤；SDK 使用 `SearchKind::Mv` 与 `VideoSearchFilters`。返回 `SearchItem::Video`，资源类型为 MV，`duration_ms` 为毫秒。搜索每个上游物理页 20 条，统一 `limit` 为 1–100，支持跨页 `offset`。

详情及统计为 `GET /v1/videos/migu:{contentId}?type=mv` 和追加 `/stats`；详情批量入口同样可用。`contentId` 为正规正整数，不能替换成歌曲或版权 ID。缺失资源返回 `resource_not_found`；元数据不代表播放权限。公开响应不提供当前用户喜欢或收藏状态。格式列表位于 `video.extensions.catalogue_formats`，只含格式标识、容器及已知体积，不输出目录中的媒体 URL；没有实测宽高时 `resolutions` 保持空数组。

歌手 MV 使用 `GET /v1/artists/migu:{singerId}/videos?type=mv&order=platform_default`。HTTP 省略 `type` 时沿用既有的 `mv` 默认值；建议显式指定。SDK 的 `ArtistVideoListRequest::new` 默认为 `all`，必须改为 `VideoKind::Mv`；`all` 包括其他视频类型，当前不支持。先验证歌手身份，再使用独立 MV 目录，每个物理页 10 条，支持 `limit=1..100` 和跨页 `offset`，不支持游标或其他排序。不会将混入视频彩铃的目录当作 MV。展示播放量如“28.3万”仅保留展示文字，精确计数从 MV 详情统计读取；目录总数未知时保持 `null`。

搜索与歌手 MV 目录仍是无凭据公开读取，拒绝显式账户及调用方凭据。详情与统计支持显式账户或调用方凭据：先核验所选账户 UID，再读取不携带账户凭据的公开元数据，标注 `source_user_id` 与 `catalogue_scope=public`；用户喜欢和收藏状态仍保持未知。未指定账户时不读取已保存默认账户。目录每响应最多 2 MiB，单次累计最多 16 MiB，总期限为 45 秒。类型错误、重复资源、错误后续页、体积超限或后续页失败均不返回部分成功；固定 HTTPS 请求不会直接追随上游给出的分页 URL。

MV 播放使用 `GET /v1/videos/migu:{contentId}/stream?type=mv&resolution=auto`，支持 `/stream/redirect`、`GET/POST /v1/videos/streams` 及 Uni MV 条目播放。`auto`（或数值 `0`，SDK 使用 `resolution=0`）按官方 PC 默认顺序选取目录中首个 PQ、HQ、SQ 格式；这不是最高画质模式，授权拒绝后不会自动换档。显式 `1080` 选择 SQ，保留统一接口省略参数时的 1080 默认值；需要登录或无权益时返回拒绝。其他高度目前不支持。格式选择不等于测量到的像素尺寸，`actual_resolution`、`width`、`height` 保持 `null`；实际格式标识位于 `extensions.format_type`。

播放先重新读取 MV 资源、版权、格式和时长，再请求 PC 播放授权。指定账户时先核验 UID，授权返回的候选会话还需独立核验同 UID；每个网络步骤检查原登录代际，退出、重登录或换号后的迟到结果不会交付。资源元数据和 CDN 请求不携带账户 Cookie/PACM；账户更新按既有凭据归属返回，相关 HTTP 响应使用 `no-store`。不指定账户的播放保持匿名。

仅接受上游实际返回的受信 HTTPS HLS 地址，并绑定 MV 资源 ID。服务端读取最多 256 KiB 的 HLS 文本，检查有限分段、结束标记及总时长；不读取音视频分段或密钥。清单比目录时长短超过 1.5 秒时拒绝作为完整播放结果；`duration_ms` 是清单总时长，目录时长另存于 `extensions.catalogue_duration_ms`，两者可能不同。授权响应最多 1 MiB，含账户核验和清单检查的总期限为 45 秒。批量接受 1–100 个 ID，先校验全部输入，再将整批绑定同一次登录；批内相同 MV 复用一次授权结果，保留原顺序和重复项，总期限仍为 45 秒，任一项失败不返回部分批次。此接口返回播放地址，不提供独立视频下载授权；更多画质与预览分支尚未实现，真实账户权益和实际播放仍待验收。

另有只返回 JSON 的原生 MV 入口 `GET /v1/videos/migu:{contentId}/native-stream?format=auto`，支持匿名、显式 `account` 或调用方凭据；`format` 支持 `auto`、`pq`、`hq`、`sq`。账户模式从所选 PACM 会话取得 H5/app token，经官方 token-validate 核对同一 UID 后，才把已验证的 GlobalToken 用于 Android v1.1 播放授权。每个异步边界检查账户代际；token 仅在内存中使用，不写入响应或媒体 URL。匿名模式使用设备 CE/V005 签名，不发送登录令牌。`auto` 按官方自动分支请求 SQ 并显式允许上游回退；`pq`、`hq`、`sq` 是严格手动档位，不会静默换档。响应保留上游实际格式和 `source_range`：起点含于源时间轴、终点不含；`duration_ms=end_ms-start_ms`。播放器必须先 seek 至 `source_range.start_ms`，否则会播放不完整的源区间。原生授权返回的 HTTP CDN 地址保持原样，仅接受指定的 `freevod.nf.migu.cn:8080` HLS VOD；服务端最多读取 256 KiB 清单，校验清单时长覆盖目录时长，不抓取分段或密钥。因 302 会丢失时间窗语义，该入口不提供重定向模式。所有账户/调用方响应均为 `no-store`；普通 `/stream` 的既有 PC 授权行为不变。真实账户原生权益、实际播放和播放设备 seek 行为仍待最终验收。


## 酷狗榜单期数与历史歌曲

`GET /v1/charts/kugou:chart:{id}/periods` 返回官方已列出的期数，支持 `limit/num`（1–100，默认 10）、`offset` 或从 1 开始的 `page`，后二者不能混用。每项的 `period` 可直接传给 SDK `ChartTrackListRequest.period`；`name`、`year`、`is_current` 和原始显示标签保留其各自含义。列表只代表上游本次返回的期数，不承诺涵盖全部历史；发布时间标签不推导时区。

选择期数使用 `GET /v1/charts/kugou:chart:{id}/tracks?period_kind=id&period_id={id}`。期号必须取自同一榜单的期数列表，酷狗要求规范正整数。`period_id` 与 `period_date` 互斥；`current` 不接受两者，省略期间仍读取当前榜。酷狗不把 `day/week` 日期自动转换成期号；咪咕、QQ 和酷我尚不支持 `id` 期间。SDK 能力分别为 `ChartPeriods`（`chart_periods`）和 `ChartHistoricalTracks`（`chart_historical_tracks`）。

历史读取先确认期号属于该榜单，再核对该期详情，固定榜单和期号读完全部物理页后提供统一窗口。未知期号返回 `ResourceNotFound`；上游退回当前期、身份或数量矛盾返回错误，不返回部分歌曲。显示名次可能跳号，`chart_rank` 保留上游名次，`chart_position` 表示从 0 开始的列表位置；`original_index`、`last_sort`、`last_original_index` 分别保留，不据此补造涨跌。

期数和曲目读取均为公开来源，不接受账户或调用方凭据；每个上游响应限制 1 MiB，整个读取限制 45 秒。期数目录最多 100 个年份组、10000 个期号，歌曲最多 128 个物理页（每页 100 首）。完整读取和固定期号不代表上游提供不可变或原子快照，也不代表播放、下载权益。
