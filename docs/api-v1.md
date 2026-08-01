# TuneWeave HTTP API v1

状态：首版实现契约。平台适配器可以逐步接入，但已实现端点不得改变这里定义的字段语义。

## 基础约定

- 默认监听地址：`127.0.0.1:7832`。
- API 前缀：`/v1`；存活检查 `/healthz` 不带版本前缀。
- 请求与响应使用 UTF-8 JSON。媒体数据不经过 TuneWeave 中转，接口返回带有效期的 URL 与必要请求头。
- 所有平台原始 ID 按字符串处理。公开引用写成 `<platform>:<id>`，例如 `netease:123456`、`qq:0039MnYb0qxYhV`、`bilibili:bvid:BV1xx411c7mD`。
- 时间使用 RFC 3339；时长统一为毫秒；文件大小统一为字节；码率统一为 bit/s。
- 调用方可选提交一次 `X-Request-ID`。值必须为 1–64 个 ASCII 字符，以字母或数字开头，其余只允许字母、数字、`-`、`_`、`.`、`:`；重复、超长或其他字符会在业务处理前返回 `400 invalid_request`。服务端未接受调用方值时生成不可预测的 `tw-` ID。所有响应都在 `X-Request-ID` 头返回最终 ID，JSON 成功/失败包络的 `meta.request_id` 与该头严格相同；无效原值不会进入响应或日志。

### 平台选择

| 参数 | 含义 |
| --- | --- |
| `platform` | 目录或账户所属平台。省略时使用服务配置的默认平台；搜索允许使用 `all` 做多平台聚合。 |
| `account` | 同一平台内由服务器托管的账户别名，默认 `default`。与同平台调用方凭证不能同时显式提供。 |
| `X-TuneWeave-Credential` | 可重复敏感请求头；每项携带一个平台的调用方托管凭证，不进入 URL、普通 JSON、日志或扩展字段。QQ 与网易云当前已实现的公开业务端点、跨平台回退和安全原始扩展均已覆盖。 |
| `playback_platform` | 首选播放来源。它只影响媒体解析，不改变原歌曲引用。 |
| `fallback` | 播放失败时是否继续尝试其他平台，默认 `true`。 |
| `fallback_platforms` | 本次请求的有序回退列表，逗号分隔；省略时使用服务器策略。 |

当路径中的引用已经带平台时，引用平台是内容来源；查询参数 `platform` 不能覆盖它。账户端点没有内容引用，因此通过 `platform` 选择账户平台。

账户别名的作用域是平台，同名的 `netease/personal`、`qq/personal` 与 `bilibili/personal` 是三份独立登录态。兼容模式由服务器保存登录后的必要凭据，重启时按 `platform/account` 恢复；不存在的非默认别名不会回退到 `default`。QQ、网易云与 B 站登录同时支持 [`调用方托管凭证契约`](credential-ownership.md) 的 `credential_mode=server|client|both`：调用方可选择保持服务器托管、完全不落盘而接收凭证，或同时保存和返回同一代际；后续请求可通过专用请求头携带一至多平台凭证。密码、验证码和二维码事务本身始终不持久化；三者的调用方凭证刷新与退出均已接通，刷新失败不会覆盖旧代际，`both` 会在发网前校验调用方与账户别名身份一致。

### 分页

列表端点统一接受：

- `limit`：默认 30，最大 100。
- `offset`：默认 0。

响应的 `meta.pagination`：

```json
{
  "limit": 30,
  "offset": 0,
  "total": 245,
  "next_offset": 30,
  "has_more": true,
  "extensions": {
    "response": { "paidCount": 1 }
  }
}
```

上游只提供页码或游标时，由适配器换算并在内部保存游标。无法可靠获得总数时，`total` 为 `null`。平台额外的分页级字段放在 `extensions`，为空时整个字段不序列化。

## 响应包络

成功响应：

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

失败响应：

```json
{
  "ok": false,
  "error": {
    "code": "authentication_required",
    "message": "该音质需要已登录的 QQ 音乐账户",
    "platform": "qq",
    "retryable": false,
    "details": {}
  },
  "meta": {
    "request_id": "tw-..."
  }
}
```

稳定错误码：

| HTTP | `error.code` | 用途 |
| ---: | --- | --- |
| 400 | `invalid_request` | 参数、引用或请求体无效 |
| 401 | `authentication_required` | 缺少 TuneWeave 或平台登录态 |
| 403 | `permission_denied` | 登录存在但权益不足 |
| 404 | `resource_not_found` | 内容或账户别名不存在 |
| 409 | `conflict` | 重复写入或状态冲突 |
| 422 | `capability_not_supported` | 平台明确不支持该能力 |
| 429 | `rate_limited` | 上游或本服务限流 |
| 502 | `upstream_error` | 上游返回异常或不可解析响应 |
| 503 | `platform_unavailable` | 适配器被禁用或上游暂时不可用 |
| 504 | `upstream_timeout` | 上游请求超时 |

## 统一实体

### Track

```json
{
  "ref": "netease:123456",
  "platform": "netease",
  "id": "123456",
  "name": "反方向的钟",
  "aliases": [],
  "artists": [
    { "ref": "netease:artist-id", "name": "周杰伦" }
  ],
  "album": {
    "ref": "netease:album-id",
    "name": "Jay",
    "cover_url": "https://..."
  },
  "duration_ms": 258000,
  "isrc": null,
  "mv_ref": null,
  "playable": true,
  "available_qualities": ["standard", "higher", "high", "lossless"],
  "extensions": {}
}
```

`extensions` 只保存无法统一但匹配或后续请求必需的平台字段，例如 QQ 的 `media_mid/song_type`、酷狗的 `hash/album_audio_id`、咪咕的 `content_id/copyright_id`。客户端不应依赖未知扩展字段。

### TrackAvailability

```json
{
  "track_ref": "netease:1969519579",
  "playable": true,
  "requested_bitrate": 999000,
  "actual_bitrate": 320000,
  "platform_code": 200,
  "message": "ok",
  "extensions": {
    "response": {
      "code": 200,
      "data": [{ "code": 200, "br": 320000, "fee": 8, "level": "exhigh", "url": null }]
    }
  }
}
```

曲目可用性只检查引用平台在目标码率下是否允许播放，不执行跨平台匹配或回退，也不替代正式流端点。`requested_bitrate` 是调用方目标值，`actual_bitrate` 是平台可提供的真实值；不可播时 `playable=false`、实际码率可为 `null`，这仍是正常成功响应。平台诊断保留在扩展中，但临时播放 URL 会被清除，播放必须继续使用统一 `/stream` 端点。

### Artist

```json
{
  "ref": "netease:6452",
  "platform": "netease",
  "id": "6452",
  "name": "周杰伦",
  "aliases": ["Jay Chou", "周董"],
  "description": "华语流行歌手、词曲作者与制作人……",
  "biography_sections": [
    { "title": "代表作品", "text": "《范特西》……" }
  ],
  "avatar_url": "https://...",
  "cover_url": "https://...",
  "album_count": 44,
  "track_count": 568,
  "mv_count": 9,
  "video_count": 8,
  "identities": ["作曲"],
  "extensions": {}
}
```

稳定歌手详情合并平台的身份详情与传记能力。网易云的 `/artist/detail` 提供名称、图片、身份与作品计数，`/artist/desc` 提供简介和分段传记；QQ 将主页头部与批量歌手详情放在同一上游批包中读取。TuneWeave 将这些响应组合为一个 `Artist`。无法跨平台统一的认证、排行、图片版本、组合资料和专题数据保留在扩展字段，不会因统一映射丢失。

歌手分类目录使用跨平台枚举：`type=all|male|female|group`，`area=all|chinese|western|japanese|korean|other`，`initial` 接受单个英文字母、`hot` 或 `other`。适配器负责转换平台数值；列表上游没有可靠总数时 `total=null`，通过 `has_more/next_offset` 继续翻页。

歌手主页标签使用 `ArtistHomepageTab`，以 `kind=wiki|album|composer|lyricist|producer|arranger|musician|song|video` 区分内容，并显式返回 `page/limit/has_more/next_page`，不把平台原生页码伪装成偏移量。已知音乐资源分别进入强类型 `tracks/albums/videos`；百科等异构介绍块提升为带 `item_type/titles/texts` 的 `introduction`，平台完整动态块仍保留在单项扩展。`tabs` 保存平台提供的标签导航元数据，`need_show/order` 保留页面展示语义。

相似歌手使用 `SimilarArtistList`，明确分离来源 `artist_ref`、`requested_limit` 和推荐 `artists`。平台只提供固定数量快照时不伪造 offset、total 或续页；平台实际过取的数据保存在列表扩展，统一结果仍遵守调用方上限。

### ArtistStats

```json
{
  "artist_ref": "netease:6452",
  "followed": false,
  "follower_count": 18717745,
  "video_counts": [
    { "category": "0", "count": 9, "extensions": {} },
    { "category": "1", "count": 9, "extensions": {} }
  ],
  "online_concert_count": 0,
  "extensions": {}
}
```

歌手动态统计与静态 `Artist` 分开，避免关注态随账户变化时污染可缓存详情。`follower_count` 是公开粉丝总数；`video_counts.category` 是平台提供的类别标识，当平台没有公开类别名称时，TuneWeave 保留原始值而不猜测语义。演出对象、推荐资源及完整动态响应放在 `extensions.response`，粉丝统计的日增量等平台字段放在 `extensions.follow_count_response`。

### User

```json
{
  "ref": "netease:444483977",
  "platform": "netease",
  "id": "444483977",
  "name": "二十点半个小",
  "avatar_url": "https://...",
  "signature": "",
  "followed": false,
  "mutual": false,
  "extensions": {}
}
```

`User` 表示可出现在粉丝、关注、评论或公开资料目录中的平台用户，不等同于选择本地登录态的 `AccountProfile`。地区、性别、认证、背景图、VIP 权益等平台资料保留在扩展字段；关系状态会随所选 `account` 改变。

`UserProfile` 在 `user` 身份外统一提供等级、累计听歌数、歌单/粉丝/关注/动态计数、生日、注册时间、背景图、详细说明及听歌排行公开状态。平台认证、绑定、隐私、地区、性别、VIP 类型、徽章和未来字段保留在 `extensions.response`；原始用户对象同时保留在 `user.extensions.profile`，因此稳定字段不会以丢失平台特有能力为代价。

### CountryCallingCode

国家/地区电话区号按平台显示分组返回：

```json
{
  "label": "常用",
  "entries": [
    {
      "calling_code": "86",
      "region_code": "CN",
      "name": "中国",
      "english_name": "China",
      "extensions": {}
    }
  ],
  "extensions": {}
}
```

`calling_code` 不带前导 `+`，始终按字符串处理；`region_code` 是平台返回的地区代码。分组顺序和平台本地化名称保持不变，条目及目录级平台原文位于扩展字段。

### Comment

```json
{
  "platform": "netease",
  "id": "3160990055",
  "content": "普通评论",
  "author": {
    "ref": "netease:278612322",
    "platform": "netease",
    "id": "278612322",
    "name": "用户",
    "avatar_url": "https://...",
    "signature": "",
    "followed": false,
    "mutual": false,
    "extensions": {}
  },
  "created_at_ms": 1582035919432,
  "created_at_text": "2020-02-18",
  "liked": false,
  "like_count": 5646,
  "parent_comment_id": null,
  "reply_count": 2,
  "replied_to": [],
  "ip_location": "上海",
  "extensions": {}
}
```

`Comment` 统一评论 ID、正文、作者、时间、点赞态、回复关系与公开 IP 地区。评论 ID、父评论 ID 及被回复评论 ID 都是平台不透明字符串；被回复内容使用 `replied_to` 快照，不强制假设对应评论仍能独立读取。会员、装扮、设备、平台标签及完整评论原文保留在扩展字段。

评论反应目录中的单项使用 `CommentReaction`：`kind` 表示 `like/hug` 等稳定反应类型，`user` 是执行反应的统一 `User`，`content` 保留平台生成的可读文案，平台装扮和完整条目原文位于 `extensions`。目录同时返回目标资源、评论 ID、评论作者引用、可用的当前评论快照和分页元数据；反应用户不会被误当作评论作者或普通评论。

`CommentReactionMutationResult` 表示评论反应写入结果，稳定返回目标资源、评论 ID、反应类型、最终 `active` 状态及可选目标用户引用。启用和停用同一反应共享一个结果结构；平台响应及操作名保留在扩展字段，不把“请求已提交”伪装成可读取的反应用户目录。

`CommentReportResult` 表示评论举报提交结果，返回目标资源、评论 ID、调用方提交的理由和 `submitted` 状态。平台完整响应保留在扩展字段；举报是独立的账户写能力，不会被混入评论创建、删除或点赞结果。

`CommentThreadStatsBatch` 表示同平台、同资源类型的一批评论线程统计。`requested_refs` 保留调用方提交顺序；每项 `stats` 同时给出对应 `requested_ref` 和上游返回的 canonical `target`，以及 `liked/like_count/comment_count/comment_count_text/share_count/comment_upgraded/musician_comment_count/latest_liked_users/comments`。平台可能把公开视频哈希归一成内部评论资源 ID，因此两种引用不能被假设恒等；完整单项和批次响应分别保留在扩展字段。

### Video

```json
{
  "ref": "netease:22695250",
  "platform": "netease",
  "id": "22695250",
  "title": "任性 (5525 Live版)",
  "creators": [
    { "ref": "netease:6452", "name": "周杰伦", "avatar_url": "https://..." }
  ],
  "description": "",
  "cover_url": "https://...",
  "duration_ms": 266000,
  "published_at": "2025-02-23",
  "play_count": 100726,
  "subscribed": false,
  "extensions": {}
}
```

`Video` 同时承载音乐平台 MV、站内视频和 B 站视频信息，创作者不被强制假设为音乐歌手。歌手视频目录以 `type=mv|all` 选择范围；`mv` 使用偏移分页，支持游标的平台通过 `cursor` 与分页扩展返回下一游标。平台返回包装记录与实际视频资源时，实际资源 ID、专用标题/封面及非空完整创作者列表优先，空白摘要不会遮住可用回退字段。平台状态、备用封面和内部资源类型保留在条目扩展。

独立 MV 目录通过 `GET /v1/videos` 读取。`catalog=all` 支持地区、类型、排序和偏移分页；`latest` 只支持地区且不伪造参考模块不存在的 offset；`exclusive` 表示平台自制内容，只支持真实存在的偏移分页。统一英文筛选同时兼容网易云中文值，实际目录、筛选、续页能力和完整响应保存在分页扩展。

视频详情端点返回 `VideoDetail`，以 `kind=mv|video` 明确资源类型，`video` 承载统一元数据，`resolutions` 列出平台实际公布的清晰度及可用的宽高、大小和格式。网易云数值 ID 默认推断为 MV，不透明字符串 ID 默认推断为站内视频；QQ 字母数字 VID 默认按 MV 处理。调用方也可通过 `kind`（兼容 `type`）显式指定，避免依赖推断。批量详情使用 `GET/POST /v1/videos/details`，限制 1–100 个同平台、同类型资源，保留输入顺序与重复项；provider 有原生批量协议时只发一次上游请求，否则使用统一逐项默认实现。

`VideoStats` 将公开互动计数与所选账户状态分开：`view_count/danmaku_count/like_count/coin_count/favorite_count/comment_count/share_count` 是平台公开计数，`liked/favorited/coins_contributed` 只表示明确选择的账户。平台或资源不提供的字段保持 `null`，匿名请求不会为了填充账户态而回退到默认账户。`VideoStream` 以 `available` 和可空 `url` 表达取流结果，同时保留备用地址、请求/实际清晰度、大小、时长、业务码、费用和平台原文；平台成功响应没有 URL 时仍返回可检查的成功数据，不会伪造播放地址。`resolution` 兼容 `res`，默认 1080；网易云与 QQ 接受 1–4320，并把上游实际命中的清晰度写入 `actual_resolution`。批量视频流使用 `GET/POST /v1/videos/streams`，保留输入顺序和重复项，但一个请求只接受同一平台，以便支持平台原生批量协议。

已关注歌手的新视频、新曲与混合新作时间线都属于账户资源。它们以毫秒时间戳 `before` 翻页，并在 `meta.pagination.extensions.next_before_ms` 返回下一页起点；`account` 只选择登录态，不改变内容平台。网易云新曲的 `limit` 单位是作品块，一个专辑块可以完整展开为多首 `Track`，因此统一条目数可能大于 `limit`，但不会裁断块内歌曲；混合新作的上游会额外返回一个续页哨兵，统一列表只返回请求数量并以最后一个保留块生成游标，哨兵留到下一页，完整原始响应和裁剪数保存在分页扩展。

混合作品流返回 `ArtistWorkUpdate[]`：`kind=track|video|mixed|unknown` 明确资源类型，已识别内容分别进入 `tracks` 或 `videos`；同一作品块同时含歌曲和视频时使用 `mixed`，两类数组都保留。实际非空资源优先于 `blockType` 提示，空的旧字段别名也不会遮住后续非空数组。`source_type`、作品标题、歌手、封面和发布时间使用稳定字段；尚未识别的平台来源仍返回 `kind=unknown`，完整负载保留在 `extensions.artist_work`，不会静默丢弃。

### Playlist

```json
{
  "ref": "netease:987654",
  "platform": "netease",
  "id": "987654",
  "name": "我的歌单",
  "description": "",
  "cover_url": "https://...",
  "creator": { "ref": "netease:user-id", "name": "Lotus" },
  "track_count": 42,
  "tags": ["华语"],
  "subscribed": false,
  "created_at": null,
  "updated_at": null,
  "extensions": {}
}
```

B 站的公开视频合集与收藏夹共享统一 Playlist 端点，但使用带资源类型的引用避免 ID 冲突：

- `bilibili:season:3629748` 表示公开合集/Season；上游身份同时保留 `season_id` 与所有者 `mid`。
- `bilibili:favorite:2883236382` 表示收藏夹；上游身份同时保留 `media_id/fid` 与所有者 `mid`。

用户创建的 B 站播放列表目录通过 `GET /v1/users/bilibili:{mid}/playlists/created` 读取，支持 `limit`、`offset` 和可选 `account`。结果按收藏夹、空间 Season/Series 的固定顺序合并：收藏夹以 `bilibili:favorite:{media_id}` 为稳定引用并保留 `fid`、默认/私有属性及视频数量，公开视频合集以 `bilibili:season:{season_id}` 返回，公开视频系列以 `bilibili:series:{series_id}` 返回。公开目录默认匿名请求；指定服务器账户或调用方凭证后，只使用该精确登录态读取其可见内容。用户隐藏收藏夹时仍返回公开 Season/Series，并在分页扩展标记该来源隐藏，不会把公开视频合集一并伪装为空。

用户收藏的播放列表目录通过 `GET /v1/users/bilibili:{mid}/playlists/favorite` 读取，分页与账户选择语义相同。B 站 Web 目录同时包含普通收藏夹和收藏的视频合集：前者返回 `bilibili:favorite:{media_id}`，后者返回 `bilibili:season:{season_id}`；失效条目保留 `invalid=true` 供调用方替换来源，不会被静默丢弃。两种目录始终与用户创建目录分开，适合作为后续 Uni Playlist 导入来源。

Season 与收藏夹通过 `GET /v1/playlists/{ref}`、`GET /v1/playlists/{ref}/items` 和 `GET /v1/playlists/{ref}/tracks` 访问；目录中出现的 `series:` 身份保持独立，待系列详情能力接入后才宣告可读取，不会误走 Season 协议。Season 详情只需 `bilibili:season:{season_id}`，provider 使用平台支持的零 owner 参数解析并校验真实 `mid`；收藏夹详情使用 `bilibili:favorite:{media_id}`，同时保留完整 `media_id`、原始 `fid` 和 owner mid。公开内容默认匿名，私有收藏夹由 `account` 或调用方凭证选择精确 B 站登录态，权限不足与不存在不会变成空结果。`/items` 将合集及收藏夹档案保持为强类型 `VideoDetail`，供 Uni Playlist 保存真正的视频项目；`/tracks` 是纯音乐客户端的兼容视图，并以 `normalized_from_video=true` 明示转换。两者都在 `extensions.video_ref`、`extensions.bilibili_playlist_kind`、`extensions.aid` 和可用的 `extensions.bvid` 中保留列表协议实际返回的视频身份；收藏夹额外保留失效状态、分 P 数量和收藏时间。上游分页可能为被过滤内容留下空洞，因此续页使用原始页坐标而非简单返回数累加。列表协议没有的 CID 与清晰度不会猜测，后续由 `/v1/videos/{ref}` 详情链补全。

B 站 UGC 视频详情通过 `GET /v1/videos/bilibili:bvid:{bvid}` 或 `GET /v1/videos/bilibili:aid:{aid}` 读取，`kind` 只能为 `video`。两种输入都会由平台响应交叉校验并规范化为 BVID 引用；EP/SS 保持独立，待 PGC 详情链接入前不会误用 UGC 端点。详情返回标题、简介、可信封面、UP 主、时间、时长、首 CID、分 P 数、分区、公开统计、状态和下载/付费/互动等 rights。`resolutions` 在 playurl 接入前保持空，并以 `resolutions_require_playurl=true` 明示，因为投稿尺寸不是账户当前真正可用的清晰度。

B 站视频统计通过 `GET /v1/videos/{ref}/stats?kind=video` 读取。公开计数来自同一份经过 AID/BVID 交叉校验的视频详情，不使用已经失效的 archive stat 接口；匿名请求的 `liked/favorited/coins_contributed` 保持 `null`。显式选择 B 站账户时才读取账户的近期点赞、投币数和收藏态，任一账户状态请求失败都返回对应错误而不拼接不完整状态。平台点赞查询只能覆盖近期记录，因此响应以 `extensions.liked_state_scope=recent` 明确限制，不能据此断言较早历史中从未点赞。

B 站分 P 目录通过 `GET /v1/videos/{ref}/parts` 读取，接受同一 AID/BVID 身份、`kind/type=video`、`account` 及统一 `limit/offset`。每个 `VideoPart` 以 `bilibili:cid:{cid}` 作为稳定分段引用，同时用规范 BVID `video_ref` 指回父视频，并保留从 1 开始的 `page`、标题、毫秒时长、尺寸、旋转状态和平台来源。详情响应中的全部分 P 会先完成身份、顺序、唯一 CID 和维度校验，再应用本地分页；多 P 视频不会只保留首 P，超过目录尾部返回空页而不是重复首 P。

B 站字幕目录通过 `GET /v1/videos/{ref}/subtitles?part={part_ref}` 读取。`ref` 仍是 AID/BVID 父视频，`part` 必须是同平台分 P 目录返回的 `bilibili:cid:{cid}`，也兼容省略平台前缀的 `cid:{cid}`；跨平台分段引用会在发网前拒绝。响应 `VideoSubtitleList` 保留规范父视频和分段引用、登录要求、是否允许投稿、默认语言，以及每条字幕的稳定引用、语言、名称、格式和锁定状态。B 站的数字 ID、`id_str`、字幕类型及 AI 状态分开保存在扩展中，不凭未知数字枚举猜测“人工/AI”。

字幕正文通过 `GET /v1/videos/{ref}/subtitles/{subtitle_ref}?part={part_ref}` 读取，`subtitle_ref` 可使用目录返回的完整同平台引用或省略平台前缀的 `subtitle:{id}`。provider 会重新获取同一父视频和 CID 的目录并按稳定 ID 选择资源，不接受调用方提交 URL；资源仅允许固定 B 站 HTTPS 字幕 CDN 的旧版 JSON 路径或现行 AI 字幕路径，不跟随重定向、不发送账户 Cookie，并限制响应为 4 MiB。`VideoSubtitleDocument` 将来源语言、类型、版本、样式和每段字幕的毫秒起止时间、原始文本、位置及音乐置信度分开建模。目录中的临时签名 URL及其查询参数不会出现在响应、扩展或错误中。

B 站完整播放清单通过 `GET /v1/videos/{ref}/playback?part={part_ref}` 读取，父视频和分 P 身份规则与字幕相同；可选 `audio_language`（兼容上游名 `cur_language`）请求平台列出的 AI 音轨。响应 `VideoPlaybackManifest` 同时保留可用清晰度说明、DASH 的 AVC/HEVC/AV1 视频轨、普通/杜比/Hi-Res 音轨和兼容 DURL 分段，不会在这一层擅自替调用方选择或混流。每条轨道分开提供 MIME、编码、带宽、尺寸、帧率、SegmentBase、主/备用 URL，清单还保留播放进度、音轨语言目录和所有媒体 URL 中最早的 `deadline`。媒体 URL 仅允许 B 站视频 CDN 的 HTTPS 地址，服务端不代理媒体字节；`headers` 只包含媒体请求必需的 `Referer`，绝不回传账户 Cookie。后续 `/stream`、仅音频和下载端点从这份强类型清单执行选择与回退。

B 站视频的仅音频轨道通过 `GET /v1/videos/{ref}/audio-stream` 选择。`part` 可使用同平台 `bilibili:cid:{cid}` 或省略平台前缀；省略时选择详情目录中的第一 P。`quality` 使用统一音质枚举，`codec` 可指定 `aac/mp4a`、`dolby/eac3`、`flac/lossless` 或平台返回的精确编码，`audio_language/cur_language` 和 `account` 沿用播放清单语义。自动选择按 Hi-Res、杜比、普通音轨顺序查找；普通轨优先以平台稳定 ID `30216/30232/30280` 区分低、标准和较高音质，同时保留实际 `bandwidth`，避免把内容相关的可变平均带宽误当成平台等级。响应分别给出 `requested_quality`、`actual_quality`、`tier`、平台音频 ID 和 `downgraded`：例如请求 `high` 但最高仅有 `30280` 时返回实际 `higher` 并明确标记降级，请求 `master` 回落到 B 站 Hi-Res 时也不会伪装为母带。主/备用签名 URL、最早到期时间、MIME、编码、带宽、时长及必要 `Referer` 原样保留；服务端不下载、混流或代理媒体字节，未匹配错误也不回显签名 URL。

B 站 DASH 视频轨道通过 `GET /v1/videos/{ref}/video-stream` 选择，分 P、账户和默认第一 P 规则与仅音频端点相同。`quality`（也接受 `resolution/res`）支持 `auto`、144P/240P/360P/480P/720P/1080P、720P/1080P 高帧率、1080P 高码率、4K、8K、AI 修复、HDR、Dolby Vision 和 HDR Vivid；常用 `720p60`、`1080p+`、`1080p60`、`4k`、`8k` 等写法会规范为统一枚举。`codec` 只接受 AVC、HEVC、AV1 及其 H.264/H.265 别名，调用方不能提交任意编码字符串。选择器先按平台质量 ID 的明确回落链查找，再在同一质量内优先平台默认编码或调用方指定编码；`codecid=7/12/13` 必须分别与 AVC/HEVC/AV1 profile 一致，冲突视为上游数据错误。响应强类型保留请求/实际质量、动态范围、平台质量 ID、质量说明、编码族与原始 profile、带宽、尺寸、原始帧率、像素宽高比、SAP、SegmentBase、主/备用签名 URL和最早到期时间。无法取得请求档位时返回真实较低档位并设置 `downgraded=true`，不会用请求分辨率覆盖实际轨道；未指定编码时优先平台默认编码，HDR/Dolby/AV1 也不会被压缩成普通 1080P/AVC。服务端仍不代理媒体字节，公共请求头只包含必要 `Referer`。

统一兼容端点 `GET /v1/videos/{ref}/stream` 在 B 站将数值 `resolution=1..4320` 映射到不高于该请求高度的最高平台质量档（请求低于平台最低 144P 时使用最低档），并复用上述视频轨选择器；省略分 P 时选择详情中经过归属校验的首 CID。返回的 `VideoStream` 直接给出实际高度、平台质量码、MIME、编码、签名 URL 和到期时间，并在扩展中保留分 P、请求/实际质量、动态范围、编码族、带宽、帧率、SegmentBase 和降级状态，不能用兼容模型遮蔽 B 站轨道事实。

`audio-download` 与 `video-download` 分别是 `audio-stream` 与 `video-stream` 的下载语义别名，返回相同的原始 DASH 轨道元数据；TuneWeave 不代理、缓存、合并或转码媒体字节。四类轨道端点都提供 `/redirect`，统一兼容 `/stream/redirect` 也可用：成功时仅返回已由 provider 校验的 302 `Location`，并附带 `Cache-Control: private, no-store` 与 `Referrer-Policy: no-referrer`，不会复制 Cookie、调用方凭证或媒体请求头。HTTP 302 无法替调用方在后续 CDN 请求中设置 B 站要求的 `Referer`；遇到要求请求头的资源时必须先读取非跳转端点的脱敏 `headers`，再由客户端自行请求，不能把跳转端点当作带头代理。

### DigitalAlbum

```json
{
  "ref": "netease:120605500",
  "platform": "netease",
  "id": "120605500",
  "name": "冀西南林路行",
  "artists": [{ "ref": "netease:13223", "name": "万能青年旅店" }],
  "description": "发端似乎在2013年\n...",
  "cover_url": "https://...",
  "published_at": "2020-12-21T16:00:01Z",
  "price": { "amount": 22.0, "currency": "CNY" },
  "is_free": false,
  "purchasable": true,
  "purchased": false,
  "sale_count": 0,
  "track_count": null,
  "tags": ["独家", "无损品质收听＆下载"],
  "extensions": {}
}
```

数字专辑是带商品、购买与销量语义的跨平台实体，不与普通 `Album` 混用。网易云公开路由 `/album/detail` 与 `/digitalAlbum/detail` 是同一上游能力的别名，均映射到一个稳定端点；平台特有的展示板、样式、购买须知和活动配置保留在 `extensions`。

### DigitalAlbumChartEntry

```json
{
  "rank": 1,
  "rank_change": 0,
  "product": {
    "ref": "netease:83848829",
    "platform": "netease",
    "id": "83848829",
    "name": "好想爱这个世界啊",
    "artists": [{ "ref": null, "name": "华晨宇" }],
    "description": "",
    "cover_url": "https://...",
    "published_at": null,
    "price": { "amount": 3.0, "currency": "CNY" },
    "is_free": false,
    "purchasable": null,
    "purchased": null,
    "sale_count": 316218,
    "track_count": null,
    "tags": [],
    "extensions": {
      "product": {
        "albumType": 1,
        "rank": 0,
        "salesCertificationSystemLevelCode": "collectionDiamond"
      }
    }
  },
  "extensions": { "upstream_rank": 0, "album_type": 1 }
}
```

榜单统一使用从 1 开始的 `rank`；`rank_change` 表示相对上一统计周期的名次变化。`period` 支持 `daily|week|year|total`，`type` 支持 `album|single`；只有年榜接受可选 `year`，省略时由平台采用当前年份。平台的零基排名、认证等级和商品状态保留在扩展字段中。

### ChartCatalog 与 ArtistChart

```json
{
  "platform": "netease",
  "view": "modern",
  "groups": [{
    "code": "OFFICIAL",
    "name": "官方榜",
    "display_type": "HORIZONTAL",
    "target_url": null,
    "charts": [{
      "ref": "netease:19723756",
      "platform": "netease",
      "id": "19723756",
      "name": "飙升榜",
      "description": "",
      "cover_url": "https://...",
      "update_frequency": "每天更新",
      "updated_at_ms": null,
      "track_count": null,
      "play_count": null,
      "subscribed": null,
      "playable": true,
      "target_kind": "playlist",
      "target_url": null,
      "previews": [{
        "rank": 1,
        "previous_rank": 5,
        "rank_change": 4,
        "track_ref": "netease:3404238777",
        "name": "周旋",
        "byline": "王以太/艾热 AIR",
        "cover_url": "https://...",
        "extensions": {}
      }],
      "extensions": {}
    }],
    "extensions": {}
  }],
  "extensions": {}
}
```

普通音乐榜单目录使用独立 `ChartCatalog`，不再伪装成普通歌单数组。`view=overview|summary|modern` 分别表示平台的榜单介绍、经典内容摘要和新版分组摘要；默认 `summary`。平台只有一套目录时三种值映射同一份最丰富响应，同时仍原样返回请求视图，不伪造额外上游分支。可播放榜单保留可用于 `/v1/charts/{ref}/tracks` 的引用，H5 等非歌单入口保持 `ref=null` 并通过 `target_kind/target_url` 表达；QQ 榜单引用固定为 `qq:chart:<topId>`，避免数值榜单 ID 与普通公开歌单 ID 冲突。预览项只有平台给出真实歌曲 ID 时才返回 `track_ref`；没有证实含义的平台排名类型和值留在扩展，不猜测成 `previous_rank/rank_change`。完整目录、分组、榜单及排名原文均保留在对应 `extensions`。

榜单歌曲使用独立 `ChartTrackListRequest`，与普通歌单读取能力分开。统一 `limit/offset` 支持任意窗口；参考 `num/page` 也可用，其中 page 从 1 开始并换算为 offset，不能和显式 offset 同时提交。`include_tags` 默认 true，并兼容 `tag/tags/withTags`；平台没有标签分支时 provider 可忽略，QQ 会在 true 时发送真实 JSON 布尔、false 时完全省略字段。平台提供的歌曲标签按真实歌曲 ID 附在对应 `Track.extensions.toplist_tags`，完整榜单摘要、标签、附加信息和索引信息保留在分页扩展。

歌手榜使用 `ArtistChart` 快照：`area` 为 `chinese|western|korean|japanese`，`entries` 中每项包含从 1 开始的 `rank`、有效时才存在的 `previous_rank/rank_change`、平台分数 `score` 和完整统一 `Artist`。网易云也接受参考参数 `type=1|2|3|4`；同时传 `area/type` 时必须指向同一区域。

### DimensionChart

```json
{
  "ref": "netease:CITY_SONG_CHART#110000@CITY#",
  "platform": "netease",
  "id": "CITY_SONG_CHART#110000@CITY#",
  "chart_code": "CITY_SONG_CHART",
  "target_id": "110000",
  "target_type": "CITY",
  "name": "北京榜",
  "description": "当前城市所在的云音乐用户，一周内收听的歌曲top内容。",
  "cover_url": "https://...",
  "updated_at_ms": 1784181600000,
  "play_count": 0,
  "share_count": 0,
  "comment_count": 0,
  "supports_comments": false,
  "extensions": { "response": { "code": 200 } }
}
```

维度榜单以 `chart_code + target_id + target_type` 确定一个平台榜单，例如城市榜或城市风格榜。三个值均作为平台不透明字符串处理；`ref` 使用平台返回的稳定榜单 ID。无法跨平台统一的榜单展示配置和完整响应保存在 `extensions`。

### DimensionChartTrackSnapshot

```json
{
  "chart_ref": "netease:CITY_STYLE_SONG_CHART#110000_1020@CITY_STYLE#",
  "chart_code": "CITY_STYLE_SONG_CHART",
  "target_id": "110000_1020",
  "target_type": "CITY_STYLE",
  "entries": [{
    "rank": 1,
    "previous_rank": 1,
    "rank_change": 0,
    "track": {
      "ref": "netease:3399839173",
      "platform": "netease",
      "id": "3399839173",
      "name": "甲乙丙丁 (你我怎么两清)",
      "aliases": [],
      "artists": [],
      "album": null,
      "duration_ms": null,
      "isrc": null,
      "mv_ref": null,
      "playable": true,
      "available_qualities": [],
      "extensions": {}
    },
    "reason": "超73%人播放",
    "reason_id": null,
    "score": null,
    "ratio": null,
    "collected": false,
    "extensions": {}
  }],
  "period_label": null,
  "groups": { "1020": "流行" },
  "extensions": { "response": { "code": 200 } }
}
```

维度榜曲目是平台返回的完整时点快照，不是分页目录，因此响应没有 `meta.pagination`，端点也不接受伪造的 `limit/offset`。`rank` 从 1 开始；有有效上期名次时，`rank_change = previous_rank - rank`，正数表示上升。歌曲主体和独立权益合并为统一 `Track`，平台理由、分组及未标准化字段保留在条目或快照扩展中。

### AlbumStats

```json
{
  "album_ref": "netease:32311",
  "subscribed": false,
  "subscriber_count": 71671,
  "comment_count": 1989,
  "share_count": 9306,
  "like_count": 0,
  "on_sale": false,
  "subscribed_at": null,
  "extensions": {}
}
```

`subscribed` 与 `subscribed_at` 可能依赖所选账户；匿名请求仍返回公开计数。平台额外的活动或游戏关联信息放在 `extensions`。

### SubscriptionResult

```json
{
  "resource_ref": "netease:32311",
  "subscribed": true,
  "extensions": {}
}
```

收藏写入统一返回最终目标引用和状态；平台确认码等附加响应保留在 `extensions`。目标引用本身决定平台，`account` 只选择该平台下的登录态。

### TrackEntitlement

```json
{
  "track_ref": "netease:2058263030",
  "playable": true,
  "downloadable": false,
  "play_bitrate": 320000,
  "download_bitrate": 0,
  "max_play_bitrate": 999000,
  "max_download_bitrate": 999000,
  "play_quality": "high",
  "download_quality": null,
  "available_qualities": ["standard", "higher", "high", "lossless", "hires"],
  "fee": 8,
  "paid": false,
  "extensions": {}
}
```

曲目权益用于批量读取专辑内每首歌当前账户可播放、可下载的最高档位，不等同于实际流地址。平台原始会员、试听与计费字段保留在 `extensions`；真正播放时仍由 Stream 端点执行指定平台与跨平台回退策略。

### SearchMultiMatch

```json
{
  "query": "海阔天空",
  "requested_kind": "track",
  "sections": [
    {
      "section": "artist",
      "kind": "artist",
      "items": [
        {
          "type": "artist",
          "data": {
            "ref": "netease:11127",
            "platform": "netease",
            "id": "11127",
            "name": "Beyond",
            "extensions": {}
          }
        }
      ],
      "extensions": { "order_index": 0, "returned_count": 1 }
    }
  ],
  "extensions": {}
}
```

多重搜索匹配不是普通分页搜索：平台可针对一个关键词同时返回歌手、歌单、MV/视频等多个高置信分区。`sections` 严格保持平台给出的顺序，`section` 保留平台分区名，`kind` 在能映射到统一搜索类型时提供；各资源继续使用统一 `SearchItem {type,data}`。未知分区和暂时无法规范化的条目不会丢弃，而是以 `opaque` 项及完整扩展原文返回。

### GeneralSearchResult

```json
{
  "query": "周杰伦",
  "search_id": "1600000000000-1234567890",
  "page": 1,
  "per_page": 15,
  "next_page": 2,
  "next_page_start": {
    "song": 15,
    "singer": { "index": 1 }
  },
  "sections": [
    {
      "section": "song",
      "kind": "track",
      "estimated_total": 10000,
      "total": 1000,
      "items": [
        {
          "type": "track",
          "data": {
            "ref": "qq:0039MnYb0qxYhV",
            "platform": "qq",
            "id": "0039MnYb0qxYhV",
            "name": "晴天",
            "extensions": {}
          }
        }
      ],
      "more_info": {},
      "extensions": {}
    }
  ],
  "direct": [],
  "related": {
    "estimated_total": 1,
    "total": 1,
    "terms": [
      {
        "display_text": "周杰伦歌曲",
        "query": "周杰伦 热门歌曲",
        "extensions": {}
      }
    ],
    "more_info": {},
    "extensions": {}
  },
  "extensions": {}
}
```

综合搜索保留一个平台搜索会话内的多类结果，不等同于只返回高置信直达项的多重搜索。各分类桶独立提供预估/确切总数、条目和分类续页信息；`search_id`、`next_page` 与完整 `next_page_start` 必须一起用于后续页。`direct` 保存无法稳定归入普通分类的直达对象，`related` 保存展示词与实际查询词，不以展示文案覆盖查询语义。

### RecommendationFeed

```json
{
  "page": 1,
  "direction": "initial",
  "loaded_count": 8,
  "prompt": "",
  "message": "",
  "batch_count": 0,
  "load_mark": 0,
  "shelves": [
    {
      "id": 301,
      "title_template": "今日为你打造",
      "title": "",
      "style": 2,
      "expires_in_seconds": 30,
      "action": null,
      "niches": [
        {
          "id": 203,
          "title_template": "",
          "title": "",
          "style": 10002,
          "sub_style": 0,
          "action": null,
          "cards": [
            {
              "id": "666124541",
              "kind": "track",
              "ref": "qq:666124541",
              "title": "玻璃",
              "subtitle": "Gareth.T",
              "cover_url": "https://...",
              "count": 0,
              "type_code": 200,
              "subtype": 201,
              "style": 10,
              "action": null,
              "extensions": {}
            }
          ],
          "extensions": {}
        }
      ],
      "extensions": {}
    }
  ],
  "next": {
    "page": 2,
    "direction": "forward",
    "loaded_count": 8,
    "seen_ids": ["301", "207"]
  },
  "extensions": {}
}
```

首页推荐流保留平台的楼层→细分组→卡片层级，不压成普通歌曲或歌单列表。已确认身份的歌曲、专辑、歌单和榜单卡片给出统一 `ref`，功能入口及未来未知类型仍保留稳定卡片字段与完整扩展原文。后续请求必须整体使用 `next` 中的页码、方向、累计楼层数和已曝光 ID；同一响应内重复楼层仍按原顺序返回，`seen_ids` 则稳定去重以防重复推荐。

### LocalTrackMatchResult

```json
{
  "md5": "bd708d006912a09d827f02e754cf8e56",
  "matches": [
    {
      "ref": "netease:65766",
      "platform": "netease",
      "id": "65766",
      "name": "富士山下",
      "artists": [{ "ref": "netease:2116", "name": "陈奕迅" }],
      "duration_ms": 258902,
      "extensions": {}
    }
  ],
  "extensions": { "matched_ids": ["bd708d006912a09d827f02e754cf8e56"] }
}
```

本地歌曲匹配使用文件标签、时长和 MD5 在目标平台反查歌曲信息，不等同于播放失败后的跨平台严格匹配。统一输入以毫秒 `duration_ms` 为主，同时兼容参考项目的秒数 `duration/duration_seconds`；无命中是正常成功结果，返回空 `matches`，不会伪造成资源不存在错误。候选始终是完整统一 `Track`，平台原始候选、命中 ID 和完整响应位于扩展字段。

### MembershipSummary

```json
{
  "user_ref": "netease:32953014",
  "level": 7,
  "active": null,
  "annual_count": -1,
  "expires_at": null,
  "icon_url": "https://p5.music.126.net/...png",
  "extensions": {}
}
```

会员摘要只把平台明确给出的值放入稳定字段。公开资料若只有等级、年费次数和图标，则 `active/expires_at` 保持 `null`，不会根据等级猜测当前是否仍在有效期。客户端会员后端明确返回服务器时间和各权益包有效期时，`active` 取最长有效期与服务器时间的比较结果，`expires_at` 取最长有效期；查询当前账户而上游未返回用户 ID 时，`user_ref` 允许为 `null`。平台动态图标、会员种类、各权益包和完整响应保留在扩展中。

### ListeningRightsAdCatalog

```json
{
  "request_uid": "opaque-ad-request-id",
  "ads": [
    {
      "id": "400002_0",
      "request_uid": "opaque-ad-request-id",
      "extensions": {}
    }
  ],
  "message": null,
  "extensions": {}
}
```

广告换听权益目录只稳定提取后续领取流程需要的广告请求 ID；广告创意、下载应用、曝光上下文及未来平台字段保持在每项 `extensions.raw/ext_json`，不会因为统一模型丢失。无投放是正常成功结果，返回空 `ads` 和可空 `request_uid`。

### ListeningRightsGainResult

```json
{
  "request_uid": "opaque-ad-request-id",
  "granted": true,
  "platform_code": 200,
  "message": "granted",
  "extensions": {}
}
```

`granted` 只在平台返回明确布尔值或 0/1 标志时填写；未知枚举或缺失字段保持 `null`，不会把顶层请求成功猜成权益已领取。实际领取请求、平台完整响应以及 `request_uid_source=explicit|ad_catalog|missing` 保留在扩展中。

### AudioRecognition

```json
{
  "matches": [
    {
      "track": {
        "ref": "netease:185809",
        "platform": "netease",
        "id": "185809",
        "name": "晴天",
        "artists": [{ "ref": "netease:6452", "name": "周杰伦" }],
        "extensions": {}
      },
      "start_time_ms": 1500,
      "extensions": { "match": { "score": 0.97 } }
    }
  ],
  "query_id": "4145b90c-aaf0-480c-b933-6e5724ffeeaf",
  "no_match_reason": null,
  "extensions": {}
}
```

音频识别结果与搜索分开建模：一个指纹可能返回多个候选，每个 `track` 都是完整 `Track`，命中位置与置信度属于单次匹配而不是歌曲本身。没有命中仍是成功请求，返回空 `matches`，并尽可能在 `no_match_reason` 保留平台原因码。`fingerprint` 是目标平台识别算法生成的不透明字符串；网易云当前使用 `shazam_v2`，参考实现通常提交 6 秒片段。平台原始匹配项与完整响应保存在扩展字段。

### Banner

```json
{
  "id": "4862548",
  "title": "新歌首发",
  "image_url": "https://p1.music.126.net/banner.jpg",
  "target_ref": "netease:3402163617",
  "target_kind": "track",
  "url": "https://music.163.com/song?id=3402163617",
  "exclusive": false,
  "extensions": {}
}
```

推广横幅的稳定目标类型为 `track`、`album`、`artist`、`playlist`、`video`、`podcast_episode`、`web`、`unknown`。网页活动通常没有资源 ID，因此 `target_ref=null`，仍保留 `url`；播客节目横幅保留节目引用和平台深链，不会被猜成歌曲，未知平台类型也不会被猜测。曝光/点击监测、颜色、广告来源、内嵌歌曲和平台追踪字段完整保留在 `extensions.banner`。

### RadioTaxonomy

```json
{
  "categories": [
    { "id": "1", "name": "音乐台", "extensions": {} }
  ],
  "regions": [
    { "id": "407", "name": "网络台", "extensions": {} }
  ],
  "extensions": {}
}
```

广播与播客目录的分类、地区 ID 都按平台不透明字符串处理，供后续电台列表筛选使用，不假设跨平台数值含义相同。平台新增字段保留在选项或响应级 `extensions` 中。

### RadioStyleCatalog

```json
{
  "sources": [
    {
      "id": 0,
      "styles": [
        {
          "id": "difm:0:1020",
          "name": "New",
          "localized_name": "新晋",
          "description": "",
          "channels": [
            {
              "ref": "netease:difm:0:10505",
              "platform": "netease",
              "id": "difm:0:10505",
              "name": "Deep Progressive House",
              "description": "",
              "cover_url": "https://p1.music.126.net/difm.jpg",
              "category": "New",
              "region": null,
              "stream_url": null,
              "current_program": null,
              "subscribed": null,
              "extensions": {}
            }
          ],
          "extensions": {}
        }
      ],
      "extensions": {}
    }
  ],
  "extensions": {}
}
```

`RadioStyleCatalog` 保留平台的来源、风格和频道三层结构，不把不同来源的分类压平。网易云 DiFM 的来源 `0/1/2` 分别对应电子、古典和爵士；风格 ID 与频道引用都带来源命名空间，因此不同来源即使出现相同数值 ID 也不会碰撞。频道复用统一 `RadioStation`，平台中文名和完整原始字段保留在扩展中。

### RadioPlaybackQueue

```json
{
  "station_ref": "netease:difm:0:10505",
  "items": [
    {
      "ref": "netease:difm-track:0:10505:199222851",
      "platform": "netease",
      "id": "difm-track:0:10505:199222851",
      "station_ref": "netease:difm:0:10505",
      "title": "Green Forest (Dezza & Rylan Taggart Remix)",
      "artist": "Max Freegrant & Slow Fish",
      "cover_url": "https://p1.music.126.net/difm-track.jpg",
      "blur_cover_url": null,
      "stream_url": "https://m7.music.126.net/difm.mp3",
      "duration_ms": 351000,
      "waveform": [0.0003, 0.2434],
      "extensions": {}
    }
  ],
  "total": 1,
  "extensions": {}
}
```

`RadioPlaybackQueue` 用于频道当前可播放队列，不把 DiFM 条目冒充普通平台歌曲。条目引用同时包含来源、频道和平台条目 ID；`station_ref` 保留归属频道，时长统一为毫秒，波形数组完整保序。`stream_url` 是平台返回的临时直链，调用方应及时使用且不能假设永久有效；平台的原始秒级时长、offset 和完整波形仍保留在 `extensions.difm_track`。

### RadioStation

```json
{
  "ref": "netease:362",
  "platform": "netease",
  "id": "362",
  "name": "金山区广播电视台综合广播",
  "description": "",
  "cover_url": "https://p1.music.126.net/radio.jpg",
  "category": null,
  "region": "上海",
  "stream_url": null,
  "current_program": null,
  "subscribed": true,
  "extensions": {}
}
```

`RadioStation` 统一广播频道的名称、封面、分类、地区、当前节目、直播音频地址和账户收藏态。目录接口不提供的详情保持 `null`，不会用猜测值填充；收藏时间、平台来源、房间 ID、评分及完整上游条目保存在 `extensions`。`ref` 与 `id` 仍按平台不透明字符串处理。

### Podcast 与 PodcastEpisode

```json
{
  "ref": "netease:336355127",
  "platform": "netease",
  "id": "336355127",
  "name": "代码时间",
  "description": "...",
  "cover_url": "https://p1.music.126.net/podcast.jpg",
  "creator": {
    "ref": "netease:32953014",
    "name": "主播",
    "avatar_url": "https://p1.music.126.net/avatar.jpg"
  },
  "category": "科技",
  "secondary_category": null,
  "episode_count": 36,
  "subscriber_count": 1000,
  "play_count": 100000,
  "subscribed": false,
  "paid": false,
  "purchased": false,
  "price": null,
  "created_at": "2024-01-01T00:00:00Z",
  "extensions": {}
}
```

`Podcast` 表示可点播的播客/电台节目集合，与提供实时流的 `RadioStation` 严格分开。平台分类、付费、价格及收藏字段只按上游明确值映射；已知价格统一为带币种的 `Money`，没有价格信息时保持 `null`。原始播客对象与完整响应保留在 `extensions`。

播客详情缺省使用公开节目集合后端；`backend=workbench`（也接受 `variant/source` 字段和 `voice/creator` 值）显式选择创作者声音歌单工作台，并通过独立能力 `podcast_workbench_detail` 发现。该后端要求 `account` 指向已登录会话；平台不支持时返回能力不支持，不会静默改用公开详情。网易云工作台的 `voiceListId/radioId`、`voiceCount` 和完整 `creator` 分别进入播客引用、节目数和主播，审核/发布状态等平台字段保留在扩展。

播客榜单返回 `PodcastChartEntry`，将 `rank/previous_rank/score` 与完整 `podcast` 分开；`previous_rank=-1` 保留平台“新上榜”语义。当前网易云支持 `new/hot/paid`：新晋及热门参考接口虽然接收 offset，但实测不会应用，付费榜则没有 offset 参数；统一分页元数据会明确记录 `requested_offset/offset_submitted/offset_applied/continuation_supported`，不会把榜单快照伪装成可续页目录。付费榜的容器语义会明确使 `podcast.paid=true`，稀疏条目中的 `creatorName` 也不会因缺少完整主播对象而丢失。

主播榜单返回 `PodcastCreatorChartEntry`，将 `rank/previous_rank/score/follower_count` 与完整 `creator: User` 分开，当前网易云支持 `newcomer/popular/trending24_hours`。主播 ID 是可继续用于统一用户能力的平台资源引用；认证、直播状态等平台专有字段留在用户及榜单条目扩展中。新人榜虽然参考模块提交 offset，但实测并未应用；热门与 24 小时榜根本没有 offset 参数，统一分页元数据会忠实区分这两种情况。

```json
{
  "ref": "netease:1367665101",
  "platform": "netease",
  "id": "1367665101",
  "podcast_ref": "netease:336355127",
  "name": "一期节目",
  "description": "...",
  "cover_url": "https://p1.music.126.net/episode.jpg",
  "creator": null,
  "audio": {
    "ref": "netease:530692704",
    "platform": "netease",
    "id": "530692704",
    "name": "一期节目",
    "artists": [],
    "extensions": {}
  },
  "duration_ms": 258000,
  "published_at": "2024-01-01T00:00:00Z",
  "serial_number": 42,
  "listener_count": 1234,
  "liked_count": 12,
  "comment_count": 3,
  "share_count": 4,
  "subscribed": false,
  "has_lyrics": true,
  "paid": false,
  "purchased": false,
  "extensions": {}
}
```

`PodcastEpisode.ref` 是节目引用，`podcast_ref` 是所属播客，`audio.ref` 才是后续取流使用的歌曲/音频引用；三者不得互换。网易云节目响应同时给出 `mainTrackId` 与 `mainSong.id` 时，两者必须一致，否则返回上游结构错误，不能静默选取其中之一。节目摘要中的零时长不会遮住完整音频时长，零创建时间也不会遮住有效的计划发布时间。节目原文和完整列表/详情响应继续保留在扩展中。

播客节目列表缺省使用公开目录，并采用 `limit=30`、最大 100；`backend=workbench`（同样接受 `variant/source` 和 `voice/creator` 别名）显式选择创作者声音歌单目录，通过独立能力 `podcast_episode_workbench_list` 发现。工作台目录要求 `account` 指向已登录会话，缺省及最大 `limit` 均为 200，并且不支持 `ascending=true`；它不会因平台不支持或认证失败而静默回退公开目录。网易云固定调用 EAPI `/api/voice/workbench/voices/by/voicelist`，工作台 `voiceId/programId`、`voiceListId/radioId` 与 `songId/trackId` 仍分别映射为节目、所属播客和承载音频引用；审核状态、可见性及未来包装字段保留在节目扩展中。

`GET /v1/account/podcast-episodes` 提供跨平台账户工作台声音查询，不与公开 `/v1/search?type=voice` 混合。它接受 `platform/account/query/display_status/visibility/fee/podcast/limit/offset`；同时兼容参考参数 `name/displayStatus/type/voiceFeeType/voiceListId/radioId`。审核状态完整支持 `auditing/only_self_see/online/schedule_publish/transcode_failed/publishing/failed`，可见性支持 `public/private`，付费筛选支持 `all/free/paid` 及 `-1/0/1`；`podcast` 既可传所选平台的裸 ID，也可传完整资源引用。该端点通过 `podcast_episode_workbench_search` 能力发现，要求已登录账户，缺省和最大 `limit` 均为 200；网易云固定调用 EAPI `/api/voice/workbench/voice/list`，省略的筛选会按参考协议显式提交 `null`，不会擅自替换成筛选值。

`PUT /v1/account/podcasts/{ref}/episodes/order` 通过独立 `podcast_episode_order_write` 能力调整声音在账户声音歌单中的固定序号；路径引用是声音歌单，JSON `episode` 是声音本身，两者必须属于同一平台。`position` 从 1 开始，超出节目数时由上游移动至末尾；`limit/offset` 完整保留参考排序接口用于定位工作台页的控制，缺省分别为 200/0。网易云固定调用 EAPI `/api/voice/workbench/radio/program/trans`，精确提交 `limit/offset/radioId/programId/position`，要求 `account` 选择已登录隔离会话，成功结果保留完整响应。真实匿名协议请求到达上游并返回 `code=400`“只允许操作自己的播客”，统一端点对缺失账户别名返回 401；真实拥有者的成功重排留待使用创作者账户验证。

`DELETE /v1/account/podcast-episodes/{ref}` 删除单条账户声音；`DELETE /v1/account/podcast-episodes` 以 JSON `refs` 或 `ids` 提供有序批量删除。`refs` 接受完整引用数组或逗号字符串，`ids` 接受所选 `platform` 的裸 ID 数组或逗号字符串，两者不能同时出现；输入顺序和重复项原样保留，不擅自去重。该操作通过独立 `podcast_episode_delete_write` 能力发现，网易云固定调用 EAPI `/api/content/voice/delete` 并以逗号拼接的 `ids` 精确复刻参考批量协议。参考文档把该字段误称为 `voiceListId`，但实际服务方法、路径和同协议调用均是声音 ID，因此统一模型不会把它误作删除整个声音歌单。删除要求 `account` 选择已登录隔离会话；使用空凭据目录的真实服务器验证缺失别名在发网前返回 401，破坏性成功分支留待可丢弃的自有声音验收。

`POST /v1/account/podcasts/{ref}/episodes` 接收原始音频请求体并完成账户声音上传，最大 500 MiB；`Content-Type` 是音频类型，查询必须给出 `filename/cover_image_id/category_id/second_category_id/description`，并可给出 `name/privacy/publish_time_ms/auto_publish/auto_publish_text/order_no/composed_songs/account`。所有字段兼容参考 camelCase 名称；布尔值接受 `true/false/1/0`，`order_no` 缺省 1 且最小 1，发布时间缺省 0（立即发布），`composed_songs` 接受逗号分隔的同平台裸歌曲 ID 或完整引用并保留顺序和重复项。音频字节只存在于脱敏请求模型和上传事务中，不进入 JSON、Debug 或扩展。网易云实现完整执行 WeAPI `/api/nos/token/alloc`（`ymusic`）→ 固定 `ymusic.nos-hz.163yun.com` 的 10 MiB NOS 分片上传与 XML 完成 → EAPI `/api/voice/workbench/voice/batch/upload/preCheck` → EAPI `/api/voice/workbench/voice/batch/upload/v2`；两次提交按参考行为使用不同 RFC 4122 v4 `dupkey`，并携带 NOS token 请求头，但 token 不会写入结果或日志。空凭据真实服务器验证完整输入在发网前稳定返回 401；真实上传、发布后详情与播放验收留待创作者账户及可丢弃音频。

`GET /v1/account/podcasts/created` 返回所选登录账户创建的播客/声音歌单，通过 `account_created_podcasts` 能力与订阅库 `/v1/account/library/podcasts` 分开。当前网易云固定以 WeAPI 调用 `/api/social/my/created/voicelist/v1`，只接受参考实现真实支持的 `limit`（缺省 20），不接收或伪造 offset；统一分页因此固定 `offset=0/next_offset=null/has_more=false`，并以 `continuation_supported=false` 明示这是不可续页快照。当前 `data.data` 包装及旧版列表包装均可解包，空的旧列表不会遮蔽后续非空兼容列表；创作者状态与完整包装字段保留在 `Podcast.extensions`，普通登录账户的空创作目录已通过真实请求验证。

节目详情缺省使用普通公开节目后端；`backend=workbench`（也接受 `variant/source` 字段和 `voice/creator` 值）显式选择平台创作者工作台详情。该分支用于平台账户拥有的声音管理数据，要求 `account` 指向已登录会话，并通过独立能力 `podcast_episode_workbench_detail` 发现；平台没有工作台能力时返回能力不支持，不会悄悄回退普通详情。网易云工作台返回的 `voiceId`、`radioId` 与 `songId` 分别映射到节目 `ref`、`podcast_ref` 与 `audio.ref`，审核/发布状态等平台特有字段完整保留在扩展。

节目榜单返回 `PodcastEpisodeChartEntry`，将 `rank/previous_rank/score` 与完整 `episode` 分开；`previous_rank=-1` 是平台明确的新上榜标记，不会丢成 `null`。因此调用方既可展示榜单变化，也可直接使用 `episode.audio.ref` 进入统一取流与跨平台回退链。

节目播放返回 `PodcastEpisodeStream`：顶层 `ref` 仍是节目引用，`audio_ref` 是原平台提供的音频引用，嵌套 `stream` 则是完整 `MediaStream`。跨平台回退成功时，`audio_ref` 不改变，实际命中的资源和平台分别由 `stream.resolved_track`、`stream.resolved_platform` 表达，所有尝试继续位于 `stream.attempts`。`extensions.episode` 保留本次解析所依据的完整节目详情。

```json
{
  "ref": "netease:1367665101",
  "audio_ref": "netease:530692704",
  "stream": {
    "url": "https://.../audio.mp3",
    "origin_track": "netease:530692704",
    "resolved_track": "netease:530692704",
    "resolved_platform": "netease",
    "requested_quality": "standard",
    "actual_quality": "standard",
    "attempts": []
  },
  "extensions": {}
}
```

节目歌词返回 `PodcastEpisodeLyrics`：顶层 `ref` 始终是节目引用，`audio_ref` 与 `lyrics.track_ref` 指向承载声音的音频资源。网易云 `/voice/lyric` 非空分支先返回受限资源 URL，TuneWeave 仅允许网易云媒体域名、拒绝重定向并限制为 16 MiB，再读取完整 JSON 转写；`plain` 提供按句段生成的 LRC，`word_synced` 原样保存含逐词时间轴、说话人和未来字段的 JSON 字符串，`format=netease_voice_json`。上游 `data=null` 是成功但无歌词，此时文本字段保持 `null` 且 `extensions.available=false`。节目详情的 `has_lyrics` 不能替代实际查询：真实声音样本可能标为 `false` 但仍有转写。

```json
{
  "ref": "netease:2058695201",
  "audio_ref": "netease:1336048748",
  "lyrics": {
    "track_ref": "netease:1336048748",
    "plain": "[00:00.000]...",
    "translated": null,
    "romanized": null,
    "word_synced": "{\"duration\":4617380,\"sents\":[...]}",
    "format": "netease_voice_json",
    "contributors": [],
    "extensions": {
      "available": true,
      "sentence_count": 675,
      "word_synced_format": "netease_voice_json"
    }
  },
  "extensions": {}
}
```

### ImageUploadResult

```json
{
  "url": "https://p1.music.126.net/109951168/avatar.jpg",
  "image_id": "109951168000000000",
  "extensions": {}
}
```

图片写入统一返回可访问 URL、平台图片 ID 与无法跨平台统一的上传响应。对象存储 token、账户 Cookie 等临时凭据不得进入结果或日志。网易云头像写入依次申请 NOS 凭据、上传原始图片、提交图片 ID；任何一步失败都不会伪造成功结果。

### Stream

```json
{
  "url": "https://...",
  "backup_urls": [],
  "headers": {
    "Referer": "https://y.qq.com/"
  },
  "expires_at": "2026-07-14T03:30:00Z",
  "format": "flac",
  "codec": "flac",
  "bitrate": 999000,
  "size": null,
  "duration_ms": 258000,
  "requested_quality": "lossless",
  "actual_quality": "lossless",
  "trial": null,
  "origin_track": "netease:123456",
  "resolved_track": "qq:0039MnYb0qxYhV",
  "resolved_platform": "qq",
  "match_score": 0.98,
  "attempts": []
}
```

统一音质枚举：`auto`、`low`、`standard`、`high`、`lossless`、`hires`、`spatial`、`master`。适配器负责映射到平台规格；实际降级时必须在 `actual_quality` 体现。

### Lyrics

```json
{
  "track_ref": "netease:123456",
  "plain": "[00:00.00]...",
  "translated": null,
  "romanized": null,
  "word_synced": null,
  "format": "lrc",
  "contributors": [],
  "extensions": {}
}
```

## 端点

### 服务发现

| 方法 | 端点 | 输入 | `data` |
| --- | --- | --- | --- |
| GET | `/healthz` | 无 | 进程状态、版本、启动时间 |
| GET | `/v1/platforms` | 无 | 已注册平台、启用状态、默认顺序 |
| GET | `/v1/capabilities` | `platform?` | 每个平台当前真正可用的能力，不包含仅计划能力 |

### 目录

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| GET | `/v1/search` | `q`（也接受 `keywords`）、`type?`（也接受 `kind`）、`variant?`、`platform?`、`account?`、`search_id?`（也接受 `searchid`）、`highlight?`、`selectors?`（URL 编码的 `[{id,name,type}]` JSON 数组）、视频专用 `order?`、`duration?`、`category_id?`（也接受 `tids`）、分页 | 带 `type/data` 判别字段的统一 `SearchItem[]`；选择项、视频筛选与平台返回的二维 selector 目录位于分页扩展，未知查询字段会被拒绝 |
| GET | `/v1/search/general` | `q`（也接受 `keywords/keyword`）、`platform?`、`account?`、`page?`、`limit?`（也接受 `num`）、`search_id?`（也接受 `searchid`）、`page_start?`（也接受 `cursor`，URL 编码 JSON 对象）、`highlight?` | `GeneralSearchResult`；保留搜索会话、多分类桶、直达结果、相关词和完整多字段续页游标 |
| GET | `/v1/search/default` | `platform?`、`account?` | `SearchDefaultKeyword`；实际查询词、展示文案、搜索类型与可选图片 |
| GET | `/v1/search/trending` | `platform?`、`account?`、`detail=brief|full` | `SearchTrendingList`；有序热搜关键词及可用的分数、说明和图标 |
| GET | `/v1/search/suggestions` | `q`（也接受 `keywords/keyword`）、`client=web|mobile|pc`、`platform?`、`account?` | `SearchSuggestionList`；关键词建议、可选统一资源及独立推荐项 |
| GET | `/v1/search/multimatch` | `q`（也接受 `keywords/keyword`）、`kind?`（也接受 `type`）、`platform?`、`account?` | `SearchMultiMatch`；按平台顺序分组的跨类型高置信匹配资源 |
| GET | `/v1/search/match` | 参考查询 `title/album/artist/duration/md5`，另支持 `duration_ms/duration_seconds`、`platform?`、`account?` | `LocalTrackMatchResult`；兼容参考项目调用形态 |
| POST | `/v1/search/match` | JSON `{title?, album?, artist?, duration_ms? | duration_seconds? | duration?, md5, platform?, account?}` | `LocalTrackMatchResult`；统一结构化调用形态 |
| GET | `/v1/banners` | `platform?`、`account?`、`catalog=music|podcast`、`client=pc|android|iphone|ipad` | `Banner[]`；省略目录时使用音乐横幅，省略客户端时使用 PC；不支持客户端分支的目录会拒绝非默认选择 |
| GET | `/v1/radio/taxonomy` | `platform?`、`account?` | `RadioTaxonomy`；广播/播客目录可用的分类与地区 |
| GET | `/v1/radio/styles` | `platform?`、`account?`、`sources?` | `RadioStyleCatalog`；来源列表接受参考 JSON 数组（如 `[0,1,2]`）、逗号列表或单值，网易云默认 `0`，保留来源→风格→频道层级 |
| GET | `/v1/radio/stations` | `platform?`、`account?`、`category_id?`、`region_id?`、`limit?`、`last_id?`、`score?`、`offset?` | `RadioStation[]`；游标下一页信息位于分页扩展 `next_cursor={id,score}` |
| GET | `/v1/radio/stations/{ref}` | `account?` | `RadioStation`；当前节目与直播音频地址按上游实时响应返回，未提供的收藏态保持 `null` |
| GET | `/v1/radio/stations/{ref}/tracks` | `account?`、`limit?` | `RadioPlaybackQueue`；频道当前直接可播放的队列、时长、封面和完整波形，默认 5 条 |
| GET | `/v1/podcasts/categories` | `platform?`、`account?`、`kind?=all|non_hot` | `PodcastTaxonomy`；完整或非热门分类的稳定 ID、名称、可选图标及完整平台扩展 |
| GET | `/v1/podcasts/category-recommendations` | `platform?`、`account?` | `PodcastCategoryRecommendations`；按分类分组的推荐播客，每组保留分类与完整 `Podcast[]` |
| GET | `/v1/podcasts` | `platform?`、`account?`、`catalog`、`category_id?`（也接受 `categoryId`）、`limit?`、`offset?`、`page?` | `Podcast[]`；统一目录类型由 `catalog` 选择，当前网易云支持 `featured`、`hot`、`category_featured`、`category_hot`、`personalized`、`today_preferred` 与 `paid` |
| GET | `/v1/podcasts/{ref}` | `account?`、`backend?=default|workbench`（也接受 `variant/source`） | `Podcast`；引用决定平台，工作台后端要求该平台登录账户 |
| GET | `/v1/podcasts/{ref}/episodes` | `account?`、`limit?`、`offset?`、`ascending?`（也接受 `asc`） | `PodcastEpisode[]`；默认每页 30 条并按最新优先，节目、所属播客和音频引用分离 |
| GET | `/v1/episodes` | `platform?`、`account?`、`catalog`、`limit?`、`offset?` | `PodcastEpisodeChartEntry[]`；当前网易云支持 `popular` 与 `trending24_hours` 节目榜 |
| GET | `/v1/episodes/{ref}` | `account?`、`backend?=default|workbench`（也接受 `variant/source`） | `PodcastEpisode`；`audio.ref` 是节目取流所需的独立音频资源引用，工作台后端要求登录账户 |
| GET | `/v1/episodes/{ref}/lyrics` | `account?` | `PodcastEpisodeLyrics`；节目与音频引用分离，句段 LRC 和平台逐词转写均完整返回 |
| PUT | `/v1/account/podcasts/{ref}/episodes/order` | JSON `{episode, position?=1, limit?=200, offset?=0, account?}` | `PodcastEpisodeOrderResult`；调整账户声音歌单中的声音序号，`episode` 兼容完整引用及 `episode_ref/programId/id` 别名 |
| POST | `/v1/account/podcasts/{ref}/episodes` | 原始音频请求体、`Content-Type`；查询含必选 `filename/cover_image_id/category_id/second_category_id/description` 和可选发布、隐私、排序、包含歌曲及 `account` 参数 | `PodcastEpisodeUploadResult`；完成令牌、NOS 分片、预检查与正式提交的完整上传事务 |
| DELETE | `/v1/account/podcast-episodes/{ref}` | `account?` | `PodcastEpisodeDeleteResult`；删除单条账户声音，引用决定平台 |
| DELETE | `/v1/account/podcast-episodes` | JSON `{refs 或 ids, platform?, account?}`；兼容 `episodeRefs/programIds/voiceIds` | `PodcastEpisodeDeleteResult`；有序批量删除账户声音，保留重复项和完整响应 |
| GET | `/v1/account/podcast-episodes` | `platform?`、`account?`、工作台筛选与分页 | 登录账户创作者工作台中的 `PodcastEpisode[]` |
| GET | `/v1/account/podcasts/created` | `platform?`、`account?`、`limit?` | 登录账户创建的 `Podcast[]` 不可续页快照 |
| GET | `/v1/tracks` | `refs` 或 `ids`（逗号分隔）、`platform?`（仅配合 `ids`）、`account?`、`song_type/type?` | 有序 `Track[]` 批量详情；共享歌曲类型应用到每项，保留重复项且一个批次只调用一个平台 |
| POST | `/v1/tracks` | 兼容 `{refs 或 ids, platform?, account?, song_type/type?}`；或 `{items/query_info:[{ref|id|mid, song_type/type?}], platform?, account?}` | 强类型逐项批量详情；`id` 是无符号数字 ID，`mid` 是平台 MID，顺序和重复项不丢失 |
| GET | `/v1/tracks/{ref}` | `account?` | `Track`；QQ 数字 ID/MID 分别走 Web 富详情分支，发行公司、流派、简介、语言、发布时间、额外字段和完整子响应位于扩展 |
| GET | `/v1/tracks/{ref}/similar` | `limit?`（也接受 `limit_per_section/limitPerSection`，默认 15）、`account?` | `SimilarTrackList`；按分区保留直接相似和相同听众歌曲，每区本地上限 1–100，不伪造分页 |
| GET | `/v1/tracks/{ref}/labels` | `account?` | `TrackLabelList`；保留零 ID、空展示字段、多行文本、图标、动作及平台原生类型/分类，不猜测未知 taxonomy |
| GET | `/v1/tracks/{ref}/related-playlists` | `previous_ids?`（也接受 `last/vecPlaylist/vec_playlist/cursor`，JSON 数组或逗号列表）、`account?` | `RelatedPlaylistList`；分离直接/听众分组，`next_ids` 是平台真实换批游标 |
| GET | `/v1/tracks/{ref}/related-videos` | `previous_id?`（也接受 `lastmvid/last_mvid/cursor`）、`account?` | `RelatedVideoList`；相关 MV 使用 VID 作为资源身份、数字 `mvid` 作为真实换批游标 |
| GET | `/v1/tracks/{ref}/versions` | `account?` | `TrackVersionList`；同曲其他版本的有序完整歌曲列表，不伪装成相似推荐或搜索结果 |
| GET | `/v1/tracks/{ref}/credits` | `account?` | `TrackCredits`；按平台原生角色分组返回制作班底、艺人身份、头像、动作与关注态 |
| GET | `/v1/tracks/{ref}/sheet-music/availability` | `account?` | `SheetMusicAvailability`；总可用性与 AI、附加目录、六线谱、标准谱、外部目录五个独立标志 |
| GET | `/v1/tracks/{ref}/sheet-music` | `source=user|ai|external`（默认 `user`，也接受 `type/ttype` 与 `0/1/2`）、`account?` | `SheetMusicList`；三种真实来源的图片谱或文件谱、完整元数据与分类计数 |
| GET | `/v1/tracks/{ref}/favorite-count` | `account?` | `TrackFavoriteCount`；收藏人数数值与平台展示文案分离 |
| GET | `/v1/tracks/favorite-counts` | `refs` 或 `ids + platform?`、`account?` | `TrackFavoriteCount[]`；1–100 项、同平台、保序并保留重复项 |
| POST | `/v1/tracks/favorite-counts` | JSON `{refs?|ids?, platform?, account?}`；`ids` 接受字符串、数字或数组 | `TrackFavoriteCount[]`；与 GET 共用平台原生批量能力 |
| GET | `/v1/tracks/{ref}/availability` | `account?`、`bitrate?`（默认 999000，也接受 `br`） | `TrackAvailability`；不可播仍返回成功包络与 `playable=false` |
| GET | `/v1/albums` | `platform?`、`account?`、`catalog=new|newest`、`area?`、分页 | `Album[]`；QQ 只支持真实 `catalog=new`，地区见下文 |
| GET | `/v1/albums/{ref}` | `account?` | `Album`；QQ 数字 ID 和 MID 共用同一端点并返回平台规范 MID 身份 |
| GET | `/v1/albums/{ref}/tracks` | 分页、`account?` | `Track[]`；QQ 数字 ID/MID 均支持任意 `offset`，并返回上游真实总数与下一偏移量 |
| GET | `/v1/albums/{ref}/track-entitlements` | 分页、`account?` | `TrackEntitlement[]` |
| GET | `/v1/albums/{ref}/stats` | `account?` | `AlbumStats` |
| GET | `/v1/digital-albums` | `platform?`、`account?`、`catalog=latest|style`、`area?`、`type?`、分页 | `DigitalAlbum[]`；上游不返回可靠总数时 `total=null` |
| GET | `/v1/digital-albums/{ref}` | `account?` | `DigitalAlbum` |
| GET | `/v1/charts` | `platform?`、`account?`、`view=overview|summary|modern`（也接受 `catalog` 及网易模块名别名） | `ChartCatalog`；默认经典内容摘要 |
| GET | `/v1/charts/podcasts` | `platform?`、`account?`、`kind?=new|hot|paid`（默认 `new`，也接受 `type`）、`limit?`、`offset?` | `PodcastChartEntry[]`；排名包装与完整播客分离，不伪造上游实际不支持的续页 |
| GET | `/v1/charts/podcast-creators` | `platform?`、`account?`、`kind?=newcomer|popular|trending24_hours`（默认 `newcomer`，也接受 `type/new/hot/hours/24h`）、`limit?`、`offset?` | `PodcastCreatorChartEntry[]`；排名、粉丝数与完整用户身份分离，不伪造榜单续页 |
| GET | `/v1/charts/artists` | `platform?`、`account?`、`area=chinese|western|korean|japanese`（也接受 `type=1|2|3|4`） | 完整 `ArtistChart` 快照 |
| GET | `/v1/charts/{ref}/tracks` | `limit/num?`、`offset?` 或 `page?`、`include_tags/tag/tags/withTags?`、`account?` | `Track[]`；引用来自可播放榜单项，默认 10 条并包含平台歌曲标签 |
| GET | `/v1/charts/digital-albums` | `platform?`、`account?`、`period=daily|week|year|total`、`type=album|single`、`year?`、分页 | `DigitalAlbumChartEntry[]` |
| GET | `/v1/charts/dimensions/{chart_code}` | `target_id`、`target_type`、`platform?`、`account?` | `DimensionChart`；也接受参考字段 `targetId/targetType` |
| GET | `/v1/charts/dimensions/{chart_code}/tracks` | `target_id`、`target_type`、`platform?`、`account?` | 完整 `DimensionChartTrackSnapshot`；无分页元数据 |
| GET | `/v1/artists` | `platform?`、`account?`、`type`、`area`、`genre`、`initial`、分页 | `Artist[]`；分类歌手目录 |
| GET | `/v1/artists/catalog` | `platform?`、`account?`、`type`、`area`、`genre` | `ArtistCatalog`；热门、完整快照与可选筛选标签分离 |
| GET/POST | `/v1/artists/details` | `refs`，或 `ids/mids + platform?`；`account?`；POST 接受字符串或数组 | 批量 `Artist[]`；限制同平台 1–100 项，保留输入顺序与重复项；QQ 使用原生批量详情协议 |
| GET | `/v1/artists/{ref}` | `account?` | `Artist`；身份详情与分段传记，平台原始附加信息保留在扩展字段；QQ 使用歌手 MID 并保留主页计数及头图结构 |
| GET | `/v1/artists/{ref}/tabs/{tab}` | `page?`、`limit?`（兼容 `num/page_size`）、`account?` | `ArtistHomepageTab`；`tab` 接受 `wiki|album|composer|lyricist|producer|arranger|musician|song|video` 及平台常用别名，音乐资源强类型返回，异构介绍保留稳定摘要与完整扩展 |
| GET | `/v1/artists/{ref}/similar` | `limit?`（兼容 `number/num`，默认 10）、`account?` | `SimilarArtistList`；保留来源、请求数量与推荐顺序，不伪造分页 |
| GET | `/v1/artists/{ref}/overview` | `account?` | `ArtistOverview`；歌手摘要、精选 `Track[]` 与是否仍有更多曲目 |
| GET | `/v1/artists/{ref}/stats` | `account?` | `ArtistStats`；关注态、视频分类计数与在线演出计数 |
| GET | `/v1/artists/{ref}/tracks` | `order=hot|time`、分页、`account?` | `Track[]`；默认按热度排序，完整平台曲目字段保留在单项扩展；QQ 上游只提供 hot，time 会明确拒绝 |
| GET | `/v1/artists/{ref}/top-tracks` | `account?` | 热门 `Track[]` 固定快照；不接受伪分页，`has_more=false` |
| GET | `/v1/artists/{ref}/albums` | 分页、`account?` | `Album[]`；QQ 支持精确任意偏移，歌手级上游信息保留在分页扩展 |
| GET | `/v1/artists/{ref}/fans` | 分页、`account?` | `User[]`；上游无可靠总数时 `total=null` |
| GET | `/v1/artists/{ref}/videos` | `type=mv|all`、分页、`cursor?`、`order?`、`account?` | `Video[]`；QQ 支持 MV/精确任意偏移和 hot 排序，不支持游标 |
| GET | `/v1/videos` | `platform?`、`account?`、`catalog=all|latest|exclusive|timeline_all|timeline_recommended|group`、`area?`、`type?`、`order?`、`group_id?`、分页 | `Video[]`；MV 与站内视频目录按后端真实能力约束筛选及续页 |
| GET | `/v1/videos/taxonomy` | `platform?`、`account?`、`kind/type=categories|groups`、分页 | `VideoCatalogOption[]`；视频分类或完整标签目录 |
| PUT/DELETE | `/v1/account/library/videos/{ref}` | `account?`、`kind/type?=mv|video` | `SubscriptionResult`；收藏或取消收藏视频资源，平台只支持其中一种资源时明确拒绝其余类型 |
| GET | `/v1/account/library/videos` | `platform?`、`account?`、分页 | 已收藏 `Video[]`；MV 与普通视频按平台真实返回共同映射，来源类型及完整条目保留在扩展 |
| GET | `/v1/account/dislikes` | `platform?`、`account?`、`kind/type?=track|artist|style`、`page?`、`cursor/last_id?`；QQ 兼容 `cmd=3|2|4` 与 `lastid` | `AccountDislikeList`；歌曲、歌手和风格三类不喜欢目录共用强类型条目，返回下一页、末项游标及平台分页元数据 |
| POST | `/v1/account/dislikes` | JSON `{platform?, account?, kind|type|id_type, ids|values}`；类别接受 `track|artist|style` 或 `1|2|3`，ID 接受单项、数组或逗号列表 | `AccountDislikeMutationResult`；批量添加不喜欢内容，顺序和重复项原样保留 |
| DELETE | `/v1/account/dislikes` | 与 POST 相同的 JSON 批量契约 | `AccountDislikeMutationResult`；取消单项或批量不喜欢内容，空批次明确拒绝 |
| DELETE | `/v1/account/dislikes/tracks` | `platform?`、`account?` | `AccountDislikeMutationResult`；以独立两阶段事务清空全部歌曲不喜欢内容，不影响歌手或风格 |
| GET | `/v1/playlists/{ref}` | `account?` | `Playlist`；`uni:` 复用同一实体，混合项目数位于 `extensions.uni_item_count`，不伪装成纯歌曲数 |
| GET | `/v1/playlists/{ref}/items` | 分页、`account?` | `PlaylistPlayableEntry[]`；统一返回 `track/mv/video/podcast_episode/radio_station`、资源引用、位置与紧凑快照；Uni 项提供稳定 `item_id`，外部只读项目为 `null` |
| GET | `/v1/playlists/{ref}/tracks` | 分页、`account?` | `Track[]`；混合 Uni 歌单先过滤非歌曲再计算真实分页，B 站合集/收藏夹视频按可播放音频内容归一并保留 `video_ref` |
| GET | `/v1/playlists/{ref}/items/{item_id}/stream` | 音质/后端/码率、`playback_platform?`、`fallback?`、`fallback_platforms?`、`unblock?`、`source?`、`account?`、`accounts?`、视频 `resolution?` | `UniPlaylistItemStream`；稳定项目身份、原始资源和统一 `MediaStream` 分离；B 站视频默认原生音轨，显式 `resolution` 选择视频轨 |
| GET | `/v1/playlists/{ref}/items/{item_id}/stream/redirect` | 同上 | 成功解析真实 URL 后返回 302 |
| GET | `/v1/resources/{type}/{ref}/comments` | `account?`、`view?`、`sort?`、评论分页参数 | `target/comments/hot_comments/top_comments/current_comment/extensions`；统一评论目录，分页位于 `meta.pagination` |
| GET | `/v1/resources/{type}/comments/stats` | `platform?`、`ids?`、`account?` | `CommentThreadStatsBatch`；同类型资源的批量评论、分享、点赞及最新条目统计 |
| GET | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `target_user_ref`、`account?`、`limit/page/cursor/id_cursor?` | `target/comment_id/target_user_ref/kind/reactions/current_comment/extensions`；评论反应用户目录 |
| PUT | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；启用评论反应 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；停用评论反应 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/reports` | 查询参数 `account?`；JSON `{reason}` | `CommentReportResult`；举报评论 |
| GET | `/v1/users/{ref}` | `account?`、`backend?=modern|legacy`（也接受 `variant/source`） | 指定用户的完整 `UserProfile`；引用决定平台；QQ 仅支持 modern 且要求查看者账户 |
| GET | `/v1/users/{ref}/music-gene` | `account?` | `UserMusicGene`；QQ 音乐基因、听歌报告、个性维度与平台展示顺序，公开用户可匿名读取 |
| GET | `/v1/users/{ref}/favorites/tracks` | 分页、`account?` | 指定用户公开引用下的 `Track[]`；需要平台登录态时由 `account` 选择 |
| GET | `/v1/users/{ref}/playlists/created` | 分页、`account?` | 指定用户创建的 `Playlist[]`；平台用户引用的 ID 语义由对应 provider 校验 |
| GET | `/v1/users/{ref}/favorites/playlists` | 分页、`account?` | 指定用户收藏的外部 `Playlist[]`；需要平台登录态时由 `account` 选择 |
| GET | `/v1/users/{ref}/favorites/albums` | 分页、`account?` | 指定用户收藏的 `Album[]`；目标用户与可选查看者账户分离 |
| GET | `/v1/users/{ref}/favorites/videos` | 分页、`account?` | 指定用户收藏的 `Video[]`；目标用户与查看者账户分离，平台要求登录时 `account` 必填 |
| GET | `/v1/users/{ref}/following/artists` | 分页、`account?` | 指定用户关注的 `Artist[]`；目标用户与查看者账户分离，平台要求登录时 `account` 必填 |
| GET | `/v1/users/{ref}/membership` | `account?`、`backend=front|client` | 指定用户的 `MembershipSummary`；引用决定平台，客户端后端要求登录 |
| GET | `/v1/users/{ref}/history` | `period=all_time|week`、分页、`account?` | 指定用户的 `PlaybackHistoryEntry[]` |
| GET | `/v1/recommendations/feed` | `platform?`、`account?`、`page?`、`direction?=initial|forward`、`loaded_count?`（也接受 `s_num/snum`）、`seen_ids?`（也接受 `v_cache/cache`，JSON 数组或逗号列表） | `RecommendationFeed`；楼层化推荐卡片及完整多字段防重复续页状态 |
| GET | `/v1/recommendations/tracks` | `platform?`、`account?`、`source?=daily|personalized|new_releases|radar`、`refresh?`、`area_id?`、分页 | `Track[]`；推荐理由、地区目录、标签和首页包装保存在扩展 |
| GET | `/v1/recommendations/playlists` | `platform?`、`account?`、`source?=daily|personalized`、分页 | `Playlist[]` |
| GET | `/v1/recommendations/videos` | `platform?`、`account?`、`kind=mv|exclusive`、`view=featured|catalog`、分页 | `Video[]`；`exclusive/catalog` 是独家放送真实分页列表 |
| GET | `/v1/recommendations/podcast-episodes` | `platform?`、`account?`、`source?=personalized|category`、`category_id?`（也接受 `categoryId/cateId/type`）、`limit?`、`offset?` | `PodcastEpisode[]`；个性化固定快照或可分类、可偏移的推荐节目目录 |
| GET | `/v1/recommendations/personal-fm` | `platform?`、`account?`、`backend?=classic|mode`、`mode?`、`sub_mode?`、`limit?` | `Track[]` 私人 FM 当前队列快照；不伪造续页 |
| POST | `/v1/recommendations/tracks/{ref}/dislike` | `account?` | `RecommendationDislikeResult`；向所选平台账户提交推荐跳过/不喜欢反馈 |
| GET | `/v1/listening-rights/ads` | `platform?`、`account?`、`type_ids?` | `ListeningRightsAdCatalog`；取得广告换听目录及后续领取所需请求 ID |
| GET | `/v1/listening-rights/gains` | `platform?`、`account?` 及参考 `reqUid/creativeType/exposureTime/clickTime/rightsGainMethod/rightsGainDuration/extraRightsGainMethod/extraRightsGainDuration/nextRightsGainDuration/source/rightsExtJson/appInfo/installed/type_ids` | `ListeningRightsGainResult`；参考查询形态领取广告换听权益 |
| POST | `/v1/listening-rights/gains` | `platform?`、`account?`；JSON 使用上述字段的 snake_case 或 camelCase 名称，`type_ids` 为字符串数组 | `ListeningRightsGainResult`；统一 JSON 形态领取广告换听权益 |
| GET | `/v1/account/profile` | `platform?`、`account?`、`backend?=modern|legacy` | 所选登录账户的完整 `UserProfile`；先从隔离会话解析用户 ID，缺少登录态时返回 401 |
| GET | `/v1/account/membership` | `platform?`、`account?`、`backend=front|client` | 当前登录账户的 `MembershipSummary`；客户端后端缺少登录态时返回 401 |

搜索类型缺省为 `track`，既接受统一名称，也接受网易云参考数字：`track|song|1`、`album|10`、`artist|100`、`playlist|1000`、`user|1002`、`mv|1004`、`lyric|lyrics|1006`、`podcast|dj|dj_radio|1009`、`radio_station|radio|broadcast`、`video|1014`、`mixed|complex|1018`、`voice|2000`，QQ 另支持统一名称 `ringtone|ring`；数字 `10` 继续表示跨平台既有的专辑搜索，不与 QQ 内部彩铃代码混淆。`podcast` 表示可点播的播客目录，`radio_station` 表示直播广播频道；两者不会因为平台字段名含 `radio` 而混为同一实体，平台没有对应搜索时会明确返回能力不支持。`variant` 支持 `default|legacy|cloud`，也兼容 `backend` 字段以及 `search/cloudsearch/auto` 值；缺省时由 provider 使用推荐后端。网易云缺省播客搜索精确对应参考 `/voicelist/search`，使用 EAPI `/api/search/voicelist/get`；`legacy` 精确对应参考 `/search`：普通类型使用 `/api/search/get`，声音使用独立 `/api/search/voice/get`；`cloud` 对应 `/cloudsearch`。每一项统一序列化为 `{type,data}`；歌曲、专辑、歌手、歌单、用户、MV/视频、播客及广播电台使用对应统一实体，其中 MV 与视频均为 `video`，歌词和彩铃搜索以 `track` 返回并在曲目扩展标明命中内容或彩铃类别。网易云 1009/`djRadios` 按 `Podcast` 映射；专用播客响应的 `baseInfo` 提升为稳定实体，外层算法与命中理由保留在 `extensions.search_item`。综合搜索、声音或上游出现尚无稳定公共结构的条目使用 `opaque`，保留平台、搜索类型、可提取的 ID/标题及完整原文。声音响应同时出现专用 `voices/voiceCount` 与通用 `resources/resourceCount` 时优先专用字段；空的旧数组或空 `result` 不会遮住后续非空数组或旧版 `data`。实际后端和上游路径位于分页扩展 `variant/request_path`，完整上游响应也保存在分页扩展；上游若不应用请求 `limit`，TuneWeave 返回真实条目并显式写入 `limit_applied=false`，不会截断后伪装成已应用分页。

QQ 分类搜索固定使用 Android `music.search.SearchCgiService/DoSearchForQQMusicMobile`，启动时生成并在 `TUNEWEAVE_DATA_DIR/qq-device.json` 私有持久化 GUID、Android ID、IMEI、QIMEI 和匿名会话。TuneWeave 已实现歌曲、歌手、专辑、歌单、MV、歌词、用户、彩铃、节目专辑和节目十类；按真实静默失败边界使用歌曲/专辑/MV/歌词/彩铃 60、歌手 40、歌单 30、其余 10 的页宽，并用同批子请求按上游逻辑槽位实现统一 `limit=1..100` 与任意 `offset`。`search_id/searchid`、`highlight`、稀疏歌单缺口、非稀疏完整性、稳定身份和完整原项均保留。`selectors` 使用强类型 `id/name/type`，同一类型重复选择会在联网前拒绝，避免参考实现中映射只保留末项而向量保留全部的歧义；合法选择同时提交字符串键值映射 `selectors` 和保序对象数组 `vec_selectors`，响应的二维 selector 分组经结构校验后位于分页扩展 `selectors`，本次选择位于 `selected_filters`。命名账户用于搜索等公开元数据时只验证 `(qq, account)` 别名存在，不把账户密钥注入不需要认证的请求；真正的音源授权再由 provider 注入该账户。上游 Python、TuneWeave Rust provider 与统一 HTTP 均真实验证彩铃和 selector 分支：彩铃“周杰伦”返回总数 553，统一结果为可播放 `Track`；selector `id=4558/type=0` 返回 2 条且请求语义完整保留。

B 站视频搜索使用 `platform=bilibili&kind=video`。provider 优先调用现行 WBI 搜索端点，自行取得并缓存 `buvid3/buvid4/b_nut`、Web Ticket 和 WBI 密钥，调用方不能覆盖签名、URL、Cookie 或请求头；平台返回风险票据、业务 `-412` 或 HTTP 412 时，客户端只切换一次仍由 B 站提供的公开视频搜索兼容端点，并在十分钟内复用该选择，不申请验证码、不回显票据，也不把风控响应伪装成空结果。匿名兼容请求不混入新设备 Cookie，选择账户时只携带对应 B 站账户 Cookie。结果保留 AID/BVID、UP 主、封面、允许空值及受限换行的纯文本简介、时长、分区、标签、命中列、公开计数、发布时间和付费/合作标志，并以稳定 `bilibili:bvid:*` 或 `bilibili:aid:*` 引用返回；平台插入的 `video_ad` 与 `video_ad_<number>` 均按推广视频强类型处理，完整卡片以 `search_result_type=video_ad`、`sponsored=true` 和可选数值子型显式标记，空标题广告占位不伪装成可用视频。已知 `ketang` 课程卡因身份和播放协议不与普通稿件互通而明确过滤，不能伪装成视频；其他未知条目类型仍作为协议漂移拒绝。`limit=1..100` 与 `offset<1000` 可跨固定 20 项上游分页取数。视频专用 `order` 接受 `relevance|most_played|newest|most_danmaku|most_favorited|most_commented`，`duration` 接受 `any|under_ten_minutes|ten_to_thirty_minutes|thirty_to_sixty_minutes|over_sixty_minutes`，`category_id` 接受正整数分区 ID；兼容参数同时接受 B 站排序值、时长数字 `0..4` 和 `tids`。这些筛选只允许用于 `kind=video`，不支持的平台会明确拒绝而不会静默忽略。

QQ 综合搜索固定使用 Android `music.adaptor.SearchAdaptor/do_search_v2` 和 `search_type=100`。首请求生成或接受调用方 `search_id`；续页精确回传平台返回的 `sid/nextpage/nextpage_start`，不会把多字段游标压成普通 offset。歌曲、歌手、MV、专辑、歌单和节目六个桶按平台顺序映射为统一类型，各桶自身的计数、`more_info`、未知字段与原始数据均保留；直达分组和相关词独立建模。平台 CGI 包络的业务码先由共享客户端校验，模块 `data` 再由强类型综合搜索模型解析，缺失桶、非法会话、畸形条目或不前进的页码均拒绝为假成功。provider 与统一 HTTP 真实搜索“周杰伦”，首屏及携带同一会话和多字段游标的下一页均通过。

QQ 首页推荐固定使用 Android `music.recommend.RecommendFeed/get_recommend_feed`，首屏默认提交 `direction=0/page=1/s_num=0`，后续页把返回楼层数累加进 `s_num`，以 `direction=1` 推进，并把所有已曝光楼层 ID 放入 `v_cache`。TuneWeave 保留重复楼层及卡片原序，但对缓存 ID 做稳定去重；非空响应若没有新增楼层 ID 会停止生成 `next`，修正参考分页器可能无限重复请求的风险。楼层、细分组、更多动作和卡片均为强类型；当前 `type=200/400/500/1000` 分别提供歌曲/专辑/歌单/榜单统一引用，`700/900` 功能入口和未知类型不会猜测资源身份。封面只接受无凭据 HTTP(S)，动作只接受安全的 QQ Music 或 HTTP(S) scheme，完整实验、布局、反馈、内嵌歌曲和未来字段保留于扩展。provider 与统一 HTTP 匿名真实验收首屏与下一页，包含重复楼层、多种资源卡片和防重复续页状态。

QQ 推荐歌单固定使用 Android `music.playlist.PlaylistSquare/GetRecommendFeed`，统一 `offset/limit` 原样映射为平台 `From/Size`，不先换成可能丢失游标语义的页码。响应 `List[*].Playlist.basic` 强类型映射歌单 ID、标题、简介、封面、歌曲数、播放量和创建者；`HasMore/FromLimit` 生成下一统一 offset，总数保持未知。非默认推荐来源、刷新和地区筛选会明确拒绝，不会静默忽略；命名账户只验证精确别名，省略账户可匿名读取公开推荐。平台声称有下一页但返回空列表、游标不前进、返回数量超限或资源字段畸形时均拒绝为假成功。provider 与统一 HTTP 匿名真实验收连续两页，每页 5 项且身份不重复。

默认搜索词与搜索结果分离：`keyword` 是应提交给搜索端点的真实词，`display_text` 是可直接展示的文案，`kind` 仅在平台类型可映射时返回，图片允许为空。网易云固定使用 EAPI `/api/search/defaultkeyword/get`；空白 `showKeyword` 会继续回退 `styleKeyword.keyWord`，算法、样式词和业务意图等动态字段完整保留在 `extensions.response`，调用方不应解析它们来替代稳定字段。

热搜目录按 `rank` 从 1 开始排序，`keyword` 必填，说明、分数、图标类型、图标 URL 和目标 URL 均按平台实际返回可空。`detail` 缺省为 `full`，也接受 `brief`，并兼容 `mode` 查询名及 `simple/detail/detailed` 值。网易云简略榜固定使用 EAPI `/api/search/hot` 和 `type=1111`，详细榜固定使用 WeAPI `/api/hotsearchlist/get`；两套响应不会互相补造缺失字段，完整原文位于列表与条目扩展。

QQ 热词固定使用 Android `music.musicsearch.HotkeyService/GetHotkeyForQQMusicMobile` 并提交按参考算法生成的搜索会话 ID。上游只有一份富目录，因此 full/brief 复用同一真实快照：full 映射说明、分值、趋势类型、图标和跳转，brief 隐去这些可选字段；展示活动标题不会覆盖可重新搜索的 `query`。封面、热词/直达/歌曲 ID、置顶态、排序与趋势对象、来源及完整原项始终保留在扩展。统一 HTTP 的 full/brief 均真实返回同序 30 项，首项为“周杰伦”，上游 `ret_code=0`。

B 站热搜固定使用现行 WBI `x/web-interface/wbi/search/square`，提交 `limit=50/platform=web`。未传 `account` 时保持匿名，显式账户或调用方凭证才附带相应身份。统一排行按上游列表顺序从 1 连续编号，真实查询词与展示文案分开；`heat_score`、词条类型、可信 HTTPS 图标、动作种类及受限的 B 站 HTTPS/`bilibili:` 跳转 URI 分别映射，服务端不会请求或跟随返回的跳转地址。full 返回这些富字段，brief 仅隐藏稳定模型中的富展示字段而不删除扩展里的平台原始语义。标题、track ID、top list 及未来字段有界保留；Provider 与统一 HTTP 已匿名真实返回 50 项及有效热度分数。

搜索建议的 `client` 缺省为 `web`。统一条目始终给出可直接重新搜索的 `keyword`，可选 `kind/display_text/icon_url`；web 建议中的歌曲、专辑、歌手、歌单等实际资源同时以统一 `SearchItem` 放入 `resource`，mobile/PC 纯关键词不会伪造资源。PC 的 `recs` 与普通 `suggests` 分别位于 `recommendations/suggestions`。网易云 web/mobile 分别固定使用 WeAPI `/api/search/suggest/web`、`/api/search/suggest/keyword`，PC 固定使用 EAPI `/api/search/pc/suggest/keyword/get`；未知或零 `type` 不会遮住可映射的 `resourceType`，为兼容参考输入，`type=mobile` 等同 `client=mobile`。

B 站搜索建议只支持 `client=web`，固定调用 `https://s.search.bilibili.com/main/suggest`，不会把 mobile/PC 偷换为 Web。省略 `account` 时使用持久匿名设备；显式服务器账户或 `X-TuneWeave-Credential` 才附带对应用户身份，不存在的精确别名不会回退 `default`。上游固定高亮标签会被解析为纯 `display_text` 和条目扩展中的 Unicode 字符区间 `highlight_ranges[{start,end}]`，不会向客户端透传可执行 HTML；未知、嵌套或未闭合标签按上游协议漂移拒绝。平台的 `code=3 + 空 tag` 是合法无匹配响应，统一返回空 `suggestions`；最多 10 项建议、实验标识、搜索 token、报告数量和未来字段均保持分离。Provider 与统一 HTTP 已匿名真实搜索“周杰伦”成功。

QQ 的 `client=mobile` 精确对应 Android `music.smartboxCgi.SmartBoxCgi/GetSmartBoxResult`：普通 `items` 与 `vec_related_items` 分别进入 `suggestions/recommendations`，`vec_direct_items` 依据 `insert_pos` 插回建议序列并尽可能提升为统一资源。搜索会话 ID 位于列表扩展；高亮展示、图标、跳转、分值、预搜索标志、关联资源 ID 及完整上游包装逐项保留。`client=web` 精确对应参考 `quick_search` 的固定 HTTPS `c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg`：各分区按上游 `order` 动态排序，单曲、歌手、专辑和 MV 提升为统一资源，未来未知分区以保留完整原项的 `opaque` 表达；分区元数据和完整响应保存在扩展。`pc` 没有独立 QQ 上游分支，会明确报错而不会偷换成 web/mobile 结果。统一 HTTP 真实返回 21 项移动端建议（首项为歌手直达资源）和 10 项 web 快速结果（首项为“晴天”歌曲，上游 `code/subcode=0`）。

多重搜索的 `kind` 缺省为 `track`，接受与普通搜索相同的统一名称和网易云数字类型，参考字段 `type` 是其别名。网易云固定使用 WeAPI `/api/search/suggest/multimatch` 并精确提交 `s/type`；非空 `result.orders` 决定分区顺序，值为 `null` 时回退兼容字段 `order`，未列入顺序但实际返回的数组仍会追加保留。`artist/playlist/new_mlog` 等已知分区分别规范化为统一歌手、歌单与视频资源，空/零视频或创作者 ID 和零时长不会遮蔽有效兼容值，未知分区以不透明条目表达；完整上游响应位于结果扩展。

本地歌曲匹配的 `md5` 必填并按 32 位十六进制校验，标题、专辑和歌手允许为空以保留参考模块的默认分支；时长省略时按参考行为使用 0。若同时提供毫秒与秒数，两者四舍五入到毫秒后必须一致。网易云固定使用未加密直连 API `/api/search/match/new`，把一项标签记录序列化进 `songs`；上游 `result.ids/songs` 分别映射为匹配 ID 和统一候选曲目，空数组原样表达无命中。

用户完整资料的 `backend` 缺省为 `modern`，也接受 `new/eapi/v2`；网易云精确对应参考 `user_detail_new`，以 EAPI 调用 `/api/w/v1/user/detail/{uid}` 并提交字符串 `all=true/userId`。`backend=legacy`（也接受 `old/weapi/v1`）精确对应 `user_detail`，以空载荷 WeAPI 调用 `/api/v1/user/detail/{uid}`。两条路径共用 `UserProfile`，但通过独立能力和 `extensions.backend/response` 保留实际后端及完整响应。空包装、空文本、零时间戳不会遮蔽后续有效兼容字段，返回用户 ID 与请求不一致时按上游错误拒绝。`/v1/account/profile` 先从指定 `platform/account` 的持久登录态取得用户 ID，再以同一账户请求资料，不会借用默认账户或把登录凭据写入响应。已真实验证公开 legacy/modern 及持久账户 modern 三条统一 HTTP 路径。

QQ 用户资料只支持 modern 后端，固定调用 Android `music.UnifiedHomepage.UnifiedHomepageSrv/GetHomepageHeader` 并提交 `uin/IsQueryTabDetail=1`。`GET /v1/users/qq:<encrypted-uin>?account=...` 把路径中的加密 UIN 作为目标用户，显式 `account` 只选择登录查看者；`GET /v1/account/profile?platform=qq&account=...` 则把所选会话的数值 music ID 精确换成同一凭据保存的 `encryptUin`，不会混用两种身份。稳定资料包含规范用户引用、名称、安全头像/背景、关注态和粉丝/关注数；朋友/访客数、用户类型、歌手关联、标签页、提示、未知字段与完整响应保存在扩展。响应身份不一致、非法布尔标志、危险 URL、畸形必需字段和非零状态都会拒绝。参考实现的匿名分支会注入固定假 music ID/musickey；TuneWeave 不制造或发送占位凭据，真实匿名请求已确认返回业务码 1000，因此两条 QQ 路径均要求真实 `(qq, account)` 登录态。普通账户的当前资料和同一加密 UIN 显式用户资料已真实返回相同规范引用。

QQ 音乐基因使用独立的 `GET /v1/users/qq:<encrypted-uin>/music-gene?account?=...`，固定调用 Android `music.recommend.UserProfileSettingSvr/GetProfileReport` 并只提交路径目标为 `VisitAccount`。它与需要查看者登录的 QQ 主页头部分离：公开音乐基因可匿名读取，显式 `account` 只选择精确 `(qq, account)` 查看者凭据，不存在时不会回退 `default`。统一 `UserMusicGene` 强类型保存目标用户、平台原样的访问标志、偏好动作、月度听歌报告、年龄变化、速度范围、性格色彩、曲风、律动、音乐偏好、人格、偏好歌手、慢歌程度、时段偏好、状态指标、主描述、AI 解读，以及 `SortArray/sortCard` 的原始展示顺序。真实响应中的 `IsVisitAccount=1` 也会出现在匿名访问公开用户时，因此字段中性命名为 `is_visit_account`，不会误解为“当前登录用户”。已知字段全部先经强类型、身份、大小、区间和安全 URL 校验；当前上游没有公开元素结构且真实响应仍为空的 Deepseek/CardBPM 动态列表只允许有界 JSON 数组并放在扩展，不用裸 JSON 代替其余已知模型。公开目标的 Rust Provider 与统一 HTTP 匿名真实通过，听歌报告、个性维度及 8 项展示/卡片顺序完整返回。

QQ 不喜欢目录固定调用签名端点 `https://u.y.qq.com/cgi-bin/musics.fcg` 的 `music.feedback.FeedbackBlack/GetDislikeList`。`track/artist/style` 分别映射 `Cmd=3/2/4` 与 `SongLastid/SingersLastid/StyleLastid`；`page` 从 1 开始，零游标按参考首屏语义省略。`zzc` 签名基于实际发送的同一份 JSON 字节计算，调用方不能注入目标 URL、签名、Cookie、代理或请求头。响应只接受所选类别容器；平台真实歌曲条目返回 `IdType=0`，参考模型使用 `1`，因此歌曲容器明确接受 `0|1` 并原样保留，歌手和风格仍严格要求 `2/3`，不会无界放宽。条目返回字符串 ID、名称、安全图片和 RFC 3339 添加时间；平台 `Token` 是目录分页元数据，不是账户凭据。QQ 没有可靠总数或 `has_more`，因此非空页从所选类别末项推导 `next_page/next_cursor`，空页终止；三类列表全空且平台 `Page=0` 是已真实出现的空目录哨兵。普通账户已真实完成三类空目录和歌曲 0→1→0 的签名读取闭环。

添加 QQ 不喜欢内容使用普通 Android `music.feedback.FeedbackBlack/AddDislike`，不复用读取目录的签名端点。统一 `track/artist/style` 和参考 `id_type=1/2/3` 分别只生成 `Songs/Singers/Styles` 一个请求容器；每项携带规范十进制字符串 `ID` 及写协议要求的 `IdType=1/2/3`，批量顺序和重复项不会被集合化。歌曲写入类型 1 与读取真实类型 0 是不同方向的协议事实，不会互相覆盖。空列表、非正整数、超出 `u64` 的 QQ ID、类别冲突和 `ids/values` 冲突均在账户查找或联网前拒绝。成功结果明确返回 `action=add/applied=true` 和规范 ID；`Retcode` 非零进入统一上游错误而不虚报写入成功。普通账户已真实添加一首歌曲并由签名目录读回精确 ID。

取消 QQ 不喜欢内容使用同一批量模型和普通 Android CGI，仅将方法切换为 `music.feedback.FeedbackBlack/CancelDislike`，成功结果返回 `action=remove`。参考实现把空 `values` 改写为空数组后仍发起写请求；TuneWeave 认为这既没有可观察效果又会增加账户风控面，因此空批次在账户查找和联网前返回 400。其余类别、ID 规范化、顺序/重复项、精确账户、强类型 `Retcode` 和完整响应语义与添加操作一致。普通账户已取消同一测试歌曲，签名目录恢复为空且无残留。

清空 QQ 歌曲不喜欢目录完整执行 `music.feedback.FeedbackBlack/CancelAllDislike` 的两阶段事务：第一请求原样保留布尔 `ISOnlyGetToken=true`，取得并校验一次性 Token；第二请求提交 `DelType=3/Token`。Token 仅在同一次 provider 调用的内存中传递，不进入稳定模型、HTTP 响应、扩展、错误或 Debug；保存第一阶段平台数据和原响应前会递归移除所有大小写形式的 Token 字段，并以 `token_redacted=true` 标记。空白、含控制字符或超过 4096 字节的 Token 会阻止第二阶段，事务不做无限重试。最终只有第二阶段 `Retcode=0` 才返回 `kind=track/action=clear/applied=true`；独立 `/tracks` 路径避免调用者把批量取消误作清空，也不会影响歌手和风格目录。

会员摘要同时提供公开用户和当前账户两条统一路径。`backend` 缺省为 `front`，也接受 `public/v1`；网易云固定使用 WeAPI `/api/music-vip-membership/front/vip/info`，公开用户把引用 ID 作为 `userId`，当前账户按参考默认分支提交空字符串。`redVipLevel/redVipAnnualCount/redVipLevelIcon` 分别映射为等级、年费次数和图标；该公开接口没有可靠有效期和激活态，因此相关字段保持可空。

`backend=client`（也接受 `detail/v2`，字段名兼容 `variant/source`）通过独立 `user_membership_client_info` 能力精确对应参考 `/vip/info/v2`，固定使用 WeAPI `/api/music-vip-membership/client/vip/info`。该分支无论是否指定用户都要求 `account` 指向已登录会话，不会静默回退公开摘要；`redplus/musicPackage/associator/voiceBookVip/albumVip` 的最长有效期驱动稳定激活态和到期时间，等级、年费次数和非空动态图标映射到稳定字段，全部权益包及未来平台字段保留在 `extensions.response`。

QQ 会员信息只支持当前登录账户：`GET /v1/account/membership?platform=qq&account=...` 固定调用 Android `VipLogin.VipLoginInter/vip_login_base`，空参数但同时在 `comm` 和 Cookie 中注入精确 `(qq, account)` 凭据。`/v1/users/{qq-uin}/membership` 仅接受与所选账户一致的正整数 UIN 并复用该能力，不能查询任意用户；这里的数值 UIN 与 QQ 主页使用的加密 UIN 是不同平台身份，不可互换。QQ 没有第二个 `client` 后端。统一摘要从 `identity.level`、全部会员标志、`userinfo.expire` 和非空等级图标提炼等级、激活态、RFC 3339 到期时间与图标，`annual_count` 不会拿年费布尔标志冒充次数。顶层容量/续费/星级字段、`identity` 的绿钻/豪华/年费/家庭/情侣等身份、`userinfo` 的积分/音乐等级/入口和未来字段均以强类型解析并保存在 `extensions.vip`，原始包装位于 `extensions.response`；已出现但类型错误的已知字段不会按默认值伪装成功。普通账户已真实验证当前账户与同一数值 UIN 用户路径，稳定权益摘要完全一致。

广告换听目录的 `type_ids` 缺省为 `400002_0`，既接受逗号列表，也兼容参考项目的 JSON 字符串数组；顺序与重复项保留，最多 100 项。网易云固定使用带实时 v3 checkToken 的 XEAPI `/api/ad/get`，精确把类型数组序列化进 `type_ids` 字符串。对象或数组广告包装统一为稳定条目，逐项解析 `extJson.contextInfo.req_id`；无效或空的前一项不会遮蔽后续有效请求 ID，无法解析的 `extJson` 仍随原广告完整保留。匿名设备可能合法返回空目录，不伪造成错误或虚构请求 ID。

广告换听领取的 `creative_type/rights_gain_method` 均默认 2；曝光和点击时间都省略时使用同一次读取的当前 Unix 毫秒。参考 GET 的显式时间值原样保留为 JSON 字符串，统一 POST 同时接受整数毫秒和字符串，以免改变上游参考协议的类型分支。四个可选时长/方式、来源、权益扩展文本、任意 JSON `app_info` 和安装态都会进入 `reqParam` 内层 JSON；缺省字段不会伪造。省略或传空 `request_uid` 时，provider 先按同一 `platform/account/type_ids` 取广告目录；目录失败、无投放或无 `req_id` 时依照参考行为继续提交空 ID，并以扩展字段明确来源。网易云领取固定使用带 v3 checkToken 的 XEAPI `/api/ad/listening/rights/gain`；匿名真实请求当前返回业务码 2001，统一映射为 `authentication_required`，不会误报已领取。

私人 FM 与每日推荐目录分离。`backend` 缺省为 `classic`，也接受 `default/personal_fm`；网易云固定使用 WeAPI `/api/v1/radio/get` 且不提交伪分页参数。`backend=mode`（也接受 `personal_fm_mode`）使用 EAPI 同路径，完整保留可选 `mode/subMode/limit`，其中 `sub_mode` 也接受 `submode/subMode`。模式字符串只做长度和空白边界校验，不把平台将来增加的模式限制在本地枚举中。响应是当前队列快照：`total` 为本次返回数量，`has_more=false/next_offset=null/continuation_supported=false`；`limit` 只控制本次映射上限，不会伪造上游分页。匿名真实联网已分别验证经典和模式后端均返回非空统一 `Track` 队列。

首页个性化和登录账户每日推荐共用按资源类型稳定的端点，但以显式 `source` 分支区分，不静默互换。`source` 缺省为 `daily`，也接受 `default`；`personalized` 兼容 `homepage/home/personalised`。网易云个性化新歌固定使用 WeAPI `/api/personalized/newsong`，提交 `type=recommend/limit/areaId`，其中 `area_id` 缺省为 0 且只允许用于该分支；个性化歌单固定使用 `/api/personalized/playlist`，精确提交 `limit/total=true/n=1000`。两者均为不支持 offset 的首页快照，非零 offset 会明确拒绝；推荐算法、文案、是否可反馈及完整包装保存在单项扩展，当前上游可能把歌单播放量返回为浮点 JSON，TuneWeave 会无损保留而不强制降格为整数。

QQ 新歌发现复用 `GET /v1/recommendations/tracks` 的 `source=new_releases` 分支；为保持省略 `source` 时平台端点可直接使用，QQ 的缺省 `daily` 也指向该公开目录，但响应扩展会明确标记真实来源为 `new_releases`。`area_id` 精确采用 QQ 原生六类：`1` 内地、`2` 欧美、`3` 日本、`4` 韩国、`5` 最新（缺省）、`6` 港台；其余值和 `refresh=true` 明确拒绝。QQ 固定调用 Android `newsong.NewSongServer/get_new_song_info`，平台只接受地区类型而没有原生分页，因此 TuneWeave 对同一完整快照应用稳定 `offset/limit`，返回真实总数与下一偏移。当前地区、六类可选目录、统计标识、按歌曲数字 ID 关联的首发标签、发布时间和完整平台响应均保留；已知目录与标签先经强类型和安全链接校验，不把畸形字段伪装成空结果。匿名请求可直接使用，显式 `account` 只校验精确别名。已真实逐类验证六个地区及各自第二个本地分页窗口。

QQ 猜你喜欢使用同一歌曲推荐端点的 `source=personalized` 分支，固定复刻 Android `music.radioProxy.MbTrackRadioSvr/get_radio_track` 的 `id=99/num=5/from=0/scene=0/song_ids=[]` 快照。参考方法不暴露稳定续页参数，因此只允许 `offset=0`；`limit` 在最多 5 首的本次快照上应用，`has_more=false/next_offset=null/continuation_supported=false`，不会把再次请求得到的新推荐冒充同一页游标。`refresh=true` 可明确记录调用者意图，但平台每次本来就是同一无状态请求，不伪造不存在的请求字段。QQ Android 匿名设备可以调用；省略账户时优先使用已经持久化的 `default` 凭据，缺少默认登录态则安全降为匿名，显式 `account` 必须精确存在并注入同一 `(qq, account)` 凭据。每首歌曲与同位置 `extras` 严格对齐，展示理由按更具体的 `RecReasonTemplate`、通用 `reason`、备用 `title` 顺序选择，低优先级字段不会覆盖高优先级字段；三个原字段、原因类型/列表、实验信息、倒计时、背景资源、关联视频卡片和完整响应全部保留。匿名 Rust provider 已真实返回 5 首，取前 3 首时全部具有非空具体推荐理由。

QQ 雷达推荐使用 `source=radar`，固定调用 Android `music.recommend.TrackRelationServer/GetRadarSong`，请求保留原生 `Page` 与 `ReqType=0/FavSongs=[]/EntranceSongs=[]`。平台只提供每页 10 首的页码，TuneWeave 会把统一任意 `offset/limit=1..100` 转成首个物理页、页内跳过数和按需连续页；因此窗口可以跨越原生页边界，`next_offset` 只在真实仍有未消费歌曲或平台后页时前进，总数在平台未提供时保持未知。物理页宣称有后页却不足 10 首、歌曲为空、ID 错位或游标不前进均按协议错误处理，不伪造续页。匿名设备可以直接调用；省略账户时优先使用持久 `default`，缺失则匿名降级，显式账户严格按 `(qq, account)` 注入。`RecommendSongIds` 与歌曲数字 ID 逐项校验，展示理由固定取更具体的 `RecReasonTemplate`；平台允许个别条目不提供模板，TuneWeave 保留为空而不虚构文案。原因列表、实验、统计、Trace、发布时间、Toast、时间戳、倒计时、视频卡片、逐页上下文、未知字段和每页完整响应均保留。平台没有刷新和地区分支，对应参数会在联网前拒绝。匿名 Rust provider 已真实验证 `offset=8/limit=3` 跨原生两页返回 3 首，下一偏移为 11，歌曲身份对齐且非空具体理由保持模板优先。

QQ 相似歌曲使用 `GET /v1/tracks/{qq-ref}/similar`，固定调用 Android `music.recommend.TrackRelationServer/GetSimilarSongs` 并提交正数 `songid`。数字 ID 直接使用；MID 先复用 Web 富详情取得同一数字身份，响应中的 `track_ref` 仍保留调用方原引用。平台一次返回两个不同语义的数据源：`vecSong` 是直接相似歌曲，`vecSongNew` 是带标题模板的相同听众分组；参考响应模型只选择后者，TuneWeave 以强类型 `direct/audience` 分区同时保留两者。`limit`（别名 `limit_per_section/limitPerSection`）默认 15，在每个不可续页快照分区本地裁切且保存原数量和是否应用上限，不把重复请求伪装成分页。歌曲保留完整统一详情、发布时间、分区排名、实验、`tf`、Trace 和未知包装；`songTagInfoList` 按数字歌曲 ID 附到对应歌曲，完整标签目录、分组模板/内容、`extra_info`、平台消息和完整响应也不会丢失。公开请求可匿名，显式账户只校验精确别名。数字 ID `97773` 与 MID `0039MnYb0qxYhV` 的真实 provider 请求均返回直接相似 12 首和一个 15 首相同听众分组，两种身份各取每区 3 首通过。

QQ 歌曲标签使用 `GET /v1/tracks/{qq-ref}/labels`，固定调用同一 Android `TrackRelationServer/GetSongLabels` 并提交正数 `songid`；数字 ID/MID 的身份解析和精确账户隔离与相似歌曲一致。`TrackLabelList` 将平台标签 ID 表达为可扩展字符串，展示文字、图标和动作均为独立可选字段，原生 `tagType/species` 只提升为明确的 `platform_type/platform_category`，在语义未证实时不擅自命名。平台合法返回 `id=0`、只有图标而无文字、只有 taxonomy 而无展示字段，以及包含换行的多项奖项；这些分支全部保留，危险图标 URL、危险动作 scheme、畸形字段、非法控制字符和超大列表会拒绝。完整原项、实验、未来字段和平台响应进入扩展，空目录按参考模型的真实空列表行为返回且不伪造分页。真实响应已确认数字 ID `97773` 同时覆盖 11 个标签项；数字 ID 与 MID 两条 provider 链路已联合验收。

QQ 相关歌单使用 `GET /v1/tracks/{qq-ref}/related-playlists`，固定调用 Android `TrackRelationServer/GetRelatedPlaylist`，提交正数 `songid` 和上一批 `vecPlaylist`。统一输入 `previous_ids` 兼容参考 `last`、原生 `vecPlaylist/vec_playlist` 与 `cursor`，可用 JSON 数组或逗号列表；超过 100 项、空项、重复项、零/非法 QQ 歌单 ID 均在联网前拒绝。平台响应实际包含两支：`vecPlaylist` 是 3 项可换批直接结果，`vecPlaylistNew` 是带“喜欢这首歌的人也爱”标题的 7 项稳定分组；参考模型只展开后者，并错误地从稳定分组生成下一游标，回传后会原样重复。TuneWeave 以 `direct/audience` 强类型分区完整保留两支，只从直接结果生成 `next_ids`，并拒绝 `hasMore` 但空批或游标集合不前进的假续页。歌单保留标题、安全封面、创建者、歌曲数、可选播放量、完整原项、分组标题/附加字段和完整响应；空播放量不会被伪造成零。数字 ID `97773` 与 MID `0039MnYb0qxYhV` 首批真实通过，随后把首批 3 个直接 ID 回传得到不同的下一批，参考游标缺陷经差分验证确认。

QQ 相关 MV 使用 `GET /v1/tracks/{qq-ref}/related-videos`，固定调用 Android `MvService.MvInfoProServer/GetSongRelatedMv`，提交字符串 `songid`、`songtype=1` 和数字 `lastmvid`；首批固定为 `0`。数字歌曲 ID 直接使用，MID 先解析为同一数字身份，响应 `track_ref` 始终保留调用方原引用。参考方法把 `last_mvid` 注释为 VID，但其响应模型实际从数字 MV ID 生成游标，真实平台也只接受并推进数字 `mvid`；TuneWeave 因此把可选 `previous_id/lastmvid/last_mvid/cursor` 严格建模为正整数，而每个可播放 MV 仍以字母数字 VID 形成 `qq:<vid>` 资源引用，数字 ID 只进入 `extensions.numeric_id`。`hasmore` 仅在非空列表提供 `next_id`，假续页、游标不前进、重复数字 ID/VID、畸形歌手、危险图片地址和超界目录均明确失败。标题、封面、播放量、歌手身份与头像、响应未知字段和完整原文均保留。数字 ID 与 MID 的真实 provider 首批均通过；统一 HTTP 连续两页各返回 3 个 MV，第二页数字游标前进且资源身份保持 VID。

QQ 同曲其他版本使用 `GET /v1/tracks/{qq-ref}/versions`，固定调用 Android `music.musichallSong.OtherVersionServer/GetOtherVersionSongs`。数字引用原样提交正数 `songid`，MID 原样提交 `songmid`，不会先把一种身份改写成另一种；响应 `track_ref` 也保持调用方来源身份。`versionList` 作为不可续页的有序完整 `Track[]` 返回，每项复用 QQ 完整歌曲映射并保留从 0 开始的 `version_index`、数字 ID、MID、歌手、专辑、文件规格、付费状态和原始条目；平台合法的 `null` 目录映射为空列表，不伪造存在版本。非对象或畸形歌曲和超过 1000 项的异常响应明确失败，顶层未来字段及完整响应进入列表扩展。公开请求可匿名，显式账户只验证精确别名而不向不需要凭据的请求注入密钥。数字 ID `97773` 与 MID `0039MnYb0qxYhV` 的真实 Provider 和统一 HTTP 均返回相同规模的 10 项非空版本列表，两种来源引用及平台顺序保持正确。

QQ 歌曲制作信息使用 `GET /v1/tracks/{qq-ref}/credits`，固定调用 Android `music.sociality.KolWorksTag/SongProducer`。数字歌曲 ID 只提交正数 `songid`，MID 只提交严格校验的 `songmid`，响应 `track_ref` 保持调用方来源身份。平台 `Lst` 的角色标题、原生类型及有序 `Producers` 映射为强类型分组；人员姓名、可选歌手 MID、HTTPS 头像、QQ 音乐或 HTTPS 动作地址和 `Follow=0/1` 关注态分别保存，未知关注值不会被误判为布尔值，原生字段仍进入扩展。平台合法空目录返回空分组；空白身份、危险 URL、畸形 MID、异常列表规模和错误容器明确失败。公开请求匿名，显式账户仅验证精确别名且不会注入不需要的登录凭据。数字 ID `97773` 与 MID `0039MnYb0qxYhV` 的真实 Provider 和统一 HTTP 均返回 10 个非空角色分组，首组“演唱”及人员“周杰伦”一致，来源引用保持正确。

QQ 曲谱存在性使用 `GET /v1/tracks/{qq-ref}/sheet-music/availability`，固定调用 `music.mir.SheetMusicSvr/HasSheetMusic`，并按参考协议以精确覆盖的 `g_tk/uin/format/inCharset/outCharset/notice/needNewCode` 公共参数请求，不混入 Android 会话、Cookie 或调用方可注入字段。上游只接受歌曲 MID；MID 直接提交，数字 ID 先通过已验证的歌曲详情解析规范 MID，统一响应仍保持调用方原引用。`hasGuitar/hasMore/hasLDY/hasQRCX/hasChongChong` 分别映射为 AI 谱、附加目录、六线谱、标准谱和外部目录五个布尔字段，`available` 对全部字段取 OR，不使用会让低优先分支遮蔽其他能力的 `if/else`。五个字段全部必需且只接受布尔或 0/1，缺失和非二进制值拒绝为假状态；未知字段与完整响应保留。数字 ID `97773` 与 MID `0039MnYb0qxYhV` 的真实 Provider/统一 HTTP 均返回可用，五个独立标志全为真且解析到相同 MID。

QQ 曲谱详情使用 `GET /v1/tracks/{qq-ref}/sheet-music` 并完整保留参考方法的三条来源分支。`source=user`（默认）调用 `GetMoreSheetMusic` 与 `scoreType=-1/ttype=0`；`source=ai` 调用同一方法与 `scoreType=-473/ttype=1`；`source=external` 调用 `GetChongChongSheetMusic`，保留其看似不对称但真实有效的 `scoreType=-1/ttype=1`，同时只为该分支加入 `platform=h5` 并使用固定签名端点。三支都精确提交 `begin=0/end=100`，因为参考公开方法没有分页参数，统一层不会伪造续页；业务码 `10007` 仅作为可解析的合法空目录放行，其他非零码明确失败。`source` 兼容 `type/ttype` 和 `0/1/2`，但响应统一为 `user/ai/external`。数字 ID 先解析规范 MID，MID 直接使用，列表仍保留原输入引用；每项以独立 `qq:sheet:<scoreMID>` 标识，强类型保存关联歌曲、名称、图片序列、版本/调性/谱型/乐器/难度、上传者、浏览量、创作与演唱信息、详情/封面/专辑/文件 URL，并同时保留任意 `totalMap` 分类计数。图片和文件并列存在，不以一种载体覆盖另一种；危险 URL、跨歌曲错位、异常目录、畸形字段和超过固定窗口的响应均拒绝。`97773` 与 MID `0039MnYb0qxYhV` 的真实 Provider/统一 HTTP 六条组合均通过：用户上传 21 项、AI 42 项、虫虫 4 项，前两类首项含 3 张谱图，虫虫首项含文件地址，两种歌曲身份的数量、计数与来源一致。

QQ 歌曲收藏人数使用 Android `music.musicasset.SongFavRead/GetSongFansNumberById`。单曲 `GET /v1/tracks/{qq-ref}/favorite-count` 与批量 `GET/POST /v1/tracks/favorite-counts` 共用一次最多 100 项的原生 `v_songId` 请求；数字 ID 直接提交，MID 先通过歌曲详情解析数字身份，同批重复 MID 只解析一次。平台 `m_numbers` 映射为精确无符号 `count`，`m_show` 独立映射为可选 `display_text`，不会让“万”等展示缩写覆盖真实人数。批量输出严格恢复调用方引用、顺序和重复项，并保留 `input_index` 与解析后的数字 ID；人数键集合缺失或多出目标、展示键越界、负数、控制字符、身份错位及混合平台在明确层级失败。响应 data 中未来字段和剥离重复数据后的顶层响应元信息进入每项扩展，避免完整批量字典在每项复制导致二次方膨胀。公开请求匿名，显式账户只校验精确别名。真实 Provider 与统一 HTTP 的单曲、GET 批量和 POST 批量均通过；`97773`、对应 MID `0039MnYb0qxYhV` 及重复数字 ID 返回相同正数和非空展示文案。

首页视频推荐以 `kind/view` 保留三个不同上游能力：`mv/featured` 对应 WeAPI `/api/personalized/mv`，`exclusive/featured`（别名 `privatecontent/entry`）对应 `/api/personalized/privatecontent`，二者都是不可续页快照；`exclusive/catalog`（也接受 `view=list/all`）对应 `/api/v2/privatecontent/list`，精确提交 `offset/limit/total="true"` 并按真实 `more` 生成下一偏移。平台没有个性化 MV 分页目录，因此 `mv/catalog` 会明确拒绝，不会拿独家放送替代。条目统一为 `Video`，MV 艺人、封面、正时长、播放量、收藏态和独家放送时间按可用字段映射，入口与分页包装完整保留在扩展。

网易云独立 MV 目录精确覆盖 `mv_all/mv_first/mv_exclusive_rcmd`。`all` 把 `area=all|mainland_china|hong_kong_taiwan|western|japan|korea`、`type=all|official|original|live|netease`、`order=rising|hot|new` 序列化为参考 `tags` JSON 字符串；相应中文值也可直接输入。`latest` 只提交 `area/limit/total=true`，明确拒绝非零 offset 及类型/排序；`exclusive` 只提交 `offset/limit` 并拒绝所有虚构筛选。`count`、`hasMore/more` 分别驱动真实总数和续页；最新目录没有续页控制时固定 `next_offset=null/has_more=false/continuation_supported=false`，不会把一屏数据伪装成完整分页。空白备用描述/封面会继续读取有效字段，零时长表达为未知而非有效 0 毫秒。三个统一目录均真实 HTTP 返回 200 和非空 `Video[]`。

QQ 分类 MV 目录使用同一 `GET /v1/videos`，要求 `platform=qq&catalog=all`。`area` 支持 `all|mainland_china|hong_kong_taiwan|western|korea|japan`，`type` 支持 `all|mv|live|cover|dance|film|variety|children`，`order` 支持 `new|hot`；省略筛选时遵循 QQ 参考默认的全部地区、全部类型、最新排序。统一 `offset/limit` 精确映射为 QQ 的 `start/size`，不会强制按整页对齐。QQ 不支持的统一目录、分组和平台专属类型会返回 `invalid_request`，不会暗中降级。响应以 VID 为稳定引用，并返回歌手、封面、时长、播放量、发布时间及真实分页；已对内地 MV 最热目录的 offset 0/1/2 进行真实连续性验证。

网易云视频收藏统一按 `kind` 分派而不混用协议：MV 精确覆盖 `mv_sub`，PUT/DELETE 分别调用 WeAPI `/api/mv/sub|unsub` 并同时提交数值 `mvId` 和参考格式字符串 `mvIds=["..."]`；普通视频精确覆盖 `video_sub`，调用 `/api/cloudvideo/video/sub|unsub` 并提交不透明字符串 `id`。数值引用省略类型时按统一规则推断为 MV，非数值引用推断为普通视频，也可显式指定。`mv_sublist` 固定调用 WeAPI `/api/cloudvideo/allvideo/sublist`，提交 `limit/offset/total=true`，将上游混合返回的字符串 `vid`、创作者、封面、时长、播放量和来源 `type` 映射为已收藏 `Video[]`，完整单项及去除大数组后的分页响应分别保存在扩展。持久化真实账户已验证列表读取，并分别完成 MV 未收藏→收藏→取消收藏及普通视频已收藏→取消收藏→恢复收藏的写入闭环，最终状态均与测试前一致。

网易云站内视频分类与标签分别精确覆盖 `video_category_list`、`video_group_list`：分类提交参考 `offset/total="true"/limit`，标签接口提交空对象且不伪造其不存在的 offset；上游即使不应用请求 limit 也返回完整目录，并以 `limit_applied=false` 明示。`catalog=timeline_all`、`timeline_recommended`、`group` 分别覆盖 `video_timeline_all`、`video_timeline_recommend`、`video_group`，完整保留参考固定字段 `groupId/need_preview_url/filterLives/withProgramInfo/needUrl/resolution`。时间线不提交虚构 limit，按 `hasmore` 与实际返回数推进下一 offset；外层算法包装和内层视频均不丢失。上游合法的 `datas=null` 分类响应规范化为空页，完全缺失或错误类型仍作为协议错误。分类 9 项、标签 107 项、全部/推荐时间线各 8 项均真实返回，累计 63 次实际 group 请求均为 200 空页。

推荐节目的 `source=personalized` 固定使用 WeAPI `/api/personalized/djprogram`，外层推荐包装中的 `program` 映射为完整 `PodcastEpisode`；节目、所属播客和承载音频三种引用保持分离，并可直接复用节目取流链路。该接口不接受分页控制，因此只允许 `offset=0`，`limit` 是本地快照上限，分页扩展明确 `continuation_supported=false/limit_applied=false`。`source=category` 固定使用 WeAPI `/api/program/recommend/v1` 并精确提交 `cateId/limit/offset`；省略 `source` 但提供分类字段时会自动选择该分支，省略分类则完整复刻参考模块未提供 `type` 的调用。上游当前 `offset` 确实生效，但即使后续偏移仍返回不同节目也可能报告 `more=false`，因此 TuneWeave 如实保留 `more`、允许调用方显式偏移，却不会伪造 `next_offset`。匿名真实 provider 与统一 HTTP 已验证分类 `2` 的偏移 0/2 分别返回不同的两期完整节目及可播放音频，两个响应均保留上游 `code=200/more=false`；同一次联网测试也覆盖既有个性化首页六分支，七个分支均返回非空类型化资源。

推荐反馈要求完整曲目引用，引用决定平台，`account` 选择该平台的持久账户别名。网易云固定使用 WeAPI `/api/v2/discovery/recommend/dislike`，精确提交 `resId`、`resType=4`、`sceneType=1`；未知平台、跨平台冲突及空 ID 会在请求前拒绝。匿名真实联网会把上游登录边界映射为 401 `authentication_required`，成功写入留到 项目范围末尾用持久化账户集中验收。

为兼容网易云参考项目，横幅端点也接受 `type=0|1|2|3`，依次对应 PC、Android、iPhone、iPad；响应始终使用统一字段与客户端名称。`catalog` 缺省为 `music`，也接受 `scope` 别名；`podcast`（别名 `dj`）使用平台播客横幅目录，网易云该目录没有客户端选择能力，因此只允许缺省的 PC 值并把目标 `60001` 映射为 `podcast_episode`。

广播电台目录同时接受参考项目的 `categoryId/regionId/lastId` 命名。网易云以 `last_id+score` 作为真实游标；两者可独立传入，另一项分别按 `0/-1` 补齐。参考接口类型虽公开 `offset`，但模块实现与真实上游都不应用它，因此 TuneWeave 仍接收并在分页扩展返回 `requested_offset` 与 `offset_applied=false`，不会把首屏伪装成偏移页。首屏还可能插入推荐电台，实际 `data` 数量可以大于请求 `limit`，TuneWeave 保留完整上游结果并以真实末项生成下一游标。

播客分类与直播广播分类保持不同实体和端点，避免把点播节目目录误当成地区电台。网易云 `kind=all` 固定使用空负载 WeAPI `/api/djradio/category/get`，`kind=non_hot`（兼容 `exclude_hot`）固定使用 `/api/djradio/category/excludehot`；`id` 统一为不透明字符串，图标依次从网页、尺寸和客户端专用字段选择，全部平台字段保存在单项扩展，整份上游响应保存在目录扩展。`/v1/podcasts/category-recommendations` 对应空负载 WeAPI `/api/djradio/home/category/recommend`，返回分组而非扁平播客列表：每个分组完整保留分类、三项推荐播客、算法/推荐文案和原始包装，不会只抽取分类而丢弃内容。`platform` 选择内容平台，`account` 只选择该平台的持久账户别名。真实统一 HTTP 分别返回 19 个完整分类、13 个非热门分类及 12 个推荐分组，首组为分类 `3`“情感”并含 3 个完整播客。

播客目录的 `catalog` 必填并采用跨平台稳定名称；平台尚未实现的目录会明确返回 `invalid_request`，不会静默换成另一种目录。网易云 `catalog=hot` 固定使用 WeAPI `/api/djradio/hot/v1` 并精确提交 `limit/offset`；`catalog=featured` 固定使用无参数 WeAPI `/api/djradio/recommend/v1`，是不可续页的完整精选快照，因此要求 `offset=0`，不接受 `category_id`，上游不会应用统一 `limit`，分页扩展以 `limit_applied=false` 明示。`limit` 默认为 30、范围 1–100，`offset` 默认为 0。统一结果映射封面、主播、分类、节目数、订阅数、播放数、付费态与创建时间；热门目录没有可靠总数时 `total=null` 且真实 `hasMore` 决定 `has_more/next_offset`，精选快照则以返回项数作为 `total` 并固定 `has_more=false`。每项原文及完整响应都保存在扩展中。

网易云 `catalog=personalized` 固定使用 WeAPI `/api/djradio/personalize/rcmd`，精确应用 `limit`，要求 `offset=0` 且不接受分类筛选。该接口返回头部推荐而不提供总数或续页游标，因此统一分页保持 `total=null/next_offset=null/has_more=false`，同时以 `limit_applied=true` 表明请求数量已传给上游；推荐算法、次级分类和完整推荐条目继续保存在播客扩展中。

网易云 `catalog=category_hot` 要求数字 `category_id`，固定使用 WeAPI `/api/djradio/hot` 并提交参考字段 `cateId/limit/offset`。上游可能在请求窗口外插入推荐项，实测 `limit=3/offset=0` 返回 8 项；TuneWeave 不截断这些真实条目，返回数量超过 `limit` 时标记 `limit_applied=false`，但下一页仍按上游窗口推进到 `offset+limit`，不能按实际返回项数跳页。可靠的 `count/hasMore` 分别映射为 `total/has_more`，分类 ID 与完整响应保存在分页扩展。

网易云 `catalog=category_featured` 同样要求数字 `category_id`，固定使用无分页参数的 WeAPI `/api/djradio/recommend`。它返回分类精选快照并可能明确 `hasMore=true`，但没有任何可提交的续页参数；统一响应如实保留 `has_more=true`，同时保持 `next_offset=null` 并写入 `continuation_supported=false`，不会虚构可用游标。该接口要求 `offset=0`，不应用统一 `limit`，因此 `total=null/limit_applied=false`。

网易云 `catalog=today_preferred`（也接受 `today`）固定使用 WeAPI `/api/djradio/home/today/perfered`。参考接口使用独立的零基 `page`，所以统一请求也显式保留可选 `page`，不把页码偷换成 offset；该目录要求 `offset=0`、不接受分类筛选，省略 `page` 时提交 0。上游不应用 `limit`，也不返回总数、hasMore 或可验证的下一页，因此分页稳定表达为 `total=null/next_offset=null/has_more=false/limit_applied=false`，实际页码和 `page_control_supported=true` 位于分页扩展。

网易云 `catalog=paid`（也接受 `paygift`）固定使用 WeAPI `/api/djradio/home/paygift/list`，提交参考实现的 `limit/offset/_nmclfl=1` 且不接受分类筛选。响应从 `data.list` 映射，`data.hasMore` 决定下一偏移，接口不提供可靠总数所以 `total=null`。`radioFeeType/feeScope` 映射付费态；`discountPrice` 存在时优先作为成交价，否则使用 `originalPrice`，网易云的分值价格转换成 `Money(amount, CNY)`，两个原始价格字段仍完整保留在播客扩展中。

网易云节目榜 `catalog=popular` 固定使用 WeAPI `/api/program/toplist/v1`，`catalog=trending24_hours`（也接受 `hours/24h`）固定使用 `/api/djprogram/toplist/hours`。两者都映射榜单包装中的节目、排名、上期排名和分数，并保留完整榜单条目及响应；包装层 `programFeeType` 优先补充节目内层付费态，避免内层默认 0 遮住榜单明确的付费值。普通节目榜的参考模块虽提交 `offset`，但真实上游对不同 offset 返回相同窗口，所以统一端点兼容接收该参数却明确返回 `offset=0/requested_offset/offset_submitted=true/offset_applied=false`；24 小时榜直接拒绝非零 offset，并标记 `offset_submitted=false`。两者都声明 `offset_control_supported=false`，没有可验证的续页控制，保持 `next_offset=null/has_more=false/continuation_supported=false`，不会伪造分页。

评论读取与写入共用目标类型和平台边界：`type` 接受 `track/mv/playlist/album/radio_episode/video/event/radio_station`、网易云参考数字 `0..7` 以及写操作一节列出的名称别名；`ref` 决定内容平台，`account` 只选择该平台登录态。`view` 缺省为 `all`，也可取 `hot` 或 `replies`；提供 `parent_comment_id` 而省略 `view` 时自动选择 `replies`。`view=all` 不带 `sort` 时使用普通历史目录及 `limit/offset/before_time_ms`，带 `sort=recommended|hot|time` 时使用现代目录并接受 `page`，只有时间排序接受 `cursor`；`view=hot` 返回热门目录，`view=replies` 要求父评论 ID。`limit` 范围是 1–100。兼容字段包括 `sortType`、`pageSize`、`pageNo`、`before/beforeTime/time`、`parentCommentId` 和 `showInner`，排序数字 `1/99/2/3` 分别映射推荐/推荐/热门/时间。

评论响应把普通、热门、置顶和当前父评论分别放在 `comments/hot_comments/top_comments/current_comment`，不会把不同语义的条目混入同一数组。平台若没有应用请求页大小，TuneWeave 保留真实返回数量，并在 `meta.pagination.extensions.limit_applied=false` 明示；例如网易云现代推荐评论实测请求 2 条仍返回 10 条。事件评论的网易引用必须使用动态接口给出的完整 `A_EV_2_...` thread ID。

批量统计端点的 `type` 使用同一套评论目标名称、别名和数字 `0..7`；`ids` 是逗号分隔的平台资源 ID，兼容单个 `id`，保留顺序与重复项，省略或过滤空项后为空时返回成功空批次。网易云固定使用 WeAPI `/api/resource/commentInfo/list`，账户不是必需，但提供 `account` 后可取得对应点赞态。该平台的视频统计可能把公开哈希转换为内部评论资源 ID；动态统计则要求主资源数值 ID，并把 canonical 目标返回为完整 `A_EV_2_{id}_0`，不能在该端点提交评论目录所用的完整动态 thread ID。调用方应以 `requested_ref` 关联原请求、以 `target` 调用后续评论线程能力。

评论反应路径把反应类型作为可扩展段；平台按读写能力分别声明并只执行自己实际支持的类型。`GET` 的统一输入使用与评论同平台的 `target_user_ref` 指向评论作者。网易云“抱一抱”目录使用 `reaction=hug`，要求登录态，并兼容参考字段 `uid`/`target_user_id`、`pageSize`、`pageNo`、`idCursor`；其两个续页值分别以不透明 `cursor/id_cursor` 接收，并在 `meta.pagination.extensions.next_cursor/next_id_cursor` 返回，调用方不得解析其中本地化日期文本。默认 `limit=100`、`page=1`，`uid` 会按评论资源平台构造成用户引用；同时提交引用和 ID 时两者必须一致。`PUT/DELETE` 分别启用和停用 `reaction`；网易云当前支持 `reaction=like`，精确映射参考 `t=1/0` 两个分支，并使用同一套八种评论目标和动态完整 thread ID。

兼容响应的字段优先级按“首个可用值”而不是“首个存在的键”处理：网易云评论、回复和作者会跳过 `null`、空字符串及零 ID；广播收藏会跳过空对象、空 JSON 包装和 `null` 分页别名；播客主播会跳过空首选 ID/昵称。这样旧摘要字段不会遮蔽后续有效身份或完整资源。

### 媒体与跨平台解析

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/audio/recognize` | `{platform?, account?, fingerprint, duration_seconds}`；指纹最大 131072 字节，时长 1–300 秒 | `AudioRecognition`；命中起点跳过不可解析的首选字段并读取有效兼容值 |
| GET | `/v1/media/cdn` | `platform?`、`account?` | `AudioCdnDispatch`；CDN 根地址、QUIC 节点、相对探活文件及缓存时限 |
| GET | `/v1/tracks/{ref}/files` | `spec/file_type?`、`song_type?`、`media_id/media_mid?`、`account?` | 单项 `AudioFileBatch`；精确文件规格授权，不代替自动音质选择 |
| POST | `/v1/media/files` | `{platform?, account?, items/file_info, default_spec/file_type?}`；每项使用 `{ref|mid, spec/file_type?, song_type?, media_id/media_mid?}` | 1–100 项 `AudioFileBatch`；同平台、保序、保留重复项 |
| GET | `/v1/tracks/{ref}/lyrics` | `account?`、`word_synced/qrc?`、`translated/trans?`、`romanized/roma?`、`singing_annotations/singingAnnotations/annotations?`、`song_type/type?`；引用决定平台 | `Lyrics`；助唱内容和时间戳为强类型字段，QQ 省略选项时保持上游 `false/false/false/false/type=1` 默认 |
| GET | `/v1/tracks/{ref}/lyrics/singing-annotations/availability` | `account?`；引用决定平台 | `SingingAnnotationsAvailability`；独立返回助唱标注是否存在，不从普通或逐字歌词内容反推 |
| GET | `/v1/tracks/{ref}/lyrics/translations/styles` | `account?`；引用决定平台 | `MultiStyleLyricTranslations`；多种翻译风格逐项返回并保留平台顺序、风格 ID、名称、歌词和时间戳 |
| GET | `/v1/tracks/{ref}/lyrics/ai-dictionary/availability` | `account?`；引用决定平台 | `AiLyricDictionaryAvailability`；独立返回 AI 歌词词典是否存在，不从词典详情数组推断 |
| GET | `/v1/tracks/{ref}/lyrics/ai-dictionary` | `account?`；引用决定平台 | `AiLyricDictionary`；保序返回短语、语境解释、原歌词行、翻译歌词行和平台歌词时间戳 |
| GET | `/v1/episodes/{ref}/lyrics` | `account?` | `PodcastEpisodeLyrics`；真实无歌词分支也返回可检查的成功数据 |
| GET | `/v1/episodes/{ref}/stream` | 与歌曲流相同的音质、后端、播放平台、回退、解灰和账户参数 | `PodcastEpisodeStream`；节目、原音频和最终解析资源身份分离 |
| GET | `/v1/episodes/{ref}/stream/redirect` | 同上 | 成功解析节目音频后返回 302，不向客户端暴露账户凭据 |
| GET | `/v1/tracks/{ref}/stream` | `quality?`、`variant?`、`bitrate?`、`immersive_type?`、`playback_platform?`、`fallback?`、`fallback_platforms?`、`unblock?`、`source?`、`account?` | `MediaStream` |
| GET | `/v1/tracks/{ref}/stream/redirect` | 同上 | 成功解析完整统一回退链后返回无缓存 302 |
| GET | `/v1/tracks/{ref}/stream/content` | `quality?`、`variant?`、`bitrate?`、`account?`；不接受二次回退或目标平台参数 | 仅供需要服务端本地处理的 provider 交付真实音频字节；固定 `private, no-store`、`nosniff`、受限音频 MIME/文件名和 512 MiB 上限，其他 provider 返回能力不支持 |
| GET | `/v1/tracks/streams` | `refs` 或 `ids`（兼容 `id`）、`platform?`、同上播放控制参数 | `StreamBatch`；逐项成功或失败，保留输入顺序与重复项 |
| POST | `/v1/tracks/streams` | JSON `{refs?|ids?, platform?, quality?, variant?, bitrate?, immersive_type?, playback_platform?, fallback?, fallback_platforms?, unblock?, source?, account?}` | `StreamBatch`；`refs/ids` 可为字符串或字符串数组 |
| GET | `/v1/tracks/{ref}/download` | `quality?`、`variant?`、`bitrate?`、`account?`；兼容 `level/backend/br` | `MediaDownload`；无可用 URL 仍是可检查的成功数据 |
| GET | `/v1/tracks/{ref}/download/redirect` | 同上 | 有专用下载 URL 时返回 302；否则尝试同音质播放 URL 后再返回 302 |
| GET | `/v1/videos/{ref}` | `kind/type=mv|video`、`account?` | `VideoDetail`，含统一视频信息和平台公布的清晰度 |
| GET | `/v1/videos/details` | `refs` 或 `ids/vids`、`platform?`、`kind/type?`、`account?` | `VideoDetail[]`；1–100 项，同平台、同类型、保序且保留重复项 |
| POST | `/v1/videos/details` | JSON `{refs?|ids/vids?, platform?, kind/type?, account?}`；引用可为字符串、逗号字符串或数组 | `VideoDetail[]`；使用 provider 原生批量能力或统一逐项默认实现 |
| GET | `/v1/videos/{ref}/stats` | `kind/type=mv|video`、`account?` | `VideoStats` |
| GET | `/v1/videos/{ref}/stream` | `kind/type=mv|video`、`resolution/res?`、`account?` | `VideoStream`；默认请求 1080，允许无可用 URL 的业务成功态 |
| GET | `/v1/videos/streams` | `refs` 或 `ids/vids`、`platform?`、`kind/type?`、`resolution/res?`、`account?` | `VideoStream[]`；1–100 项，同平台、保序且保留重复项 |
| POST | `/v1/videos/streams` | JSON `{refs?|ids/vids?, platform?, kind/type?, resolution/res?, account?}`；引用可为字符串、逗号字符串或数组 | `VideoStream[]`；使用 provider 原生批量能力或统一逐项默认实现 |
| GET | `/v1/videos/{ref}/stream/redirect` | 同上 | 有可用 URL 时返回 302，否则返回 404 |
| GET | `/v1/videos/{ref}/parts` | `kind/type=video`、`account?`、`limit?`、`offset?` | `VideoPart[]`；稳定 CID、规范父视频引用与统一分页 |
| GET | `/v1/videos/{ref}/playback` | 必填 `part`，`kind/type=video`、`audio_language/cur_language?`、`account?` | `VideoPlaybackManifest`；完整 DASH/DURL、多编码和音轨清单 |
| GET | `/v1/videos/{ref}/audio-stream` | `part?`、`kind/type=video`、`quality?`、`codec?`、`audio_language/cur_language?`、`account?` | `VideoAudioStream`；选择单条音轨并显式返回实际等级、降级、主/备用 URL 和媒体请求头 |
| GET | `/v1/videos/{ref}/audio-stream/redirect` | 同上 | 对选中的音轨返回无缓存 302；不代传媒体请求头 |
| GET | `/v1/videos/{ref}/audio-download` | 同上 | `VideoAudioStream`；原始音轨下载语义别名，不代理媒体字节 |
| GET | `/v1/videos/{ref}/audio-download/redirect` | 同上 | 对下载音轨返回无缓存 302；不代传媒体请求头 |
| GET | `/v1/videos/{ref}/video-stream` | `part?`、`kind/type=video`、`quality/resolution/res?`、`codec=avc|hevc|av1?`、`account?` | `VideoTrackStream`；选择单条 DASH 视频轨并保留实际质量、动态范围、编码、帧率、SegmentBase 与降级状态 |
| GET | `/v1/videos/{ref}/video-stream/redirect` | 同上 | 对选中的视频轨返回无缓存 302；不代传媒体请求头 |
| GET | `/v1/videos/{ref}/video-download` | 同上 | `VideoTrackStream`；原始视频轨下载语义别名，不代理媒体字节 |
| GET | `/v1/videos/{ref}/video-download/redirect` | 同上 | 对下载视频轨返回无缓存 302；不代传媒体请求头 |
| GET | `/v1/videos/{ref}/subtitles` | 必填 `part`，`kind/type=video`、`account?` | `VideoSubtitleList`；稳定字幕身份、登录要求和不含临时资源 URL 的语言目录 |
| GET | `/v1/videos/{ref}/subtitles/{subtitle_ref}` | 必填 `part`，`kind/type=video`、`account?` | `VideoSubtitleDocument`；强类型样式和毫秒字幕段，不公开临时正文 URL |

酷狗公开歌曲搜索使用 `GET /v1/search?platform=kugou&kind=track&q=...`。当前实现固定访问官方 HTTPS Web 搜索端点，不接受账户、Cookie、URL、请求头或代理覆盖；部署方如需代理只能在启动时设置 `TUNEWEAVE_KUGOU_PROXY`。统一歌曲引用优先使用平台 `album_audio_id`，基础媒体 `hash`、各音质哈希、原始权益码与搜索后端保存在有界扩展中；`playable` 在接入真实播放权益检查前保持 `null`，不会根据搜索列表猜测会员歌曲可播。任意 `offset` 由最多两个固定 100 项上游页精确切片，分页扩展会声明实际后端和上游页宽。

酷狗歌曲详情使用 `GET /v1/tracks/kugou:<album_audio_id>`，只接受规范正整数 `album_audio_id` 且当前公开层不接受账户参数。实现先通过固定 HTTPS Android 网关取得歌曲、歌手、专辑和真正的 `audio_id`，再以该 `audio_id` 查询基础、高品、无损、Hi-Res 与母带媒体规格；每段响应都必须与上一段身份严格一致，避免把外形相同但语义不同的数字 ID 混用。发行日期、语言、版本、分类、平台发布标记和各规格哈希、大小、码率、时长进入强类型歌曲及有界扩展；发布标记不等同于播放权益，因此播放链接入前 `playable` 仍为 `null`。

酷狗歌词使用 `GET /v1/tracks/kugou:<album_audio_id>/lyrics`，支持统一 `word_synced/qrc`、`translated/trans` 与 `romanized/roma` 展示选项，不接受账户、歌曲类型或助唱标注参数。实现先复用歌曲详情形成稳定的 `album_audio_id + hash + duration + keyword` 候选搜索，再优先选择平台 proposal，并分别下载 KRC 与 LRC；候选 `accesskey` 只存在于当前请求内，不进入响应、日志或错误。KRC 必须通过 `krc1` 文件头、循环 XOR、受限 zlib 解压和 UTF-8 校验，嵌入语言目录中的原生 `type=1/0` 分别映射为翻译和音译；普通 LRC 独立进入 `plain`，KRC 进入 `word_synced` 并决定 `format=krc`，因此普通歌词不会覆盖更高精度逐字歌词。某一格式缺失时保留另一格式，只有两者都不可用才返回失败。

酷狗公开播放和下载分别复用统一 `/v1/tracks/{ref}/stream` 与 `/download` 端点，当前接受默认 variant、匿名账户和普通音质家族；`auto/low/standard` 从 128 kbps 起步，`higher/high`、`lossless`、`hires`、`master` 在目标规格缺失时按明确顺序降级并通过 `actual_quality` 如实返回，显式 bitrate 只接受 1–320000。每次请求先用固定 HTTPS Android 网关校验 privilege，再用同一 `album_audio_id + album_id + hash` 请求 tracker；响应哈希和身份必须回配。平台返回的 HTTP CDN 地址不会原样暴露，仅在主机属于受信 `*.kugou.com` 且无凭据、端口和片段时升级为 HTTPS。完整授权返回全曲；只有试听权益时 stream 返回准确 `TrialWindow`，文件大小按字节窗口而非上游误报的全曲大小计算，download 则保持 `available=false` 并附权益诊断，不把试听片段伪装为完整下载。统一 `/download/redirect` 也会拒绝把试听 URL 降级冒充完整下载，而 `/stream/redirect` 在完整执行指定平台、严格匹配和回退链后才返回 302；两者都使用 `private, no-store`。Uni Playlist 的安全快照不保存酷狗哈希等 provider 私有字段，因此播放时会以稳定 `album_audio_id` 重新补全当前媒体元数据。当前公开层不接受账户、非默认 variant 或沉浸式参数。

酷狗公开歌单使用 `GET /v1/playlists/kugou:<global_collection_id>` 与 `/tracks`，只接受以 `collection_` 开头的规范公开集合身份且不接受账户。元数据固定访问 Android `/v3/get_list_info`，歌曲固定访问公开 `/pubsongs/v2/get_other_list_file_nofilt`；统一 `offset` 直接传递为平台 `begin_idx`，因此任意 offset 都形成连续窗口，不受参考项目页码换算和页宽对齐限制。平台只在首段返回完整 `list_info`，实现会在字段存在时严格复核集合身份，同时接受后续段有意返回的空对象。歌曲当前身份使用 `mixsongid` 对应的 `album_audio_id`，与其不同的历史 `add_mixsongid` 仅作为诊断保留；标准、高品、无损与 Hi-Res 规格均映射为强类型歌曲能力。歌单封面仅接受官方 `*.kugou.com` 与 `*.kgimg.com` HTTPS 地址，HTTP 地址只在同一受信主机上升级。Uni Playlist 可用 `{ "platform": "kugou", "type": "playlist", "id": "<global_collection_id>" }` 导入该来源；服务端托管导入和客户端托管来源展开共用同一 provider 契约，完整遍历保持平台顺序与重复项。真实 HTTP 已验证任意非对齐分页，并将一份 91 首公开歌单完整导入后成功播放首项。

酷狗设备身份属于 provider 内部状态，不是账户或调用方凭证。全新部署在 `TUNEWEAVE_DATA_DIR/kugou-device.json` 创建符合 UUID v4 的匿名 GUID，并以其 MD5 的无符号 128 位十进制值确定性派生 MID；Web 搜索只复用稳定 MID，不为此提前注册。首次歌曲详情、歌单、歌词、权益或播放等移动端请求会固定访问 `https://userservice.kugou.com/risk/v2/r_register_dev`，以内部 AES-CBC 档案、RSA PKCS#1 v1.5 密钥包和完整 Android 签名取得 24 位 `dfid`，随后将 GUID、MID、`dfid` 和注册时间原子保存并在重启后复用。并发首次请求只执行一次注册。若本地 `dfid` 已损坏，或平台对已注册但本地遗失 `dfid` 的当前 GUID 返回成功空数据，provider 会使该注册失效；后一分支等待 1 秒冷却，轮换整套匿名身份并只重试一次，第二次仍无 `dfid` 时明确失败，不无限重试。GUID/MID 派生关系损坏时拒绝启动，API 请求不能覆盖设备文件、设备字段、注册地址、签名、请求头或代理。

咪咕公开歌曲搜索使用 `GET /v1/search?platform=migu&kind=track&q=...`。Provider 固定访问官方 HTTPS `bmw/search/song/v1.0`，不接受账户、Cookie、目标 URL、请求头或请求级代理；部署方只能通过 `TUNEWEAVE_MIGU_PROXY` 配置服务端正向代理。统一歌曲引用使用稳定 `migu:<contentId>`，并强类型保留 `songId`、`copyrightId`、`resourceType`、歌手、专辑、时长、官方图片和普通/逐字歌词资源。已确认的 `PQ/HQ/SQ/ZQ/ZQ24` 分别映射标准、高品、无损和 Hi-Res，其中详情当前可能以 `ZQ` 表示搜索结果中的 `ZQ24`；`AV3A/Z3D` 等语义尚未验证的格式仅按平台原码保存在有界规格列表，不能伪装为统一沉浸音质。搜索结果中的 `more` 作为独立替代版本记录保留，不扁平为额外命中；搜索权益标志不足以证明当前可播，因此 `playable` 保持 `null`。平台实际页宽固定为 20，统一 offset/limit 通过最多 6 个连续上游页精确切片，既支持非页宽对齐窗口，也不会无限翻页；分页元数据明确返回实际后端、物理页宽和抓取页数。

咪咕歌曲详情使用 `GET /v1/tracks/migu:<contentId>`，只接受 1–64 位规范 ASCII 字母数字 `contentId`，当前公开层不接受账户。Provider 固定访问官方 HTTPS `MIGUM2.0/v1.0/content/resourceinfo.do`，并要求成功响应恰好包含一条 `resourceType=2` 的歌曲、其 `contentId` 与请求完全一致且 `copyrightId` 合法；空目录返回资源不存在，多条、错位或类型漂移视为上游错误。详情返回平台当前别名、歌手、专辑、可信官方封面、时长、关联 MV、标签、统计、歌词资源、试听窗口、VIP/下载标志及关联资源；外部 URL 仍只允许固定 `d.musicapp.migu.cn/data/oss/` HTTPS 路径。平台同时返回的 `rateFormats` 和 `newRateFormats` 分开强类型保留，可用音质从两者并集计算，因此新版列表不会遮掉旧列表独有的 LQ；已确认的 LQ/PQ/HQ/SQ/ZQ/ZQ24 映射统一音质，其他格式只保留平台原码。详情中的有效、VIP 或试听标志都不足以证明当前请求可播放，只有平台明确返回资源或素材失效时才设 `playable=false`；实时可听性和播放权益由播放链负责。

咪咕歌词使用 `GET /v1/tracks/migu:<contentId>/lyrics`，当前公开层不接受账户、`song_type` 或助唱标注参数。Provider 从严格歌曲详情取得 LRC、MRC 和 TRC 资源，只允许固定咪咕 HTTPS 媒体域名及 `/data/oss/` 路径；三种格式并发下载、独立记录成功或失败，每个响应限制为 4 MiB。MRC 密文按平台 64 位有符号分组算法解密为 UTF-16LE，并要求同时存在行时间和逐字时间。MRC 存在时 `format=mrc` 且完整内容位于 `word_synced`，普通 LRC 仍独立保存在 `plain`；只有 LRC 缺失或下载失败时才从 MRC 派生行级歌词。TRC 独立映射为 `translated`，不存在时不伪造翻译；诊断只包含格式、内容类型、字节数和错误分类，不回显歌词地址。

咪咕实时权益使用 `GET /v1/tracks/migu:<contentId>/availability`，固定调用 HTTPS `strategy/pc/can-listen/v1.0`，要求响应恰好包含同一 `contentId`，并把 `canListen` 与 `limitLength` 分开保留；`playable=false` 且 `limit_length=true` 表示只能试听，不等于完全没有媒体。公开播放和下载使用统一 `/stream` 与 `/download`。每次请求先刷新严格资源详情和版权身份，再调用 `strategy/listen-url/h5/v2.4`；参考项目的 v1 当前只返回权益数据，匿名 v2 当前返回成功空 URL，因此都不作为虚假的备用媒体链。H5 `AB CD 01` 二进制信封在大小上限内解密并强类型解析，返回歌曲身份若存在必须与请求回配。`auto` 从目录已知最高规格发起，`low/standard`、`higher/high`、`lossless`、`hires` 分别选择 `PQ/HQ/SQ/ZQ24`，显式 bitrate 只接受 1–320000；平台匿名链当前会把高档请求降为 PQ，响应必须以 `requested_quality` 和 `actual_quality=standard` 如实表达，不凭目录规格伪造高品质。

咪咕媒体 URL 只接受 `freetyst.nf.migu.cn` 的 HTTPS 标准端口、`/public/product8th/product` 或 `/public/product9th/product` 路径，并要求 `Tim`、`Key`、`playSessionId` 三个授权字段各恰好出现一次；不接受凭据、片段、其他主机、端口或目录。平台没有给出可验证的过期语义，`Tim` 不能被猜成 `expires_at`，所以当前保持 `null`。完整授权的流可用于统一下载；当 `canListen=false`、`limitLength=true` 且平台返回完整起止窗口时，stream 明确携带 `TrialWindow`，download 则返回 `available=false`、`url=null` 并记录已隐藏试听 URL，`/download/redirect` 也不会把试听流冒充完整文件。

咪咕已参与统一 resolver、歌曲播放、Uni Playlist 播放和媒体跳转。调用方可用 `playback_platform=migu`、`source=migu` 或 `fallback_platforms` 明确选择顺序；默认回退也会在前序来源失败后尝试咪咕。跨平台解析仍使用标题、歌手、专辑、时长和版本标签的严格评分，成功响应保留原始引用、实际咪咕引用、全部尝试、匹配分数、平台真实音质及试听窗口。调用方托管的 `migu:` Uni 项经 `/v1/uni/items/stream` 无状态验证和播放，不需要先写入服务器。完整流和下载可经对应 `/redirect` 得到无缓存 302；试听资源只能用于 stream，下载跳转返回 403 且不包含 `Location`。真实统一 HTTP 已验证网易云歌曲精确回退到咪咕、调用方托管 Uni 播放、完整媒体跳转和受限试听拒绝下载。

咪咕公开歌单使用 `GET /v1/playlists/migu:<musicListId>` 与 `/tracks`，只接受规范正整数 `musicListId` 且不接受账户。元数据固定访问 `resource/playlist/v2.0`，要求 `resourceType=2021` 和返回 ID 与请求完全一致；标题、简介、可信官方封面、创建者、曲数、发布时间、标签、核心统计及安全过滤后的沉浸展示分别进入稳定字段或类型化扩展。歌曲固定访问 `MIGUM3.0/resource/playlist/song/v2.0`；真实上游会把大于 50 的 `pageSize` 静默压为 50，因此 Provider 固定使用 50 首物理页，以最多 3 个连续页实现统一 `limit=1..100` 与任意 `offset`，并在跨页时复核总数和发布时间。每首歌仍使用稳定 `contentId`，保存真实音质、版权和展示元数据，额外标记全局歌单位置；顺序与重复出现均不折叠。Uni Playlist 可用 `{ "platform":"migu", "type":"playlist", "id":"<musicListId>" }` 完整导入 Server 模式，或经 `/v1/uni/materialize/imports` 在 Client 模式完整展开后只返回请求页。真实统一 HTTP 已将一份 195 首公开歌单完整导入并播放首项，同时验证 Client 模式不创建服务器歌单。

咪咕公开音源协议不需要账户、设备注册或用户签名。歌曲搜索、歌单详情、歌单歌曲、资源详情、实时权益和 H5 播放六个 API 入口均固定为官方 HTTPS 域名和标准端口；H5 二进制信封使用平台公开客户端协议常量解密响应，不是用户凭据，也不能由请求覆盖。Provider 禁用重定向，连接和整次请求分别限制为 10 秒与 20 秒；普通 API 最多读取 8 MiB，歌词最多 4 MiB，声明长度与分块累计长度都会独立执行上限。429 映射为可重试 `rate_limited`，5xx 为可重试上游错误，其余 4xx 不重试；业务码、结构漂移、身份错位和权限不足保持不同失败语义。请求不能提交目标 URL、Cookie、任意头或请求级代理，部署方只有 `TUNEWEAVE_MIGU_PROXY` 可配置服务端代理。全新数据目录下的全部真实网络用例已通过，免费歌曲实际从受信 CDN 读取 1 KiB 媒体，会员歌曲保持 65–125 秒试听，非法传输覆盖与非法歌单身份均在联网或媒体跳转前返回 400。

酷我公开歌曲搜索使用 `GET /v1/search?platform=kuwo&kind=track&q=...`。旧参考项目的 `/api/www/search/searchMusicBykeyWord` 已被平台拒绝，Provider 固定访问当前官网实际使用的 HTTPS `/search/searchMusicBykeyWord`，不接受账户、Cookie、目标 URL、请求头或请求级代理；部署方只能通过 `TUNEWEAVE_KUWO_PROXY` 配置服务端正向代理。统一身份要求上游 `MUSIC_<rid>` 中的 `rid` 为规范正整数，并返回 `kuwo:<rid>`；歌手、专辑、别名、时长、MV、目录权益和媒体规格以强类型结构保存，不携带旧响应中的不可信媒体标签。已确认的 `s/h/p/ff/hr/dtsx` 分别映射低、标准、高品、无损、Hi-Res 与环绕音质，语义尚未验证的 `zply/zpga*` 等级只保留平台原码。目录在线和付费标志不足以证明当前匿名请求可播，因此只有明确离线时才设置 `playable=false`。上游页码从 0 开始且已真实接受固定 100 项页宽；统一任意 offset 和 `limit=1..100` 最多读取两个连续页并复核总数。

酷我歌曲详情使用 `GET /v1/tracks/kuwo:<rid>`，只接受规范正整数且公开层不接受账户。当前官网对 `/api/www/music/musicInfo` 要求当次匿名跟踪 Cookie 和由该 Cookie 动态生成的 `Secret`；Provider 固定先访问官方 HTTPS 首页取得严格限定名称、长度与字符集的 Cookie，只在当前实例内缓存最多 30 分钟，再使用官网算法生成带随机 8 位 nonce 的单次签名。Cookie、Secret 和 nonce 不持久化、不回显，也不进入普通日志、Debug、扩展或错误；只有 HTTP 401/403 或平台精确签名拒绝状态才刷新会话并重试一次。成功响应必须同时以 `MUSIC_<rid>` 和数字 `rid` 回配请求身份，随后映射歌曲、歌手、专辑、可信官方封面、时长、曲序、发行日、MV、无损目录、评分、付费/试听标志与有界专辑简介。目录中的 `online/isListenFee/payInfo` 不替代实时播放权益，只有明确离线时才设 `playable=false`。当前签名实现已用官网 JavaScript 算法固定向量做差分验证，并通过真实 Provider 与统一 HTTP。

酷我歌词使用 `GET /v1/tracks/kuwo:<rid>/lyrics`，当前公开层不接受账户、`song_type` 或助唱标注参数。Provider 同时请求官方 HTTPS LRCX 二进制端点和移动端行级端点：LRCX 的单一查询先把固定协议文本按 `yeelion` 循环 XOR 并做 Base64，响应必须通过 `tp=content` 信封、4 MiB 传输上限、8 MiB zlib 解压上限、Base64/XOR 及无替换 GB18030 解码。合法 `<start,duration[,offset]>` 逐字标记原样保存在 `word_synced`，有该来源时始终返回 `format=lrcx`；移动端行时间按真实浮点秒四舍五入到毫秒并组成普通 LRC。移动端当前存在间歇性“音乐查询失败”，该低精度来源失败时从 LRCX 只移除语法有效的逐字标记派生 `plain`，不会反过来降级或覆盖 `word_synced`；每条链独立记录安全诊断，只有两者均失败才返回错误。当前公开响应未提供可验证的翻译或音译时，对应字段保持 `null`。统一 HTTP 已真实确认逐字优先、普通歌词回退和上游端点/不透明查询不泄漏。

酷我实时权益、公开播放和下载分别使用统一 `/v1/tracks/kuwo:<rid>/availability`、`/stream` 与 `/download`。三者固定调用当前官网签名后的 HTTPS `/api/v1/www/music/playUrl`，只提交平台实际签发匿名全曲的 `128kmp3` 档；真实对照确认 `br=320kmp3`、`2000kflac` 和省略 `br` 都返回同一 128 kbps MP3，因此目录中即使存在高品或无损规格，匿名响应仍如实返回 `actual_quality=standard`、`bitrate=128000`，并保留调用方的 `requested_quality`。`code=200` 且 URL 通过校验才表示完整全曲；`code=-1` 映射匿名权限拒绝，stream 返回 403，download 返回可检查的 `available=false/url=null`；`code=-1001` 表示资源不可用。当前匿名链未返回试听地址或窗口，不能虚构 `TrialWindow`。媒体 URL 只接受 HTTPS 标准端口、单标签 `*-sycdn.kuwo.cn` 主机、无凭据/查询/片段且以 `.mp3` 结尾的路径，服务端签名 Cookie、`Secret`、请求 ID 和播放端点均不回显。公开层不接受账户、非默认 variant、沉浸式参数或超出 `1..10000000` 的 bitrate；高于公开档位的合法音质或码率请求允许透明降级，实际结果始终单独报告。

酷我已参与统一 resolver、歌曲播放、Uni Playlist 播放和媒体跳转。默认跨平台顺序为网易、QQ、酷狗、酷我、咪咕；调用方也可用 `playback_platform=kuwo`、`source=kuwo` 或 `fallback_platforms` 显式指定。跨平台搜索候选继续以标题、歌手、专辑、时长和版本标签严格评分，成功流保留原始引用、实际酷我引用、匹配分数、每次尝试以及平台真实音质。调用方托管的 `kuwo:` Uni 项可经 `/v1/uni/items/stream` 无状态播放，不创建服务器歌单。完整流和下载可从相应 `/redirect` 获得 provider 已验证的无缓存 302；付费拒绝不会产生 `Location`。真实统一 HTTP 已将网易“好运来”精确匹配为酷我免费全曲，验证显式来源、Uni Client 模式、两个 HTTPS CDN 跳转和付费下载 403；服务器测试固定覆盖默认顺序和同一契约。

酷我公开歌单使用 `GET /v1/playlists/kuwo:<pid>` 与 `/tracks`，只接受规范正整数 PID 且不接受账户。Provider 固定调用当前官网动态签名后的 HTTPS `/api/www/playlist/playListInfo`，成功响应必须回配同一歌单 ID，并把标题、描述、可信官方封面、创建者、曲数、标签、收听数和官方标志映射到统一字段或类型化扩展；`code=-1` 且没有 data 表示不存在或不公开。歌曲页固定使用 100 首物理页，以最多两个连续请求实现统一 `limit=1..100` 与任意 offset，跨页复核歌单身份和总数；每首歌曲必须同时回配 `MUSIC_<rid>` 与数字 `rid`，并记录全局歌单位置，顺序和重复项均不折叠。官网大页偶发 504，因此同一页只允许对可重试的传输、429 或 5xx 等待 250 ms 后补发一次；业务拒绝、身份漂移和结构错误不重试，也不使用参考项目的递归或共享计数。

公开酷我歌单可通过 `{ "platform":"kuwo", "type":"playlist", "id":"<pid>" }` 导入 Uni Playlist。Server 模式完整遍历后原子保存，Client 模式同样先验证完整来源再按统一 offset/limit 返回无状态项目，且不会创建服务器记录；两者都保持来源顺序、重复项和每项稳定身份。真实统一 HTTP 已完整导入一份 69 首用户公开歌单，Client 模式只返回所请求的位置 68 且服务器目录仍为空；另一份 318 首官方歌单以 `offset=99&limit=3` 跨两个物理页返回连续位置 99–101。服务端防回归测试另以重复来源确认两个 Uni 项拥有不同 item ID。

酷我公开音源协议的首页、搜索、详情、播放、歌单、LRCX 和移动歌词入口均是编译期固定的官方 HTTPS 标准端口，客户端禁止重定向，连接和整次请求分别限制为 10 秒与 20 秒。普通 API 最多读取 8 MiB，歌词传输最多 4 MiB，LRCX 解压最多 8 MiB；声明长度、分块累计长度和解压累计长度分别执行上限。429 映射为可重试限流，5xx 映射为可重试上游错误，其余 4xx 不重试；业务拒绝、身份漂移和结构错误不会伪装成传输失败。请求不能覆盖目标 URL、Cookie、任意头、账户或请求级代理，部署方只有 `TUNEWEAVE_KUWO_PROXY` 可配置服务端代理；配置及客户端 Debug 不输出代理、匿名 Cookie、动态签名或会话状态。全新数据目录下的统一 HTTP 已覆盖搜索、详情、逐字歌词、免费与付费权益、播放、跨页歌单、Client Uni 和非法输入，并从受信 CDN 通过 HTTP 206 读取 1 KiB `audio/mpeg` 验证真实媒体，不下载整首或记录签名 URL。

汽水音乐公开歌曲搜索使用 `GET /v1/search?platform=soda&kind=track&q=...`。Provider 固定访问官方 HTTPS PC 歌曲搜索入口并只发送查询词、20 首物理页游标和平台固定 `aid=386088`；真实消融确认该链不需要 Cookie、匿名设备、`x-helios` 或 `x-medusa`，因此不会虚构或持久化无效设备状态。统一歌曲引用使用规范正整数 ID 形成 `soda:<track_id>`，并强类型返回歌手、专辑、可信官方封面、时长以及 `medium/higher/highest/lossless/hires/spatial` 目录规格；搜索本身不额外触发实时权益请求，因此搜索结果的 `playable` 保持 `null`，由权益或播放端点实时判定。上游实际页宽固定为 20，统一任意 `offset` 与 `limit=1..100` 最多连续读取 6 页；分页不猜测总数，并返回实际物理页宽、抓取页数和后续游标。当前公开搜索不接受账户、非默认 variant、平台搜索状态、Cookie、目标 URL、请求头或请求级代理；部署方只有 `TUNEWEAVE_SODA_PROXY` 可配置服务端代理。客户端禁用重定向，连接和整次请求分别限制为 10 秒与 20 秒，声明长度和分块累计长度分别受 8 MiB 上限约束，429 与 5xx 保持可重试分类。真实官方搜索已验证目标歌曲身份和连续分页。

汽水歌曲详情使用 `GET /v1/tracks/soda:<track_id>`，公开层不接受账户。Provider 固定调用官方 HTTPS 匿名 `track_v2`，只提交规范歌曲 ID、`media_type=track`、固定 `aid=386088`、`device_platform=web` 和 `channel=pc_web`；真实消融确认无需 Cookie、设备或签名。响应必须回配同一歌曲 ID 和媒体类型，并以强类型映射歌曲、歌手、专辑、可信封面、时长、目录音质、统计、词曲作者、高潮及首唱时间片、语言、标签、分享平台和卡拉 OK 状态；结构数量和文本长度均受限。`canonical_share_url` 只由已验证 ID 构造，详情只在平台明确离线时设置 `playable=false`，目录权益仍不能冒充实时播放授权。上游同一响应携带的歌词留给独立歌词能力处理，`url_player_info`、`video_model`、临时媒体主机和令牌不会进入详情、扩展或日志；权益、播放和内容交付每次都重新获取短时授权。

汽水歌词使用 `GET /v1/tracks/soda:<track_id>/lyrics`，公开层不接受账户、`song_type` 或助唱标注参数。Provider 刷新同一匿名 `track_v2` 并严格回配歌曲 ID 与媒体类型，把平台 `[行起点,行时长]<相对字偏移,字时长,保留字段>` 毫秒格式保存在 `word_synced`，同时从相同已验证行派生带三位毫秒时间戳的 `plain`；`format=krc` 始终由逐字轨决定，普通歌词不能覆盖或截断高级轨。每一行、每个字标签、顺序和相对时间范围都会验证，正文限制 4 MiB、最多 20,000 行且每行最多 2,000 个时序单元；任一逐字结构损坏会返回明确上游错误，不会静默降级为普通文本。与官方分享页 `lyrics.sentences` 的真实差分确认 26 行正文和 214 个字的文本、绝对起止时间完全一致；页面额外两行前置信息不混进演唱正文，词作者通过独立 contributor 表达，页面末句的 `Number.MAX_SAFE_INTEGER` 展示哨兵也不会污染真实末字结束时间。当前公开响应没有独立翻译或音译，因此对应字段保持 `null`；扩展只记录计数和时间单位，不含歌词请求中的临时 player 数据。真实统一 HTTP 已确认普通与逐字轨同时存在，非法账户和平台外参数均在联网前返回 400。

汽水实时权益使用 `GET /v1/tracks/soda:<track_id>/availability`，接受 `bitrate=1..10000000` 且公开层不接受账户。Provider 每次刷新匿名 `track_v2`，严格解析内嵌 player 模型并校验平台成功码、歌曲媒体 ID、音频类型、SSL 标志、实际时长、媒体规格、单次有效期及固定 `*-luna.douyinvod.com` HTTPS 主机；`backup_url` 当前既可能是单字符串也可能是数组，两种官方形态由强类型联合字段兼容。player 媒体 ID 与歌曲 `vid` 一致且时长回配时表示完整授权；与 `preview/audition_info` 的 VID、起点和时长一致时表示试听，不能仅凭目录 `only_vip_playable` 猜测。真实免费样本返回完整 115.357 秒加密媒体并按请求码率选择实际档位；会员样本只返回从 107904 ms 开始的 60001 ms 试听。两者当前均为 `cenc-aes-ctr`，响应只公开安全媒体规格、音质、试听窗口、有效期及 `requires_local_decryption=true`，不会回显主备 URL、`kid`、`spade_a`、文件 ID、哈希、player 模型或临时令牌。

汽水公开播放使用统一 `/v1/tracks/soda:<track_id>/stream`，下载使用 `/download`，真实字节由 `/stream/content` 在服务端交付。公开层接受默认 variant、普通音质或 `bitrate=1..10000000`，不接受账户和沉浸式参数；`surround/dolby/master` 不会被静默降级，`auto/low/standard/higher/high/lossless/hires/spatial` 映射为明确的目标码率并以 `actual_quality` 如实返回所选档位。JSON 只携带无秘密的本站相对内容 URL，不公开短时 CDN 地址、KID 或 `spade_a`；内容请求重新刷新授权，按主链和最多四个备链顺序尝试固定受信 HTTPS CDN，拒绝重定向，要求 HTTP 200 和授权声明的精确字节长度，并受 512 MiB 上限约束。加密媒体在内存中校验 KID、MP4 样本映射和 CENC AES-CTR 后输出 M4A，FLAC 分支重组为原生 `fLaC`；未加密媒体也必须与声明 codec 和容器一致。完整公开曲可用于统一下载；仅试听流保留精确 `TrialWindow`，下载明确返回 `available=false/url=null`，不能把试听片段冒充整曲。`/stream/redirect` 与完整 `/download/redirect` 可以安全跳转到同源相对内容端点，试听下载跳转返回 403。真实统一 HTTP 已验证完整与试听 AAC 均返回 `audio/mp4`、长度一致并可由 FFmpeg 完整解码；另以网易云来源曲严格匹配到同名、同歌手且时长只差 64 ms 的汽水公开曲，保留来源引用、目标引用、0.9 匹配分和成功轨迹，302 后的跨平台替代音频同样通过完整解码。

汽水公开歌单使用 `GET /v1/playlists/soda:<playlist_id>` 与 `/tracks`，只接受规范正整数 ID 且不接受账户。Provider 固定调用官方匿名 HTTPS `/luna/pc/playlist/detail`，只提交 ID、原始 cursor、`count=100`、固定 `aid=386088`、`device_platform=web` 和 `channel=pc_web`；参考实现使用的 `cnt` 当前会被平台忽略并一次返回整表，因此不沿用。详情强类型保留标题、描述、可信封面、创建者显示名、可见曲数、收藏/分享/评论统计、审核状态、排序类型、创建/更新时间和原始资源数；响应必须回配同一歌单 ID，平台码 `1000005` 映射为不存在或不公开。上游 cursor 按原始位置推进，受限、删除或过滤项会造成一页少于 100 首：真实歌单的 506 个原始位置只产生 398 首可见曲，因此统一 offset 必须从起点按 cursor 顺序累计可见曲，不能直接透传，也不能按歌曲 ID 去重。每页核对可见总数、原始资源数和更新时间，cursor 必须前进且不得重复，单次统一窗口最多扫描 128 个物理页；返回曲目携带从 0 开始的稳定可见位置。`{ "platform":"soda", "type":"playlist", "id":"<playlist_id>" }` 可直接用于 Server 或 Client Uni Playlist 导入；真实验收中跨页窗口 `offset=87&limit=4` 连续返回位置 87–90，两种导入均得到完整 398 项，Client 物化只返回请求的末项位置 397 且不创建服务器记录。

汽水公开专辑使用 `GET /v1/albums/soda:<album_id>` 与 `/tracks`，只接受规范正整数 ID 且不接受账户。Provider 固定访问 `https://www.qishui.com/share/album?album_id=...`，禁用重定向、限制 HTML 为 8 MiB，并以带字符串转义和 128 层深度上限的扫描器只提取 `_ROUTER_DATA` 顶层 JSON；页面的请求头、日志器和其他 SSR 元数据不会进入统一响应。专辑 ID、名称、艺人、发行公司、说明、封面、发行时间、声明曲数及每首歌曲的专辑身份均需回配，`hasError=true` 映射为资源不存在；曲目表最多 10,000 项且必须与声明曲数一致，保留原始顺序和重复项并以 `album_position` 标明零基位置。官方分享页当前返回完整专辑快照，因此统一 `limit=1..100` 与任意 offset 在验证完整快照后本地切片，不虚构上游分页。`{ "platform":"soda", "type":"album", "id":"<album_id>" }` 可作为 Uni Playlist 来源；真实 38 首专辑已通过详情、`offset=37` 末曲、Client 物化和 Server 原子持久化导入验收，导入结果完整保留 38 项及 `type=album` 来源摘要。

为兼容参考项目调用方，音频识别请求也接受 `audio_fp`/`audioFP` 作为 `fingerprint` 的别名、`duration` 作为 `duration_seconds` 的别名；响应只使用统一字段名。

助唱标注存在性是与歌词正文分离的目录能力。QQ 数字歌曲 ID 直接提交；MID 先通过歌曲详情解析真实数值 ID，再固定调用 `GetSingingAnnotationsInfo` 的 `needNum=false` 布尔分支。响应保留请求引用作为 `track_ref`，并在扩展中提供 `numeric_id` 和平台原始数据；省略平台标志时按上游语义返回 `available=false`，畸形标志不会被当作不存在。

多风格翻译不压缩进普通歌词的单个 `translated` 字符串。QQ 端点同样支持数字 ID 和 MID，固定调用 `BatchGetMultiStyleTransLyric`；`lyrics` 保留平台顺序与重复项，每个条目的 `style/style_name/lyric/timestamp` 独立建模和解密。缺失列表按平台语义返回空数组，但容器、条目字段或任一非空密文畸形时整次请求失败，不把密文或不完整条目伪装成可显示歌词。

AI 歌词词典存在性与词典详情是两个独立能力。QQ 数字 ID 或由 MID 解析的数值身份固定调用 `IsAIDictExists`，`available` 只来自平台 `exists` 标志；字段缺失按平台默认 `false`，畸形布尔值返回上游错误，不能用后续详情是否为空反推或覆盖。

AI 歌词词典详情固定调用 QQ `GetAIDictInfo`。统一 `entries` 保留 `dictList` 的顺序与重复项，并把 `phrase/explanation/lyric_text/translated_lyric_text/lyric_timestamp` 分开建模；平台时间戳保持不透明字符串，不擅自改成毫秒。缺失列表和条目内缺失字符串按平台参考默认返回空值，已出现但类型错误的容器或字段则拒绝，未知字段及完整原始响应保存在扩展中。

CDN 调度是播放地址解析的底层目录能力，不代替歌曲流端点。`roots` 保留平台给出的顺序和重复项，调用方可把相对 `test_file` 拼接到候选根地址做连通性探测；`nodes` 额外表达 QUIC、IP 栈和主机信息，`expires_in_seconds/refresh_after_seconds/cache_for_seconds` 决定目录生命周期。QQ 每次请求使用新的 GUID，固定启用新域名与 IPv6；TuneWeave 只接受无嵌入凭据的 HTTP(S) 根地址，并拒绝绝对探活 URL、空目录、非正计时及业务失败，完整平台响应仍保存在扩展供诊断。

精确文件端点用于不能被统一音质枚举完整表达的 QQ 文件能力。`AudioFileAccess` 明确返回 `spec/filename/relative_url/access_token/decryption_key/available/encrypted/format/codec/bitrate/quality/platform_code`；`relative_url` 仍需与 `/v1/media/cdn` 的根地址拼接，加密文件需要调用方使用 EKey 解密。不可用文件不是整批失败，`available=false` 与真实 `platform_code` 可逐项检查；但成功码缺 URL、加密成功缺 EKey、返回项错位或不安全绝对 URL 会作为上游错误拒绝。批量引用必须属于同一平台，`ref` 与 `mid` 同时提供时必须一致，输入顺序和重复项原样保留。

QQ 文件规格的稳定整数/名称映射如下。`0..43` 与参考旧 Web API 完全同序；`44` 是 SDK 已公开的 `SpecialSongFileType.TRY_OGG_640`；上游 v0.7 又在末尾加入三个彩铃规格，因此完整范围为 `0..47`。

- `0..16`：`dts_x, master, atmos_2, atmos_5_1, atmos_7_1, dolby_atmos, nac, flac, ogg_640, ogg_320, ogg_192, ogg_96, mp3_320, mp3_128, aac_192, aac_96, aac_48`。
- `17..29`：`encrypted_dts_x, encrypted_vinyl, encrypted_master, encrypted_atmos_2, encrypted_atmos_5_1, encrypted_atmos_7_1, encrypted_dolby_atmos, encrypted_nac, encrypted_flac, encrypted_ogg_640, encrypted_ogg_320, encrypted_ogg_192, encrypted_ogg_96`。
- `30..43`：`trial, accompaniment, multi_track, piano, music_box, guzheng, qudi, hulusi, suona, handpan, electric_guitar, drums, kazoo, therapy`。
- `44`：`trial_ogg_640`。
- `45..47`：`ring_128, ring_96, ring_48`。同时兼容参考枚举名 `ACC_*/ATMOS_51/ATMOS_71/ATMOS_DB/TRY/ACCOM/MULTI/BAYIN/SHOUDIE/GUITAR/RING_*` 及对应加密名称，名称不区分大小写。

顶层 `default_spec/file_type` 缺省为 `mp3_128`，并像参考方法一样决定整批使用普通 `GetVkey` 还是加密 `GetEVkey`；逐项 `spec/file_type` 只覆盖该项文件前缀。`media_id/media_mid` 存在时文件名使用一次媒体 MID，省略时保留参考实现的两次歌曲 MID 拼接行为。`account` 省略时使用匿名 UIN；提供时必须命中 QQ 平台下的持久账户别名，UIN、Key 与 LoginType 只由 provider 注入，调用方不能在请求中提交 Cookie 或密钥。匿名 sid 被 `1000/104400/104401` 拒绝时仅清除并刷新一次；账户凭据失效不会被匿名重试掩盖。

QQ 统一歌曲流建立在同一精确授权上，但不会把需要 EKey 的加密文件直接冒充普通播放器可播 URL，也不会把彩铃混入歌曲 `auto`。`auto` 依次尝试常用 OGG/MP3 320k、192k、128k、96k 和 48k；`lossless/hires/surround/spatial/dolby/master` 只在明确请求时选择对应规格。`bitrate` 只接受平台真实存在的 `48000/96000/128000/192000/320000/640000` bit/s，未知值返回 `invalid_request` 而不猜测；`spatial` 的 `c51/ste` 分别选择 5.1 和立体声规格，QQ 不存在的 AAC 沉浸分支会明确拒绝。完整文件不可用时，播放可返回带精确 `TrialWindow` 的试听；下载排除试听，避免把片段标为完整文件。授权 PURL 与 CDN 根地址在服务端做同源安全拼接，首选 HTTPS，剩余根地址按上游顺序和重复项保存在 `backup_urls`，有效期取文件授权和 CDN 目录的较短值。已知文件、版本 MID、`song_type`、`size_new` 和试听信息先解析为内部强类型元数据；冲突、畸形容器和类型错误不会退化成裸 JSON 猜测。

音频流的统一音质为 `auto/low/standard/higher/high/lossless/hires/surround/spatial/dolby/master`。网易云兼容字段 `level` 是 `quality` 的别名，并完整接受 `standard/higher/exhigh/lossless/hires/jyeffect/sky/dolby/jymaster`，其中 `exhigh/jyeffect/sky/jymaster` 分别映射为 `high/surround/spatial/master`。`variant=default|legacy|modern` 选择 provider 推荐后端、旧版码率后端或新版等级后端；兼容字段 `backend` 接受 `v0/song_url` 与 `v1/song_url_v1` 等别名。网易云缺省使用现代 v1；`variant=legacy` 时 `bitrate`（兼容 `br`）按原始无符号 bit/s 精确提交，省略时再由 `quality` 映射默认码率；现代后端按参考行为忽略 `br`。

`immersive_type=c51|ste|aac` 选择沉浸声音频类型，并兼容网易云字段名 `immerse_type`/`immerseType`。省略时网易云 `spatial/sky` 使用上游默认 `c51`；显式选择仅在现代 `song_url_v1` 且音质为 `spatial/sky` 时写入 `immerseType`，其他音质和旧版协议不会误发该字段。该控制与音质、账户和跨平台路由一同贯穿单曲、批量、播客及 Uni Playlist 项播放；不支持的值返回 `invalid_request`，不会静默降级为另一种沉浸声类型。

`available_qualities` 始终按上述能力层级从低到高返回，不依赖平台响应数组的偶然顺序。网易云歌曲元数据中的 192 kbps `m` 档映射为 `higher`，320 kbps `h` 档才映射为 `high`；QQ 的 192k OGG/AAC 同样映射 `higher`，`size_new[2]/[6]` 是 `spatial` 而不是较低级的 `surround`，DTS/`size_new[9]` 才是环绕；零大小不会遮住其他真实可用规格。当逐字 YRC 与逐行 LRC 同时存在时，`Lyrics.format` 标记能力更高的 `yrc`，但 `plain` 与 `word_synced` 两份内容都会保留；歌词贡献者的无效旧 ID 也不会遮住有效 `userId`。QQ 的 `qrc/trans/roma` 是相互独立的上游开关，返回内容使用平台自定义 3DES+zlib 编码；TuneWeave 解密后按实际 `qrc` 响应标志选择 `qrc` 或 `lrc`，逐字歌词存在时不会被逐行格式覆盖。歌曲 ID/MID、`song_type` 和完整上游响应仍保存在扩展。

批量 GET 的 `refs` 是逗号分隔完整资源引用；`ids/id` 是平台内 ID，`platform` 省略时使用服务默认平台，且只能与 `ids` 一起使用。POST 的 `refs/ids` 既可为单个字符串或逗号字符串，也可为字符串数组。两种输入都不折叠重复项；混合平台引用按来源 provider 分组，声明原生批量能力的平台使用单次批请求，其余 provider 使用同一严格逐项契约，再还原原顺序。`StreamBatch.outcomes` 为每个输入返回独立 `status/stream/error_code/error/extensions`，单项不可用不会把整个 HTTP 请求变成失败；provider 批次诊断位于 `extensions.provider_batches`。QQ 当前统一单曲播放已完成，但尚未声明原生 `audio_stream_batch`，避免把逐项实现误报成单批优化。

`unblock=true` 是参考 `/song/url/v1?unblock=true` 与 `/song/url/match` 的统一兼容预设，不另建第二套解灰逻辑。指定 `source=qq|kugou|kuwo|migu|...` 时先在该平台严格匹配，再回到原平台；省略时依次尝试 QQ、酷狗、酷我、咪咕和原平台。该模式始终保留原平台兜底，所以兼容输入中的 `fallback=false` 不会关闭兜底；为避免两套路由规则冲突，不能同时提交 `playback_platform` 或 `fallback_platforms`。`account` 绑定首个目标来源，所有尝试及失败原因都返回在 `attempts`。

Uni Playlist 项流复用同一音质与路由参数，并额外以 `accounts` 为每个平台选择独立账户。查询值可写成 `netease=main,qq=green-diamond`，也可传 URL 编码后的 JSON 对象；同一平台不能同时由兼容 `account` 和 `accounts` 指定。歌曲直接进入严格解析器；播客先取得节目承载音频；MV/视频按同一平台顺序选择原生视频流或严格匹配到其他平台的音频；广播刷新原平台直播地址或动态队列，不会用标题把直播频道错配成歌曲。`resolution=1..4320` 只适用于 MV/视频，请求档位和上游实际档位分开保留。响应中的 `item_id/source_ref/kind` 始终指向歌单项目，`stream.resolved_track/resolved_platform/attempts` 表达本次实际媒体来源。

节目流复用上述同一套解析器，不单独维护低能力播放分支。provider 先把节目 ID 解析为原音频 `Track`，随后才应用 `playback_platform/fallback/unblock/source/account`；因此网易云节目可以在原音频权益不足时严格匹配到 QQ 等平台，但节目引用本身不会被替换成歌曲引用。网易云 项目范围已真实验证公开节目 JSON 取流和 302；跨平台成功命中仍随 QQ 项目范围接入后补验。

下载端点复用同一套音质、后端、精确码率和账户参数，但不会把播放 URL 冒充不存在的专用下载能力。`MediaDownload.available` 与可空 `url` 明确表达下载能力，平台返回的实际音质、码率、大小、时长、业务码、费用和完整原文均保留；空白编码不会遮住有效容器格式，零响应时长不会遮住歌曲元数据时长。网易云新版下载即使顶层 `code=200`，单档 `code=-110/url=null` 仍返回 `available=false`；其 `/download/redirect` 先取专用下载地址，缺失时才以同音质播放 URL 兜底。QQ 的参考协议本身以同一文件授权取得可下载媒体，因此统一下载直接使用所选完整文件的授权 URL，但排除 `trial/trial_ogg_640`；真实文件不存在或无权限时仍返回可检查的 `available=false`，不会把试听片段标成下载成功。只有取得非空安全 URL 才发出 302 `Location`，客户端账户凭据和上游 Cookie不会进入重定向响应。

网易云的 `/v1/videos/{ref}`、`/stats` 和 `/stream` 分别精确覆盖 `mv_detail/video_detail`、`mv_detail_info/video_detail_info` 与 `mv_url/video_url`。MV 详情、统计和平台公布的 240/480/720/1080 四档播放地址已经完成真实 HTTP 验收；站内视频 ID 是不透明字符串，失效资源的 404 以及 `code=200` 但空 URL 列表都保持原始业务语义。又以账户收藏中的当前有效普通视频真实验证详情、4 档资源、统计、480p 非空播放 URL 与统一 302 重定向。

QQ MV 播放固定调用 `MvUrlProxy/GetMvUrls`，单项和 1–100 项批量共用同一强类型映射。QQ 的字母数字 VID 在省略 `kind` 时默认解释为 `mv`；显式 `video` 会被拒绝，避免把尚未支持的普通视频伪装成 MV。平台返回的 MP4/HLS 各档、直连/通用/免流/m3u8 地址、文件类型、编码、大小、令牌、有效期和业务码全部保留；选择时先取不高于请求值的最高已知清晰度，同档优先直接 MP4，只有没有低档时才向上选择。常规 `url` 为空不会遮住可用的 `comm_url/freeflow_url/m3u8`；所有可输出或重定向的地址都必须属于固定 QQ/Tencent Music 媒体域且不得嵌入用户凭据。重复 VID 在单次上游请求后按输入位置恢复，平台以 VID 为键的响应不会使重复项消失。

### 登录与账户

QQ、网易云与 B 站当前均声明 `caller_managed_credentials`：所有会创建登录态的首个请求接受 `credential_mode=server|client|both`，多步二维码/验证码事务固定首请求选择，确认阶段不能改写。`client/both` 的成功登录或刷新响应新增一次性 `caller_credential`，并强制 `Cache-Control: no-store` 与 `Pragma: no-cache`；普通状态、账户和业务响应仍不回显凭证。业务请求以可重复 `X-TuneWeave-Credential` 请求头按平台携带，和同平台显式 `account/accounts` 冲突时返回 400。QQ 与网易云当前已实现的公开业务端点、跨平台回退及安全原始扩展已经接通；B 站已覆盖二维码登录、会话状态、账户资料、刷新和退出，其他业务端点随 项目范围顺序逐项接通。这只表示凭证所有权桥覆盖已实现端点，不代表对应平台的扩展候选已经实施。封装格式、大小限制、跨平台组合、刷新/退出与脱敏要求以 [`docs/credential-ownership.md`](credential-ownership.md) 为准。

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| GET | `/v1/auth/country-codes` | `platform?`、`account?` | `CountryCallingCodeGroup[]`；登录可选国家/地区及电话区号目录 |
| POST | `/v1/auth/qr` | `{platform, account?, login_type?, credential_mode?}` | 二维码事务 ID、二维码 URL/图片、过期时间；事务固定凭证模式 |
| GET | `/v1/auth/qr/{transaction_id}` | 无 | `waiting/scanned/confirmed/expired/failed`；成功时按事务模式保存、返回或同时处理登录态 |
| POST | `/v1/auth/password` | `{platform, account?, principal_type, principal, password, credential_mode?}` | 登录状态、脱敏账户摘要及显式模式允许时的一次性调用方凭证 |
| POST | `/v1/auth/principals/status` | `{platform, account?, principal_type?, principal, country_code?}` | `AuthPrincipalStatus`；查询主体是否已注册，不创建登录态 |
| POST | `/v1/auth/challenges` | `{platform, account?, method?, principal, country_code?, credential_mode?}` | 短信等挑战事务；事务固定凭证模式 |
| POST | `/v1/auth/challenges/validate` | `{platform, account?, method?, principal, code, country_code?}` | `AuthChallengeValidation`；仅校验挑战码，不创建登录态 |
| POST | `/v1/auth/challenges/{transaction_id}/verify` | `{code}` | 验证状态；成功时按事务模式处理登录态；网易云兼容 `{captcha}` |
| POST | `/v1/auth/session/refresh` | `{platform, account?, credential_mode?}` 或调用方凭证请求头 | 刷新状态和脱敏账户摘要；调用方模式返回新凭证代际 |
| GET | `/v1/auth/session` | `platform + account?` 或一份调用方凭证请求头 | 当前会话状态，不返回凭据 |
| DELETE | `/v1/auth/session` | `platform + account?` 或一份调用方凭证请求头 | 服务器账户删除结果，或提示调用方丢弃已退出的凭证 |
| GET | `/v1/account` | `platform`、`account?` | 脱敏账户资料与权益摘要 |
| GET | `/v1/account/playlists` | `platform`、`account?`、分页 | `Playlist[]` |
| GET | `/v1/account/library/albums` | `platform`、`account?`、分页 | 已收藏的 `Album[]`；收藏时间保留在条目扩展，付费专辑计数等保留在分页扩展 |
| PUT | `/v1/account/library/albums` | JSON `{refs|ids, platform?, account?}`；`refs` 接完整专辑引用，`ids` 与 `platform` 配合，单项、数组和逗号列表均可 | 批量收藏 `SubscriptionResult[]`；顺序与重复项不丢失，同一批次只允许一个平台 |
| DELETE | `/v1/account/library/albums` | 与批量 PUT 相同的 JSON 契约 | 批量取消收藏 `SubscriptionResult[]`；保留顺序、重复项与平台失败 ID |
| GET | `/v1/account/library/radio-stations` | `platform`、`account?`、分页；`catalog=broadcast|styled`、`sources?` | 已收藏的 `RadioStation[]`；缺省为普通广播，`styled`/`difm` 返回 DiFM 风格频道收藏 |
| GET | `/v1/account/library/podcasts` | `platform`、`account?`、分页 | 已订阅的 `Podcast[]`；列表身份明确使 `subscribed=true`，完整平台条目与分页响应保留在扩展 |
| GET | `/v1/account/following/artists` | `platform`、`account?`、分页 | 已关注的 `Artist[]`；关注时间和平台原始资料保留在条目扩展 |
| GET | `/v1/account/following/artists/new-videos` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Video[]`；`before` 与 `next_before_ms` 均为毫秒时间戳 |
| GET | `/v1/account/following/artists/new-tracks` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Track[]`；`limit` 按作品块计数，块内歌曲完整展开，上游新曲总数保留为分页 `total` |
| GET | `/v1/account/following/artists/new-works` | `platform`、`account?`、`limit?`、`before?`、`source_type?`、`first_request?` | `ArtistWorkUpdate[]`；歌曲/MV 混合更新流，上游额外续页哨兵不计入本页，未知来源保留原文 |
| GET | `/v1/account/following/artists/new-tracks/play-all` | `platform`、`account?` | 最近至多 50 首新 `Track[]`；固定快照，不伪装成可翻页目录 |
| GET | `/v1/account/favorites/tracks` | `platform`、`account?`、分页 | `Track[]` |
| GET | `/v1/account/history` | `platform`、`account?`、`period=all_time|week`、分页 | `PlaybackHistoryEntry[]`，含 `track`、`play_count`、`score`、`last_played_at` |
| GET | `/v1/account/history/podcast-episodes` | `platform?`、`account?`、`limit?`、`offset?=0` | `PodcastEpisodePlaybackHistoryEntry[]`；完整分离节目、承载音频、播放时间与终端信息 |
| GET | `/v1/account/cloud/tracks` | `platform?`、`account?`、`limit?`、`offset?` | `CloudTrack[]` 分页及云盘容量统计 |
| GET / POST | `/v1/account/cloud/tracks/details` | 查询或 JSON `refs?|ids?`、`platform?`、`account?` | 保持输入顺序和重复项的 `CloudTrack[]` |
| GET | `/v1/account/cloud/tracks/{ref}/download` | `account?` | 云盘源文件的统一 `Stream`；不可用时返回明确业务错误 |
| GET | `/v1/account/cloud/tracks/{ref}/download/redirect` | `account?` | 302 到云盘源文件 URL；源文件 URL 缺失时回退到同平台同账户普通取流 |
| GET | `/v1/account/cloud/lyrics` | `platform?`、`account?`、`user_id`、`track_id` | 云盘文件标签中的统一 `Lyrics` |

QQ 专辑详情固定调用 `AlbumInfoServer/GetAlbumDetail`。纯十进制引用作为正数 `albumId`，其他合法字母数字引用作为 `albumMId`；服务端返回 MID 时统一 `Album.ref/id` 使用该规范身份，数字 ID 保留在扩展。`basicInfo`、发行公司与 `singer.singerList` 分别强类型解析，副标题、发行日期、描述、语种、类型、流派、百科地址、公司资料、全部署名歌手及未知字段不会因统一摘要而丢失；封面 URL 只从已校验 MID 拼接到固定 QQ 图片域。身份冲突、缺失 ID/MID、空名称或畸形已知字段返回上游错误，不会输出半成品专辑。

QQ 专辑歌曲固定调用 `AlbumSongList/GetAlbumSongList`，数字 ID 使用 `albumId`，MID 使用大小写精确的 `albumMid`；统一 `offset/limit` 直接映射到 `begin/num`，因此非整页对齐的偏移不会被静默取整。`songList[*].songInfo` 逐项进入完整统一曲目映射，外层曲序信息、未知字段和完整响应位于扩展；平台返回的 `albumMid/totalNum` 驱动规范专辑身份与真实 `total/next_offset/has_more`。响应专辑身份不一致、曲目指向其他专辑、超过请求页宽、总数越界或在总数耗尽前返回空页时会明确报上游错误。

QQ 新专辑目录固定调用 Android `newalbum.NewAlbumServer/get_new_album_info`，只接受该平台真实存在的 `catalog=new`。`area` 省略时为内地，也接受 `1|mainland_china|内地`、`2|hong_kong_taiwan|港台`、`3|western|欧美`、`4|korea|韩国`、`5|japan|日本`、`6|other|其他` 及文档化短别名；QQ 没有全部地区和独立 `newest` 分支，相关输入会明确拒绝。统一任意 `offset/limit` 精确映射为 `start/num`，总数和返回数驱动真实续页。每张专辑以规范 MID 为稳定身份，数字 ID、三类别名、全部歌手、发行日期、平台类型/地区/流派/语种、公司、封面、曲数/可播放曲数/长音频数、推荐理由及未知字段都被保留；provider 和 统一 HTTP 已验证 offset 0/1 连续窗口与全部六区非空目录。

QQ 歌手快照目录使用独立 `GET /v1/artists/catalog`，固定调用 Web `music.musichallSinger.SingerList/GetSingerList` 并提交 `hastag=0`。`type` 支持 `all|male|female|group`；`area` 支持 `all|chinese|hong_kong_taiwan|western|japanese|korean`；`genre` 支持 `all|pop|rap|chinese_style|rock|electronic|folk|r_and_b|ethnic|light_music|jazz|classical|country|blues` 及文档化别名。结果以 `ArtistCatalog.featured_artists/artists/filters` 分开保存热门歌手、完整平台快照和可选地区/性别/流派/首字母标签；Q029 的索引分页仍使用 `/v1/artists`，不会把一次性快照伪装成真实续页。歌手 MID、数字 ID、别名、拼音、国家/地区、趋势、关注数、图片及未知字段完整保留；命名账户只验证精确别名，不向公开 Web 目录注入密钥。provider 和 统一 HTTP 已真实验证默认及华语女流行筛选非空。

QQ 歌手索引分页使用 `GET /v1/artists?platform=qq` 和同一组 `type/area/genre` 筛选；`initial` 支持 `all`、`A-Z`、`#`，并兼容参考协议的 `-100/1..27`。上游 `GetSingerListIndex` 不接受调用方页宽，而是按 `sin` 返回最多 80 项；TuneWeave 精确保留任意 `offset`，按需组合一至两个连续物理窗口，再裁成 `limit=1..100` 的逻辑页。`meta.pagination` 使用平台真实总数，扩展保留索引/筛选 ID、固定窗口大小、请求窗口数、物理返回数、热门歌手、筛选标签及完整响应。固定窗口缺项、跨窗口总数变化或筛选回显冲突会明确失败，不会跳过歌手后继续分页。provider 真实验证 offset 0/1 连续、100 项双窗口和 A 首字母筛选。

QQ 歌手详情固定调用 Android `UnifiedHomepageSrv/GetHomepageHeader` 并以歌手 MID 定位。`Info.Singer` 和 `Info.BaseInfo` 提升为统一身份、名称、别名、头像与背景；歌手数字 ID、类型、关注态、粉丝/关注/好友/访客计数，以及完整 `Info/TabDetail/Prompt` 保留在扩展。当前平台把 `SingerHeaderPic` 返回为包含裁剪坐标、高分辨率图、3D 图和图片 MID 的对象，TuneWeave 同时兼容参考项目声明的旧字符串形态；图片资源 MID 独立允许平台实际使用的下划线版本后缀，但歌手资源 MID 仍保持严格字母数字校验。主页默认 Tab 返回的 `null` 列表规范化为空列表，不据此伪造作品总数；所有对外图片 URL 必须是无内嵌凭据的 HTTP(S) 地址。

QQ 歌手歌曲固定调用 Android `musichall.song_list_server/GetSingerSongList`，使用 `singerMid/order=1/begin/number`；参考能力没有时间排序，`order=time` 不会静默降级。当前上游会忽略较小的 `number` 并固定返回最多 30 条，但 `begin` 仍精确应用，因此 TuneWeave 把物理响应安全裁成调用方请求的逻辑窗口，用逻辑条目数推进 `next_offset`，同时在分页扩展公开 `upstream_returned/limit_applied` 并保留完整物理响应。这样任意非整页 `offset` 可连续遍历且不会因上游过取而越过资源；`singerMid/totalNum/songList[*].songInfo`、外层曲序和未知字段均经强类型包装保留。

QQ 歌手专辑固定调用 Android `music.musichallAlbum.AlbumListServer/GetAlbumList`，使用 `singerMid/order=1/number/begin`。稳定专辑 MID 优先于数字 ID，译名、歌手、封面、发行日期、曲数、类型、标签和完整原项均映射或保留；`albumList=null` 作为合法空页处理，歌手身份、总数或条目畸形则明确失败。当前上游同样会把较小的 `number` 固定过取为 30 条，统一接口按请求的逻辑窗口裁切并在分页扩展公开 `upstream_returned/limit_applied`；统一 HTTP 验证确认任意 offset 连续且不会跳过专辑。

QQ 歌手 MV 固定调用 Android `MvService.MvInfoProServer/GetSingerMvList`，使用 `singermid/order=1/count/start`。默认 `type=mv`，显式 `type=all` 在 QQ 当前只提供歌手 MV 的能力边界内使用同一目录；排序只接受 hot，不支持游标。VID 优先于数字 MV ID，标题、封面、时长、播放量、发布时间戳和完整原项均映射或保留；该响应不包含歌手名，TuneWeave 不伪造空名 creator，而是在条目及分页扩展保留 `singer_mid`。分页保留过取裁切防线及 `upstream_returned/limit_applied`；统一 HTTP 验证中上游 `count=2` 精确返回 2 条，总数 10426，offset 0/1/2 连续。

QQ MV 详情固定调用 Android `video.VideoDataServer/get_video_info_batch`。请求完整列出 VID、SID、封面、时长、歌手、开关、消息、名称、描述、播放量、发布时间、收藏态、GMID、上传者资料及关联歌曲等 21 个唯一字段；参考实现中重复的 `uploader_hasfollow` 选择项被去重。单项和 1–100 项批量共用原生批量请求，返回映射按输入重建，因此重复 VID 和调用顺序不会被上游字典覆盖；缺失、额外、错位或畸形 VID 明确失败。上传者字段允许平台真实存在的空字符串/null，并保持其他非标量为错误；每项保留完整自身详情及去除整批 data 后的顶层响应元数据，避免批量响应在每项重复造成二次方膨胀。详情接口不伪造播放清晰度，实际档位继续由流端点返回。已真实验证普通短视频 MV 与歌手 MV，以及异构三项、首尾重复的统一批次。

`principal_type` 至少允许平台实际支持的 `email`、`phone` 或平台账号类型；密码默认按明文接收并立即在适配器内完成平台要求的摘要，也可用 `password_format: "md5"` 明确提交已有摘要。`method` 至少允许 `sms`，并可由平台扩展。上游存在多种登录方式时必须全部接入，不能只保留二维码这一条流程。

网易云播客订阅列表固定使用 WeAPI `/api/djradio/get/subed`，提交 `limit/offset/total=true`，并将 `count/hasMore`（兼容 `more`）映射为统一分页。列表本身比条目内可能陈旧的 `subed=false` 更明确，因此返回项稳定标记 `subscribed=true`，不会让低层默认值遮住账户资料库语义。订阅与取消订阅分别使用 `/api/djradio/sub` 和 `/api/djradio/unsub`，统一为同一资源路径的 PUT/DELETE，并由 `account` 选择隔离的持久登录态。

网易云最近声音固定使用 WeAPI `/api/play-record/voice/list` 并提交 `limit`（默认 100、范围 1–100）。平台包装中的 `data.pubDJProgramData` 映射为完整节目，节目引用、所属播客与 `mainTrackId/mainSong` 对应的承载音频引用不会混淆；`playTime` 转换为 RFC 3339 的 `played_at`，`os/multiTerminalInfo` 映射为独立终端对象，完整记录和响应仍保存在扩展。上游没有 offset 或续页控制，所以只接受 `offset=0`，即使 `total` 大于本次条目数也保持 `next_offset=null/has_more=false/continuation_supported=false`。使用隔离持久账户通过真实统一 HTTP 返回两条记录，节目 `netease:2059302984` 与音频 `netease:1342589772` 保持分离，上游 `code=200`。

网易云 DiFM 频道收藏复用 `/v1/account/library/radio-stations`：`catalog=styled`（别名 `style/difm`）选择风格频道目录，`sources` 默认 `0`，也接受 `[0,1,2]`、`0,1,2` 或单值，仅允许电子/古典/爵士 `0/1/2`。上游返回的是固定收藏快照，没有分页控制，因此只接受 `offset=0`，`limit` 仅保留调用意图并以 `limit_applied=false/continuation_supported=false` 明示平台没有应用；频道引用保持 `netease:difm:{source}:{channelId}`。同一引用可用于账户资料库的 PUT/DELETE 订阅端点，`account` 始终选择隔离登录态。持久化真实账户通过统一 HTTP 请求三源目录，上游返回 `code=200` 和空收藏快照；写入路径未用于改变该账户现有收藏。

国家区号目录允许省略 `platform` 并使用服务默认平台；`account` 只选择该平台的请求会话。网易云固定以 EAPI 调用 `/api/lbs/countries/v1`，公开目录不要求登录；统一结果保留上游分组顺序、电话区号、地区代码和中英文名称。不存在的非默认账户别名仍按账户隔离规则返回认证错误，不会静默退回默认会话。

`/v1/auth/principals/status` 只查询注册状态，不发送验证码、不登录。`principal_type` 省略时默认 `phone`；网易云兼容参考字段 `phone/countrycode`，分别作为 `principal/country_code` 的别名，也接受 `countryCode`，手机号和区号均可为字符串或数字，区号缺省或为空时使用 `86`。统一结果用 `exists` 表示是否注册，并保留 `has_password`、平台已脱敏的 `display_name`、`avatar_url` 和 `platform_code`；完整上游响应位于 `extensions.response`，原始手机号不进入稳定字段或日志。

`/v1/auth/challenges/validate` 与事务验证端点语义不同：它只调用平台的验证码校验能力，不登录、不保存 Cookie，也不要求先发送验证码。`method` 省略时默认为 `sms`；网易云还兼容参考字段 `phone/captcha/ctcode`，分别作为 `principal/code/country_code` 的别名，手机号和区号都接受字符串或数字，区号缺省或为空时使用 `86`。`valid=false` 是正常业务结果，仍以 HTTP 200 返回，并通过 `platform_code`、`message` 和 `extensions.response` 保留平台信息；空白上游 `message` 不会遮蔽有效 `msg`。手机号和验证码不会回显。需要验证码登录时仍使用 `/v1/auth/challenges` 创建不透明事务，再调用 `/{transaction_id}/verify`。

验证码登录事务同样允许省略 `method`（默认 `sms`），并兼容 `phone/ctcode` 与后续验证请求中的 `captcha`；这些标量既可为字符串也可为数字。发送端点只发送一次，不会自动重试；事务验证成功后才保存对应 `platform/account` 的登录态。

QQ 手机验证码使用 `music.login.LoginServer` 的 Android 链路。`principal` 默认必须是 5–32 位数字并作为 `phoneNo` 原样提交；确实持有平台加密手机号时可显式使用 `encrypted:<opaque>`，仅去掉此前缀后提交 `encryptedPhoneNo`，不会因 JSON 字符串类型而把普通手机号误判为密文。发送遇到 `20276` 时错误详情保留平台 `security_url`，但不会创建一个实际未发送验证码的事务；频率限制也不会自动重试。验证端提交 `loginMode=1`、`tmeLoginMethod=3/tmeLoginType=0`，成功后按 `(qq, account)` 原子保存凭据。

QQ 会话状态固定调用 Android `music.UserInfo.userInfoServer/GetLoginUserInfo`，服务端凭据失效码返回正常的 `authenticated=false` 账户资料。不存在的 QQ 账户别名同样返回未认证资料，不会把缺失别名静默替换为 `default`。凭据自带的创建时间和有效秒数只有同时存在时才计算本地到期扩展，服务端检查结果始终具有更高优先级。

B 站会话状态固定调用 Web `x/web-interface/nav`，只把强类型凭证生成的站点 Cookie 放入请求；二维码刷新令牌不会进入普通业务 Cookie。平台 `-101` 或 `isLogin=false` 返回正常的 `authenticated=false`，登录成功时 UID 必须与选中凭证一致，并映射昵称、受限头像 URL、邮箱/手机验证、等级、认证、挂件、大会员、钱包和 WBI 口令等已知账户字段。不存在的精确账户别名不访问上游且不回退 `default`，调用方托管凭证使用同一映射链路。

B 站刷新先以当前 Cookie 查询 `x/passport-login/web/cookie/info`。平台声明无需刷新时仍验证当前会话，并在 `client/both` 模式返回同一凭证代际；需要刷新时使用平台时间戳和官方固定公钥生成 RSA-OAEP SHA-256 `correspondPath`，从固定 `www.bilibili.com/correspond/1/...` 页面提取严格校验的实时 CSRF，再依次提交 Cookie 刷新和旧 refresh token 确认。新响应必须同时提供完整 Cookie、新 refresh token 和相同 UID，确认后还要通过 `nav`，全部成功后才能按归属模式原子发布。退出固定调用 `passport.bilibili.com/login/exit/v2`；上游确认成功或明确表明凭据已失效时才删除服务器精确别名并提示调用方丢弃自管凭证，网络、CSRF 或未知响应失败时保留旧代际。

QQ 会话刷新调用 Android `music.login.LoginServer/Login` 并固定 `loginMode=2`。`loginType=1`、`loginType=2` 和其他登录类型分别保留微信、QQ 及移动端/验证码凭据所需的不同字段集合，`comm.tmeLoginType` 与原凭据一致；旧凭据同时进入 Android `comm` 和 Cookie。只有平台返回成功且新凭据完整通过强类型校验后，才原子替换同一 `(qq, account)` 的凭据代际；网络、业务码、响应解析或写盘失败都不会预先删除旧凭据。

QQ 退出调用同一 Android 登录服务的 `Logout`，并把精确账户凭据同时放入 `comm` 与 Cookie。平台成功或明确返回凭据已经失效时删除本地对应 `(qq, account)`；不存在的别名幂等返回 `removed=false`。限流、未知业务码和网络失败保留本地凭据以便重试，不会影响同平台其他账户；若上游已关闭会话但本地删除失败，则返回明确的本地持久化错误，而不是伪报退出完成。

`GET /v1/account/playlists?platform=qq&account=...` 把当前账户创建的歌单和收藏的外部歌单合并为一个连续分页：创建歌单固定在前，收藏歌单随后，跨边界请求不会重复或漏项。创建目录使用账户 music ID，收藏目录使用同一凭据的加密 UIN，不能用占位凭据或其他账户替代；创建与收藏总数、完成/隐藏/更多标记、删除 ID、失败 ID 及两份完整上游响应都保存在分页扩展。普通歌单 ID 为统一 `qq:<id>`；平台以 `id=0, dirid>0` 表示的“我喜欢”等特殊目录使用 `qq:dir:<dirid>`，以便后续详情、完整歌曲分页和 Uni 导入保持稳定身份。普通账户的当前目录与同一数值 UIN 创建目录已真实返回 18 项，平台身份和顺序一致，当前账户路径全部保留可写 `dirId` 引用。

QQ 的 `GET /v1/playlists/{ref}` 与 `/tracks` 固定调用 Android `music.srfDissInfo.DissInfo/CgiGetDiss`。公开 `qq:<playlist-id>` 使用 `disstid`，账户特殊目录 `qq:dir:<dirid>` 使用 `disstid=0/dirid/enc_host_uin` 并要求精确 `account`；详情请求保留标签和创建者，歌曲分页使用 `onlysonglist=true` 并关闭不需要的标签/用户包装。`song_begin/song_num` 精确对应统一 offset/limit，当前页数量、总数和 `hasmore` 必须相互一致；歌曲完整复用 QQ 强类型 Track 映射。公开歌单详情与分页已真实验证；普通账户的 `qq:dir:201` 详情和首个 100 首分页又真实返回总数 158，并与喜欢歌曲目录身份、顺序一致。

QQ 喜欢歌曲使用同一 `CgiGetDiss` 的 `disstid=0/dirid=201` 分支。`GET /v1/account/favorites/tracks?platform=qq&account=...` 从所选凭据读取 `encryptUin`；`GET /v1/users/qq:<encrypted-uin>/favorites/tracks` 直接使用目标用户的加密 UIN，并允许可选 `account` 作为查看者会话。该分支固定发送 `tag=true/userinfo=true/orderlist=true`，不发送只属于普通歌单精简取曲的 `onlysonglist`；两端共享严格 offset/limit、分页一致性、零进度拒绝和 Track 映射。加密 UIN 作为不透明平台用户 ID 处理，不接受空白、控制字符或超长值，也不会以参考项目的占位凭据代替缺失账户。普通账户的两条路径均以两页完整返回 158 首歌曲，身份、顺序、总数和终止状态完全一致。

QQ 的公开用户歌单目录保持两种身份边界：`GET /v1/users/qq:<numeric-uin>/playlists/created` 调用 `PlaylistBaseRead/GetPlaylistByUin`，只接受正整数 UIN，并在完整取得上游创建目录后应用统一 offset/limit；`GET /v1/users/qq:<encrypted-uin>/favorites/playlists` 调用 `PlaylistFavRead/CgiGetPlaylistFavInfo`，把目标加密 UIN、offset 和 size 原样提交。两端的 `account` 都只是可选查看者会话，省略时不会被改写为 `default` 或强制要求本地账户。条目明确标记 created/favorite 与 subscribed 状态，删除/失败 ID、隐藏/完成标记、总数和完整响应均保留。已从公开歌单 `qq:7039749142` 动态取得其创建者两类标识，并通过 provider 与统一 HTTP 真实验证两个匿名目录分支：创建目录总数 6137，收藏目录合法返回空目录；普通登录账户还在可逆写入闭环中读到收藏目录由 0 增至 1，再于取消收藏后恢复为 0。

QQ 收藏专辑统一为当前账户与指定用户两个视图：`GET /v1/account/library/albums?platform=qq&account=...` 从精确 `(qq, account)` 凭据读取加密 UIN；`GET /v1/users/qq:<encrypted-uin>/favorites/albums` 直接使用目标用户标识，并允许独立的可选查看者 `account`。两端都固定调用 Android `music.musicasset.AlbumFavRead/CgiGetAlbumFavInfo`，把 `offset/limit` 原样提交为 `offset/size`，不会先换算整页。专辑优先以 MID 建立稳定引用并保留数字 ID、名称/标题/译名、曲数、发行/收藏时间戳、状态、位置、完整歌手和未知字段；平台 `hasmore/total` 同时驱动真实续页，矛盾标志、零进度、超页、畸形身份或必需字段会明确失败。上游 `logo` 字段既可能是传统图片 MID，也已真实返回完整 QQ 图床 URL：前者安全拼接 CDN，后者按无凭据 HTTP(S) URL 校验并将 `y.gtimg.cn` 的 HTTP 升级为 HTTPS，不会把 URL 再拼进 MID 模板。普通账户已在可逆收藏期间验证当前与指定用户非空视图一致，取消后两端恢复为空。

QQ 专辑收藏写入通过普通 Android `music.musicasset.AlbumFavWrite` 完成：收藏调用 `FavAlbum`，取消收藏调用 `CancelFavAlbum`。单项 `PUT|DELETE /v1/account/library/albums/qq:<numeric-id>` 与批量 `PUT|DELETE /v1/account/library/albums` 使用同一批量实现；批量体可提交完整 `refs`，也可提交 `platform=qq` 与 `ids`，保留原始顺序和重复项并一次发送 `v_albumId`。专辑 ID 必须是正整数，输入在账户读取和网络访问前完成校验；凭据严格取自所选 `(qq, account)`。只有包络码与强类型 `result` 同时为零且 `v_failedAlbumId` 为空才返回成功，部分失败、未知失败 ID、缺字段或畸形响应不会被误报为已改变。普通账户已完成单张收藏、非空读回、取消和空目录恢复的真实闭环，最终没有测试对象残留。

QQ 收藏 MV 同样保留当前账户和指定用户两种视图：`GET /v1/account/library/videos?platform=qq&account=...` 使用所选账户自身的加密 UIN；`GET /v1/users/qq:<encrypted-uin>/favorites/videos?account=...` 使用路径中的目标用户，但因上游明确要求登录，查看者 `account` 必须显式给出。两端固定调用 Android `music.musicasset.MVFavRead/getMyFavMV_v2`，精确提交 `encuin/pagesize/num`，其中 `num` 是零基页。统一任意 `offset/limit` 会用相同物理页宽读取至多两个连续页并裁出逻辑窗口，不要求调用方按页对齐。响应没有可靠 `total/hasmore`，因此 `total=null`，只在已读缓冲区仍有项目或末个真实页恰好满页时推断 `has_more`；完整页数据、计数和原响应保留在分页扩展。每项优先以 VID 建立稳定引用，正数 MV ID 仅作回退；标题、名称、安全封面、播放量、发布时间、歌手、状态和未知字段全部保留并标记已收藏。参考模型允许用 `singerId` 填补 MV ID，可能把歌手身份误当视频身份，TuneWeave 明确拒绝该歧义。普通账户的当前账户与同一加密 UIN 用户视图已真实返回相同合法空目录，单页且没有伪造总数。

QQ 关注歌手也同时提供当前账户与指定用户视图：`GET /v1/account/following/artists?platform=qq&account=...` 从所选凭据取得目标加密 UIN，`GET /v1/users/qq:<encrypted-uin>/following/artists?account=...` 使用路径目标并强制显式登录查看者。两端固定调用 Android `music.concern.RelationList/GetFollowSingerList`，把统一 `offset/limit` 原样提交为 `HostUin/From/Size`，不会先换算参考接口的 `page`，上游 `LastPos` 只作为兼容元数据保留而不替代真实 offset。条目以歌手 MID 建立稳定 `Artist` 引用，同时保留关联歌手账户的加密 UIN、列表目标关注态、当前查看者关注态、粉丝数、描述、安全头像和完整原项；总数、更多标志、消息、隐私锁定和完整响应位于分页扩展。普通页会严格核对返回数、总数与续页，锁定目录允许显式空页，超出总数的 offset 也按空终页表达。普通账户已真实验证当前账户与同一加密 UIN 用户视图，均返回单页总数 0 的一致合法空目录。

二维码与验证码端点返回的 `transaction_id` 是 TuneWeave 生成的随机不透明标识，不是上游二维码 key、手机号或 token。敏感字段仅在请求生命周期或短期事务仓库内使用；保存后的平台凭据只通过账户别名引用，版本化 `caller_credential` 也只会在平台已支持且显式选择 `client/both` 的成功响应中出现。密码、验证码、原始 Cookie 与上游事务标识不会写入普通响应。

`POST /v1/auth/qr` 的 `image_data_url` 是可直接显示的自包含图片；网易云与 B 站返回 `data:image/svg+xml;base64,...`，二维码编码在进程内完成，不会把登录 URL 发送给第三方图片服务。B 站支持 `login_type=default/web/bilibili`，固定使用 BBDown 的 Web Passport 创建与轮询链，区分未扫码、已扫码待确认、过期、确认和平台失败；重复 `Set-Cookie` 优先于固定 `crossDomain` 回填，二维码 key 与 Cookie 只存在于服务端事务或选定凭证归属中。B 站已真实完成二维码创建、未扫码轮询、普通账户扫码确认、服务器凭据保存、刷新及服务重启恢复；同一账户随后真实读取资料、账户视频状态、字幕正文和普通权益播放清单。QQ 音乐支持 `login_type=qq/default`、`wx/wechat/weixin` 和 `mobile/app`，分别返回 QQ 互联 PNG、微信 JPEG 和 QQ 音乐客户端 PNG；这些平台二维码没有可安全复用的独立扫码文本，因此 `url` 与 `image_data_url` 均为同一自包含图片。移动端二维码在图片返回前已建立持久 MQTT 订阅，后续 GET 轮询可跨请求接收扫码、取消、过期、失败和确认事件，不会在两次请求间临时断开订阅。QQ 互联成功参数只接受固定 QQ 登录 HTTPS 域名的 `/check_sig` 地址，从中提取并校验必要参数后仍由 provider 构造固定签名端点，不会跟随回调提供的任意地址；二维码轮询只发送 `qrsig`，签名交换不继承轮询 Cookie。签名交换仅在固定 QQ Graph HTTPS 主机间手动跟随最多 10 次跳转，逐跳携带和汇总非空 Cookie，不允许后续同名域清理 Cookie 覆盖此前有效的 `p_skey`；OAuth 收到完整签名跳转链的 Cookie。平台最终仍未下发 `p_skey` 时明确失败，不用空哈希伪造授权。QQ 的 qrsig、微信 uuid、移动端二维码 ID、OAuth code、MQTT token 和临时 Cookie 只存在于 10 分钟进程内事务，HTTP 响应仍只暴露随机外层事务 ID；确认成功后由首请求固定的 `credential_mode` 决定按 `(qq, account)` 保存、只返回调用方或保存并返回同一凭证代际。二维码 key 和业务码按首个可解析的非空候选映射，空顶层兼容字段不会遮住 `data` 中的有效值。QQ 互联、微信、QQ 音乐客户端和手机验证码四种登录均已真实完成确认；三类服务器凭据已刷新并通过服务重启恢复，微信调用方凭据已完成不落盘会话检查。

文件账户后端默认位于 `.local/data/accounts`，可用 `TUNEWEAVE_DATA_DIR` 改变其父目录。账号别名在路径中使用 UTF-8 十六进制编码，不能构造路径穿越；每次更新先在同目录写入私有临时文件并同步，再以原子重命名发布新代际，启动只读取最新完整代际。Unix 权限为目录 `0700`、文件 `0600`，Windows 继承数据目录 ACL。文件内的平台会话凭据目前不做静态加密，因此运维必须保护该目录且不得同步或提交；除显式 `client/both` 登录或刷新产生的调用方凭证外，凭据从不进入 Debug、普通错误、HTTP 响应或日志。服务器托管的网易云 `DELETE /v1/auth/session` 保持既有兼容语义：即使上游退出请求不可达也清除本地凭据，并以错误详情 `local_session_removed` 明确结果；调用方模式不会删除调用方存储，只有上游确认退出后才返回 `caller_credential_discard_required=true`，`both` 同时删除身份匹配的精确服务器别名。

### Uni Playlist

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| GET | `/v1/uni/playlists` | `limit?`、`offset?` | 分页服务端 `UniPlaylist[]` 目录，不内联项目 |
| POST | `/v1/uni/playlists` | JSON `{name, description?}` | 新建的空 `UniPlaylist` |
| POST | `/v1/uni/playlists/imports` | JSON `{name?, description?, sources:[{ref?, platform?, type?, id?, account?}]}` | `UniPlaylistImportResult`，完整分页后原子创建的多来源合并歌单 |
| GET | `/v1/uni/playlists/{ref}` | 完整 `uni:<opaque-id>` 引用 | 持久化的 `UniPlaylist` 元数据 |
| PATCH | `/v1/uni/playlists/{ref}` | JSON `{name?, description?}`，至少一个字段 | 修改后的 `UniPlaylist` 元数据 |
| DELETE | `/v1/uni/playlists/{ref}` | 完整 `uni:<opaque-id>` 引用 | `UniPlaylistDeleteResult`，原子删除元数据与全部项目 |
| GET | `/v1/uni/playlists/{ref}/export` | 完整 `uni:<opaque-id>` 引用；可选 `Accept-Encoding: gzip` | 完整安全的 `UniPlaylistDocument` V1 副本 |
| POST | `/v1/uni/playlists/import-document` | JSON `{document, preserve_id?=false}` | `UniPlaylistDocumentImportResult`，完整验证后原子创建服务端副本 |
| POST | `/v1/uni/materialize/imports` | 查询 `limit?=200`、`offset?=0`；JSON `{name?, description?, sources:[{ref?, platform?, type?, id?, account?}]}` | 分页 `UniPlaylistMaterializeImportsResult`，完整展开来源但不持久化 |
| POST | `/v1/uni/materialize/items` | JSON `{items:[{ref,kind}], accounts?}` | `UniPlaylistMaterializeItemsResult`，不持久化的 V1 客户端项目 |
| POST | `/v1/uni/items/stream` | JSON `{item, quality?, variant?, bitrate?, immersive_type?, playback_platform?, fallback?, fallback_platforms?, unblock?, source?, account?, accounts?, resolution?}` | `UniPlaylistClientItemStream`，客户端托管单项的无状态播放结果 |
| GET | `/v1/uni/playlists/{ref}/items` | `limit?`、`offset?` | 分页 `UniPlaylistItem[]`，严格保留位置和重复来源项 |
| POST | `/v1/uni/playlists/{ref}/items` | JSON `{items:[{ref,kind}], accounts?}` | `UniPlaylistItemAddResult`，原子追加类型化混合项目 |
| DELETE | `/v1/uni/playlists/{ref}/items/{item_id}` | 稳定的单次出现项目 ID | `UniPlaylistItemDeleteResult`，仅删除该项目并重编号后续位置 |
| PATCH | `/v1/uni/playlists/{ref}/items/order` | JSON `{item_ids:[...]}` | `UniPlaylistItemOrderResult`，原子提交完整显式顺序 |

`UniPlaylist` 使用独立 `uni:` 命名空间，不归属于网易云等外部 provider。稳定字段包含同值的 `ref/platform/id` 身份、名称、描述、`item_count` 以及毫秒级 `created_at_ms/updated_at_ms`；新建歌单的项目数为 0。名称去除首尾空白后必须为 1–200 字节，描述去除首尾空白后最多 4000 字节，未知 JSON 或查询字段会被拒绝。`PATCH` 只修改明确提交的名称或描述，空字符串可以清除描述但不能清除名称；未提供任何字段会被拒绝，相同值是幂等成功且不会刷新更新时间或重写文件。身份、创建时间、项目数、项目顺序及项目快照不会随元数据修改而变化。`DELETE` 在一次发布中移除指定歌单及全部项目索引，返回删除前元数据和 `removed_item_count`，但不复制完整项目数组；不存在的资源不会被当作幂等成功，也不会影响其他歌单。目录默认 `limit=50/offset=0`，限制 1–100 项，按不可变 `created_at_ms` 降序并以 ID 升序打破同时间并列；响应包含真实 `total/next_offset/has_more`，不为目录项复制完整项目列表。单项 `GET/PATCH/DELETE` 必须提交完整引用，错误平台、畸形 ID 和不存在的歌单分别返回统一错误包络。

客户端托管和 Server/Client 显式复制共用固定格式 `tuneweave_uni_playlist_v1`。`UniPlaylistDocument` 顶层包含 `format/id/name/description/item_count/created_at_ms/updated_at_ms/items/extensions`；每个项目包含稳定 `id`、零基 `position`、`kind`、外部平台 `source_ref`、紧凑 `snapshot`、`added_at_ms` 和用途受限的导入来源扩展。文档必须保留同一来源的重复出现，项目 ID 唯一且位置连续，声明数量与数组一致，项目时间不得晚于文档更新时间；结构上限为 100,000 项，具体 HTTP 端点仍可采用更低的请求和响应限制。

V1 不提供任意扩展对象：顶层只声明 `duplicates_preserved`，项目只允许成组的 `import_source_index/import_source_ref/import_source_type` 和可选 `imported_from_item_id`，快照只允许规范引用、可播放性、音质档位、视频/播客/电台静态展示摘要。所有层级拒绝未知字段，因此不存在 Cookie、token、密码、验证码、设备身份、临时媒体 URL、签名、账户别名或任意请求头的合法槽位。快照 `cover_url` 只接受受限 HTTPS 展示地址，服务端不得根据它发起媒体或资源请求；真正播放仍按 `source_ref` 重新进入 provider、账户权益检查和统一回退链。

服务端导出在同一存储读快照中取得歌单元数据和完整项目序列，避免并发编辑产生数量或顺序撕裂；未知的存储扩展不会穿透，白名单字段形状错误则明确失败，不会把原始 JSON 退回客户端。不安全的旧封面地址会从交换快照中删除。响应使用 `Cache-Control: private, no-store` 和 `Vary: Accept-Encoding`；请求接受 `gzip` 且质量值大于零时返回 `Content-Encoding: gzip`，显式 `gzip;q=0` 优先于通配符并保持 identity。导出快照、JSON 序列化和压缩均在受控阻塞任务中完成，gzip 直接接收序列化输出，不额外保留完整未压缩响应缓冲。压缩与 identity 解码后具有相同安全文档，仍受 V1 的 100,000 项结构上限约束。导出是调用时刻的独立副本，不建立 Server 与 Client 自动同步关系。

文档导入请求体上限为 16 MiB，并在任何写入前完成格式、身份、数量、时间、连续顺序、唯一项目 ID、来源引用、快照及扩展白名单校验。默认生成与文档 ID 不同的新服务端 `uni:<id>`，但保留全部项目稳定 ID；`preserve_id=true` 才尝试把文档 ID 用作服务端 ID，已有资源会返回 `conflict`，绝不覆盖。验证、转换或持久化任一阶段失败都不会产生目标记录。成功结果明确返回源文档 ID、是否保留歌单 ID，以及 `atomic=true/item_ids_preserved=true/automatic_sync=false`；导入仅复制一次，不建立后续同步。

无状态来源展开与服务端导入共用来源语法、账户选择、provider 能力和完整上游分页状态机，可合并公开、账户可见、本地 Uni、B 站合集/收藏夹及 provider 后续扩展的可播放集合。服务端先完整读取所有来源并保持“来源顺序 → 来源内部顺序”和重复项，再以 `limit=1..500/offset=0..100000` 只返回客户端当前需要的一页；统一总量上限为 100,000 个可播放项目，单来源分页最多 2,000 页，非推进游标、缺失下一页游标和越界集合会明确失败。返回项目使用全局零基位置和由来源索引、来源内位置、集合引用及资源引用派生的确定性 ID，因此同一来源快照的分页重试可以安全拼接；上游集合变化后位置和 ID 可以相应变化，客户端需要重新 materialize。响应包含建议名称、描述、全部来源摘要、真实总数和标准分页元数据，但会清除来源结果中的服务器账户别名，并声明 `persisted=false/source_pagination_complete=true/response_paginated=true`。该端点不创建 `uni:<id>`、不发布文件，也不建立后续同步关系。

无状态项目标准化一次接受 1–100 个歌曲、MV、视频、播客节目或广播引用，与服务端项目追加共用同一份类型、来源平台和 `accounts` 校验，再逐项调用对应 Provider 获取真实元数据。返回项目按输入顺序从零编号，每次出现都分配独立稳定项目 ID，因此重复来源不会折叠；快照经过 V1 安全字段转换，不信任调用方提交标题、封面或播放能力。`accounts` 只在本次请求中选择各来源平台的账户，不进入客户端项目或响应扩展。成功与失败都不创建 `uni:<id>`、不读取或写入 Uni Playlist 存储；响应明确返回 `persisted=false/provider_validated=true`。

客户端托管项目播放只接受一个完整 V1 `item`，不会接收完整歌单、任意媒体 URL、请求头或 provider 原文。服务端先验证项目身份、外部来源、快照和用途受限扩展，再与 Server 模式项目共用同一条歌曲、MV/视频、播客和广播解析链；`playback_platform/fallback_platforms/fallback/unblock`、音质、码率、沉浸音频、视频清晰度、分平台账户别名和 `X-TuneWeave-Credential` 调用方凭据语义保持一致。`accounts` 在 JSON 中使用按平台键控的对象，`fallback_platforms` 使用逗号分隔的平台顺序；`resolution` 只允许 MV 或视频。响应不虚构 `playlist_ref`，而是返回项目 ID、来源、类型和统一 `MediaStream`，并声明 `client_hosted=true/persisted=false`。该端点不会读取或写入 Uni Playlist 存储；需要 GET/302 的客户端后续可使用独立的短期内存票据，而不能提交可篡改的目标 URL。

生产服务把每份歌单保存到 `TUNEWEAVE_DATA_DIR/uni-playlists/<playlist-id>.json`，与 `accounts` 凭据目录分离；`store.json` 只保存目录格式版本，内存目录索引在启动时由记录重建，单歌单写入不克隆或重写其他歌单。记录先写入并同步同目录私有临时文件后发布；Unix 使用原子替换，Windows 使用可在下次启动恢复的单记录备份切换。未知版本、文件名与身份不一致、畸形记录、未授权的目录内容都会阻止启动而不会被静默覆盖。首个正式 Release 前只保证全新部署使用当前目录格式，不提供开发快照之间的旧文件转换。记录只保存歌单结构与必要元数据快照，不保存媒体字节或平台凭据。

混合项目 `kind` 当前完整区分 `track`、`mv`、`video`、`podcast_episode` 和 `radio_station`。添加端点逐项按 `ref` 的来源平台调用已注册 provider 获取真实元数据，而不是信任调用方伪造标题：歌曲快照包含标题、艺人、专辑、时长、ISRC、封面、版本标签和播放能力摘要；MV/视频包含创作者、时长、封面、平台视频类型与发布时间；播客节目包含主播、时长、封面、所属播客、独立音频引用、发布时间和期号；广播电台包含名称、封面、分类、地区、当前节目及是否具有直接流。只保存这些播放匹配所需的紧凑字段，不复制整份上游原文或易过期的流地址。

`accounts` 是按平台键控的账户别名对象，例如 `{ "netease": "default", "qq": "green-diamond" }`；每个来源只使用自己的别名，不能提交 `uni` 账户或本批次未出现的平台。一次可追加 1–100 项，所有资源完成解析后才执行一次存储发布；失败不会留下半批数据。来源引用可以重复，存储不会静默去重：每次出现都会生成独立 `item_...` ID 和连续零基 `position`，后续删除/重排按项目 ID 工作。读取默认 `limit=50/offset=0`、范围 1–100，分页 `total/next_offset/has_more` 基于实际项目序列。歌曲重复项、MV 和播客节目混合写入时会保留独立项目 ID、真实快照和节目音频引用。

QQ Uni 来源支持两种稳定形态：`type=playlist` 导入一个已选择的公开、用户创建或用户收藏歌单，`id` 使用目录端点返回的普通歌单 ID；当前账户 `dir:*` 特殊目录可同时带 `account`。一次导入最多提交 100 个来源，因此可从 `/v1/users/{ref}/playlists/created` 与 `/favorites/playlists` 选择多个歌单并跨平台合并，不会把可能包含数千个歌单的非可播放目录无界展开。`type=favorite_tracks` 则把本身可播放的用户喜欢歌曲列表作为单一来源，使用 `{platform:"qq",type:"favorite_tracks",id:"<encrypted-uin>",account?}`，完整遍历 `CgiGetDiss` 的 offset/limit 后原子写入；来源摘要保留用户引用、类型、总数和可选查看者账户。统一 HTTP 使用公开非空来源完成 1/1 项导入，来源总数、写入结果、回读页和持久化文件计数一致。

删除端点只接受项目本身的稳定 ID，因此同一 `source_ref` 出现多次时仅移除指定的一次出现；返回被删除项目的原位置，剩余项目连续重编号。重排端点要求 `item_ids` 与当前项目集合完全一致且每个 ID 恰好出现一次，缺项、未知项、重复 ID 或畸形 ID 均在写入前拒绝，不能用部分顺序隐式移动项目。成功响应返回完整新序列和 `changed`；提交与现状相同的完整顺序返回 `changed=false`，不刷新文件或更新时间。两项操作都通过单次持久化发布完成，失败不留下部分删除或部分重排。

导入来源不是账户歌单的同义词。公开歌单无需 `account`，私有或账户可见集合才为该来源单独指定账户别名；同一次合并可以让不同平台甚至同平台的不同来源使用不同账户。每个来源必须二选一提交 `ref`，或提交 `platform+id`；`type` 可与两种写法同时使用并默认 `playlist`，规范化为最多 64 字节的蛇形 ASCII 标识。Provider 可按平台实现 `season`、`favorite_folder` 等类型，因此 B 站公开视频合集和个人收藏夹不会共用模糊的数字 ID 语义；不支持的类型返回 `capability_not_supported`。`uni:` 也可作为 `type=playlist` 的来源以再次合并，但本地来源不接受账户。

```json
{
  "name": "跨平台合并",
  "sources": [
    { "platform": "netease", "type": "playlist", "id": "3778678" },
    { "ref": "bilibili:3629748", "type": "season" },
    { "platform": "bilibili", "type": "favorite_folder", "id": "2883236382", "account": "default" }
  ]
}
```

一次导入接受 1–100 个来源，按请求中的来源顺序逐一完整读取所有分页，再按各来源内部位置生成新的稳定项目 ID；来源和条目都允许重复，不进行隐式去重。任何来源、分页或身份检查失败时目标歌单完全不创建；全部读取成功后，歌单级 `extensions.import_sources` 来源摘要、条目级来源索引/引用/类型和所有项目通过一次存储发布原子创建。未指定名称时使用来源名称按 `A + B` 派生并安全限制为 200 UTF-8 字节；单来源未指定描述时沿用其描述。公开来源的跨歌单完整合并、来源边界和重启恢复均已验证。

B 站 `season` 与 `favorite_folder` 来源已接通真实导入，来源 `id` 使用不带类型前缀的正整数，provider 内部再绑定到不可混淆的 Season 或收藏夹身份。导入项目保持 `kind=video`；需求样例 `season:3629748` 实际遍历 617 项，`favorite_folder:2883236382` 实际返回 98 项，两者一次 HTTP 请求按来源顺序原子合并为 715 项。收藏夹元数据报告 99 项但平台列表过滤了其中 1 项，TuneWeave 保留真实来源总数与实际写入数，不生成虚假视频补齐。

B 站 `video` 类型 Uni 项默认把视频视为可播放音频来源：未提交 `resolution` 时先用项目原始 B 站身份选择 BM02 音轨，`quality` 与 `accounts.bilibili` 指定的服务器账户或调用方托管凭证会进入同一播放请求。成功响应的统一 `MediaStream` 保留实际音质、码率、格式、编码、备用 URL、到期时间和必要 `Referer`，`extensions.transport=native_video_audio` 与完整强类型音轨结果用于区分来源；不会返回账户 Cookie。显式提交 `resolution=1..4320` 时改用原生视频轨，并标记 `transport=native_video`。Uni 项的 `/stream/redirect` 同样只是无缓存 302，无法代传 `MediaStream.headers`；需要 `Referer` 的客户端必须使用 JSON 端点自行发起媒体请求。

只有原始 B 站音轨失败且 `fallback=true` 时，解析器才按 `playback_platform/fallback_platforms` 顺序搜索其他音乐平台；匹配使用 Uni 快照中的标题、艺人、时长、ISRC 和版本标签，保持严格阈值，翻唱、现场或版本冲突不会因“能播放”而被接受。每个平台可用 `accounts` 提交独立账户，全部候选、分数、账户和失败分类按尝试顺序返回。默认音乐回退列表不包含 B 站，普通歌曲不会反向搜索 B 站视频；B 站视频是原始项目时仍自动先尝试自身音轨。公开视频 `BV1Jt411P77c` 已通过真实 Uni HTTP 取得原生音轨，账户隔离、原音轨失败后 QQ 严格命中及显式视频轨分支完成自动化验收。

### 写操作

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/playlists` | JSON `{platform?, account?, name, visibility?|privacy?, kind?|type?}` | `PlaylistMutationResult`，创建歌单 |
| PATCH | `/v1/playlists/{ref}` | JSON `{account?, name?, description?|desc?, tags?, variant?}` | `PlaylistMutationResult`，更新元数据 |
| DELETE | `/v1/playlists/{ref}` | 查询参数 `account?` | `PlaylistDeleteResult`，删除单个歌单 |
| DELETE | `/v1/playlists` | JSON `{refs?|ids?, platform?, account?}` | `PlaylistDeleteResult`，同平台批量删除 |
| POST / DELETE | `/v1/playlists/{ref}/tracks` | JSON `{refs?|ids?, account?}` | `PlaylistItemMutationResult`，增加/移除普通歌曲 |
| POST / DELETE | `/v1/playlists/{ref}/videos` | JSON `{refs?|ids?, account?}` | `PlaylistItemMutationResult`，增加/移除视频歌单项目 |
| POST / DELETE | `/v1/playlists/{ref}/items` | JSON `{refs?|ids?, kind?|type?, account?}` | `PlaylistItemMutationResult`，按显式类型增删项目 |
| PUT | `/v1/playlists/{ref}/tracks/order` | JSON `{refs?|ids?, account?}` | `PlaylistTrackOrderResult`，提交完整歌曲顺序 |
| PUT | `/v1/account/playlists/order` | JSON `{refs?|ids?, platform?, account?}` | `PlaylistOrderResult`，提交当前账户完整歌单顺序 |
| PUT | `/v1/playlists/{ref}/cover` | 查询参数 `account?`、`filename?`、`image_size?`、`crop_x?`、`crop_y?`；请求体为最大 20 MiB 的 `image/*` 字节 | `PlaylistCoverUpdateResult` |
| POST | `/v1/resources/{type}/{ref}/comments` | 查询参数 `account?`；JSON `{content}` | `CommentMutationResult`，创建评论 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/replies` | 查询参数 `account?`；JSON `{content}` | `CommentMutationResult`，回复指定评论 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}` | `account?` | `CommentMutationResult`，删除指定评论 |
| PUT | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`，启用评论反应 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`，停用评论反应 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/reports` | 查询参数 `account?`；JSON `{reason}` | `CommentReportResult`，提交评论举报 |
| PUT | `/v1/account/library/albums/{ref}` | `account?` | `SubscriptionResult`，收藏专辑 |
| DELETE | `/v1/account/library/albums/{ref}` | `account?` | `SubscriptionResult`，取消收藏专辑 |
| PUT | `/v1/account/library/radio-stations/{ref}` | `account?` | `SubscriptionResult`，收藏广播电台或 `netease:difm:{source}:{channelId}` 风格频道 |
| DELETE | `/v1/account/library/radio-stations/{ref}` | `account?` | `SubscriptionResult`，取消收藏广播电台或 DiFM 风格频道 |
| PUT | `/v1/account/library/podcasts/{ref}` | `account?` | `SubscriptionResult`，订阅播客 |
| DELETE | `/v1/account/library/podcasts/{ref}` | `account?` | `SubscriptionResult`，取消订阅播客 |
| PUT | `/v1/account/following/artists/{ref}` | `account?` | `SubscriptionResult`，关注歌手 |
| DELETE | `/v1/account/following/artists/{ref}` | `account?` | `SubscriptionResult`，取消关注歌手 |
| PUT | `/v1/account/avatar` | 查询参数 `platform?`、`account?`、`filename?`、`image_size?`、`crop_x?`、`crop_y?`；请求体为图片字节，`Content-Type: image/*`，最大 20 MiB | `ImageUploadResult` |
| POST | `/v1/account/cloud/uploads` | 查询参数 `platform?`、`account?`、`filename`、`bitrate?`、`song_name?`、`artist?`、`album?`；请求体为原始音频字节，最大 500 MiB | `CloudUploadResult`，由 TuneWeave 代理检查、上传、登记并发布 |
| POST | `/v1/account/cloud/uploads/ticket` | 查询参数 `platform?`、`account?`；JSON `{md5, file_size, filename, bitrate?, content_type?}` | `CloudUploadTicket`，含是否需要上传、临时曲目 ID、资源 ID 及受限对象存储请求信息 |
| POST | `/v1/account/cloud/uploads/complete` | 查询参数 `platform?`、`account?`；JSON `{provisional_track_id, resource_id, md5, filename, song_name?, artist?, album?, bitrate?}` | `CloudUploadResult`，登记并发布后的云盘曲目引用 |
| POST | `/v1/account/cloud/imports` | 查询参数 `platform?`、`account?`；JSON `{md5, source_track_id?, bitrate, file_size, file_type, song_name, artist?, album?}` | `CloudImportResult`，免上传导入结果及云盘曲目引用 |
| POST | `/v1/account/cloud/matches` | 查询参数 `platform?`、`account?`；JSON `{user_id, cloud_track_id, target_track_id?}` | `CloudMatchResult`；目标为 `0` 或省略时取消匹配 |
| DELETE | `/v1/account/cloud/tracks` | JSON `{refs?|ids?, platform?, account?}` | 删除选定平台账户中的云盘曲目 |
| PUT | `/v1/account/favorites/tracks/{ref}` | 完整歌曲引用、`account?` | 收藏结果 |
| DELETE | `/v1/account/favorites/tracks/{ref}` | 完整歌曲引用、`account?` | 取消收藏结果 |
| PUT | `/v1/account/favorites/playlists/{ref}` | 完整歌单引用、`account?` | 收藏结果 |
| DELETE | `/v1/account/favorites/playlists/{ref}` | 完整歌单引用、`account?` | 取消收藏结果 |

创建歌单时 `visibility=public|private` 与参考 `privacy=0|10` 等价，`kind=normal|video|shared` 与参考 `type=NORMAL|VIDEO|SHARED` 等价；同一语义的统一字段和参考字段不得同时提交。元数据更新的 `variant=default|batch|individual` 分别表示自动选择、参考批量模块和独立字段模块；批量分支必须同时包含名称、描述和标签。标签既可用字符串数组，也可用参考分号字符串，空数组或空字符串表示清除。

QQ 的创建协议目前只接受 `visibility=public` 和 `kind=normal`，其他统一变体会在联网前返回 `invalid_request`，不会伪装成 QQ 已支持。创建后的自建歌单以可继续读取、增删歌曲和删除的 `qq:dir:<dirId>` 作为主引用；服务端同时返回的公开 `tid` 以 `public_playlist_id/public_playlist_ref` 保存在扩展中，不能反过来把 `tid` 猜成目录 ID。当同名歌单已存在时，QQ 可能返回自动调整后的 `dirName`，因此 `playlist.name` 使用服务端实际名称，原始请求名保存在 `extensions.requested_name`。创建操作必须指定或使用默认 QQ 账户，且只读取精确 `(qq, account)` 凭据。

QQ 删除协议只接受目录 ID，因此单个和批量删除都必须使用创建结果或当前账户自建歌单给出的 `qq:dir:<dirId>`，不会把普通 `qq:<tid>` 猜成另一个身份。单批最多 100 项；结果中的 `extensions.results[]` 按请求顺序保留目录 ID、服务端返回身份、名称和 `deleted`。当前 `DelPlaylist` 会把含中文名称的 JSON 正文编码为 GBK；客户端先严格尝试 UTF-8，只在原字节不是合法 UTF-8 时完整按 GBK 解码，随后仍执行相同的包络码、`retCode` 和身份校验，空体或其他坏 JSON 不会被当成成功。QQ 对不存在目录仍返回业务成功但将 `dirId` 置为 `0`，TuneWeave 会返回 `deleted=false` 和 `all_deleted=false`，不会虚报真实删除；非零平台业务码仍作为错误返回，并附带已完成项以便调用方恢复。

歌单写入的 `refs` 是完整 `platform:id`，`ids` 是由路径或显式 `platform` 绑定的平台 ID；两者均接受单值、数组和逗号分隔字符串，但不能同时出现，输入顺序和重复项原样保留。批量删除和账户歌单排序不能混合平台。`/tracks` 只操作普通歌曲；`/videos` 只操作视频项目；`/items` 以 `kind=track|video` 选择，兼容参考 `type=0|3`。网易云创建结果会跳过零 ID，项目写入与排序结果会跳过空快照 ID，再采用后续有效兼容字段；`playlist_track_add/delete` 实际是 VIDEO 歌单的 `type=3` 项目接口，不会被错误复用为普通歌曲增删。

QQ 的普通歌单歌曲增删接受目标 `qq:<tid>`，当前账户自建目录使用可写的 `qq:dir:<dirId>`；创建结果和当前账户自建目录都直接以目录引用作为主引用，项目仅接受 QQ 歌曲引用。TuneWeave 会先用 Web 富详情把 MID 或数字 ID 解析为平台实际要求的 `songId + songType` 元组；同一请求内的重复引用只查询一次，但写入仍保留原顺序和重复项。这里不使用默认 `songType=0` 的数字歌曲批查，以免特殊类型歌曲被错误解析。添加分支保留 JSON 布尔 `bFmtUtf8=true`，删除分支按参考客户端的普通参数规范化发送；删除响应与歌单删除一样可能是 GBK JSON，解码后仍必须通过 CGI 顶层业务码、内层 `retCode`、返回 `dirId/tid` 及歌曲身份校验。任一层的 `80092` 都返回 `conflict`，空体、身份冲突或意外歌曲结果不会被包装成成功。QQ 详情读取在写操作后可能短时返回旧快照，写结果以已经交叉校验身份的 `result.songlist` 表示平台接受的事务结果，调用方需要强读一致性时应在有界等待后重新读取，不得重复发送写请求。

当前直接写入平台歌单要求资源已经能被目标 provider 接受；网易云因此要求项目引用属于网易云。Uni Playlist 与后续跨平台导入层在目标平台和歌曲来源平台不同时，必须先执行严格匹配；低于阈值时返回 `match_rejected`，不得把同名但不同版本的歌曲写入目标歌单。

歌曲喜欢写入由路径中的完整引用选择 provider，`account` 缺省为 `default`，未知查询字段会被拒绝。QQ 接入固定的“我喜欢”目录 `201`：TuneWeave 先查询歌曲详情，将 MID 或数值引用解析为平台要求的正数 `songId` 与 `songType`，再以所选 `(qq, account)` 凭据调用 `PlaylistDetailWrite/AddSonglist|DelSonglist`；不会把 MID 当数字、猜测歌曲类型或在缺失账户时访问歌曲服务。添加分支按参考协议保留 `bFmtUtf8=true`，删除分支使用普通 Android 参数规范化；两者除要求 CGI 包络与内层 `retCode=0` 外，还校验返回的 `result.dirId=201`、正数 `result.tid` 以及精确歌曲结果：添加必须只确认本次歌曲，删除必须返回空歌曲列表。删除成功正文可能是 GBK JSON，仅在原始字节不是合法 UTF-8 时进行完整限定解码，之后仍执行相同校验。普通账户已经完成 158→159→158 的可逆读写闭环，平台拒绝或矛盾结果不会虚报为已喜欢或已取消。

歌单收藏写入同样由完整引用选择 provider，并与歌曲喜欢使用不同能力。QQ 只接受正整数公开歌单 `qq:<tid>`，不会把 `qq:dir:<dirId>` 当成外部歌单；请求用所选账户的加密 UIN 调用 `PlaylistFavWrite/FavPlaylist|CancelFavPlaylist`。只有 CGI 顶层业务码为 0、响应 `result=0` 且目标不在 `v_failedPlaylistId` 中时才返回成功；收藏已存在或取消本就未收藏时，按 QQ 实际幂等结果处理，不由 TuneWeave 本地猜测。普通账户已完成公开歌单收藏目录 0→1→0 的真实可逆闭环，两次写响应均为 `result=0` 且失败列表为空，最终没有留下测试收藏。

评论目标类型接受统一名称 `track/mv/playlist/album/radio_episode/video/event/radio_station`，也兼容网易云参考数字 `0..7`；`song/music`、`dj/program`、连字符形式分别是对应统一类型的输入别名。`ref` 决定评论所属平台，`account` 只选择该平台的隔离登录态，评论 ID 始终按不透明字符串处理。事件评论的网易引用 ID 必须是从动态接口取得的完整 `A_EV_2_...` thread ID。创建、回复和删除使用同一评论写结果结构，明确返回目标、`create/reply/delete` 动作、可用的新评论 ID 和平台扩展；空白内容会被拒绝，但合法内容的首尾空格不会被静默改写。网易云三种写操作固定使用 EAPI `/api/resource/comments/add|reply|delete`，并由服务端取得 v2 checkToken 后注入请求头；客户端不能提交或覆盖 token。反应启用与停用则使用独立的 `CommentReactionMutationResult`，避免混淆评论本体动作和评论反应状态。

举报端点只把目标和账户选择统一化，不扩张平台能力。理由必填且只以去除空白后的结果判空，合法文本原样提交。网易云参考模块仅支持歌曲评论，因此该适配器只接受 `type=track`，固定构造 `R_SO_4_{id}` 并以 EAPI 调用 `/api/report/reportcomment`；其他目标在上游请求前返回 `invalid_request`。

头像请求省略 `filename` 与 `Content-Type` 时分别使用 `avatar.jpg` 和 `image/jpeg`；歌单封面分别使用 `playlist-cover.jpg` 和 `image/jpeg`。两者共享最大 20 MiB、非空图片和安全文件名校验；上传响应中的空 URL 不会遮蔽后续有效 URL。为兼容网易云参考项目，查询参数也接受 `imgSize/imgX/imgY` 与 `img_size/img_x/img_y`；该参考实现从首次引入起就没有把这三个裁剪参数传给上游，因此网易云适配器会接受并在扩展中标记 `applied=false`，不会虚假执行或声明裁剪。调用方应在上传前自行生成目标方形图片。

`POST /v1/account/cloud/uploads` 是兼容代理流程：调用方提交原始音频字节和必填安全文件名，`Content-Type` 省略时由 provider 按扩展名推断。TuneWeave 计算 MD5、解析音频标签、检查是否需要上传、上传 NOS、登记云盘信息并发布；显式 `song_name/artist/album` 优先于文件标签，标签按字段在主标签与备用标签间选择首个有效值，仍缺失时曲名取安全化文件主名，歌手和专辑分别使用“未知艺术家/未知专辑”。查询字段 `song`、`songName` 是 `song_name` 的兼容别名。该端点保持参考服务的 500 MiB 上限，并在单次请求期间持有一份音频缓冲；适合兼容和较小文件，不会把 NOS token 返回给调用方。

云盘大文件优先采用三段直传事务，避免让 TuneWeave 服务端持有整份音频：调用方先计算文件 MD5 与字节数并申请 `CloudUploadTicket`；仅当 `upload_required=true` 时，按返回的 `upload_method`、`upload_url` 和 `upload_headers` 原样上传音频字节；随后用票据中的临时曲目 ID 与资源 ID 调用完成端点。`upload_required=false` 时跳过字节上传，直接完成登记和发布。文件大小统一为字节，码率统一为 bit/s，省略码率时使用 `999000`。为兼容网易云参考参数，票据端点接受 `fileSize/contentType`，完成端点接受 `songId/resourceId/song`。

直传票据中的 `x-nos-token` 是短期敏感凭据，只能发送给同一票据给出的受限对象存储地址，不得写入日志、持久化或转发给其他来源。provider 必须限制上传目标域名和查询参数；网易云当前只接受无凭据、无自定义端口的 `http(s)://*.127.net` NOS 地址，并固定使用 `offset=0&complete=true&version=1.0`。普通 Debug 输出与 `extensions` 不包含该 token。

云盘免上传导入适用于文件已经被其他用户上传，或文件本身是目标平台音源的场景。TuneWeave 的 `bitrate` 仍统一使用 bit/s；网易云参考接口内部使用 kbps，因此 provider 执行 `floor(bit/s / 1000)`，调用方不得自行预除。省略 `source_track_id` 时使用参考默认 `-2`；歌手和专辑缺省时由网易 provider 使用“未知”。兼容字段为 `id/fileSize/fileType/song`；导入响应中的空或零首选歌曲 ID 会继续回退后续有效结果字段。

云盘歌词兼容查询字段 `uid/sid`。云盘匹配兼容 JSON 字段 `uid/sid/asid`，ID 可为字符串或数字；`target_track_id=0`、`asid=0` 或省略目标均表示取消现有匹配，而不是匹配到曲目 0。两项操作都只作用于查询参数选中的平台账户，不会改变其他平台登录态。

云盘资料库中的 `CloudTrack.ref` 是云盘条目 ID；内嵌 `track.ref` 仍是平台歌曲引用，两者不能互换。稳定字段包含文件名、文件大小、文件类型、码率、MD5、加入时间和可选的匹配歌曲引用，平台原始条目保留在扩展。空或非对象 `simpleSong`、空云盘 ID、零匹配 ID 不会遮蔽后续有效兼容字段。列表同时保留分页与容量统计。详情和删除只允许二选一提交 `refs` 或 `ids`：完整引用会推断平台，原始 ID 由显式或默认 `platform` 绑定；混合平台、平台冲突和两种选择器并用均在上游请求前拒绝，顺序和重复项不被静默改写。

网易云列表、详情和删除分别使用 WeAPI `/api/v1/cloud/get`、`/api/v1/cloud/get/byids` 和 `/api/cloud/del`；删除载荷依照参考实现保留为单元素的逗号拼接 ID 数组。源文件下载严格使用上游既有拼写的 EAPI `/api/cloud/dowonload`。普通 `/v1/tracks/{ref}/stream` 及下载端点也会把非默认 `account` 传入元数据解析与最终取流，确保同一云盘引用不会错误借用默认账户。

### 平台扩展

不能合理统一的功能放在 `/v1/extensions/{platform}`，仍使用统一包络和错误码。

| 方法 | 端点 | 用途 |
| --- | --- | --- |
| GET | `/v1/extensions/netease/calendar` | 查询指定毫秒时间范围内的网易云账户音乐日历 |
| GET/POST | `/v1/extensions/netease/anonymous-session` | 读取持久化匿名身份或注册/刷新 `MUSIC_A`；兼容 `/register/anonymous` 与参考拼写 `/register/anonimous` |
| GET/POST | `/v1/extensions/netease/register/anonymous` | 匿名身份的正确拼写兼容路由 |
| GET/POST | `/v1/extensions/netease/register/anonimous` | 匿名身份的上游参考拼写兼容路由 |
| GET/POST | `/v1/extensions/netease/check-token` | `version?=v2|v3`（默认 v3）、`refresh?`；读取缓存或注册/刷新网易云易盾 anti-cheat token |
| GET/POST | `/v1/extensions/netease/register/checktoken` | checkToken 的参考兼容路由，默认 v3 |
| GET/POST | `/v1/extensions/netease/register/checktoken/v2` | 固定读取或刷新 v2 token；另有固定 v3 路由，旧 `/register/checktoken` 别名默认 v3 |
| GET/POST | `/v1/extensions/netease/register/checktoken/v3` | 固定读取或刷新 v3 token |
| POST | `/v1/extensions/netease/api` | 在固定网易云域名上调用指定 `/api/...` 路径，支持 `eapi/weapi/api/linuxapi/xeapi` |
| GET | `/v1/extensions/netease/batch` | 以参考项目的查询参数形式批量调用网易云 `/api/...` 路径 |
| POST | `/v1/extensions/netease/batch` | 以 JSON 对象批量调用网易云 `/api/...` 路径 |
| POST | `/v1/extensions/qq/api` | 使用固定 QQ Android/Web 普通或签名 CGI 档案调用一个 `module + method + param` |
| POST | `/v1/extensions/qq/batch` | 在一次固定 QQ CGI 请求中执行最多 20 个有键子请求并保持键与响应对应关系 |

网易云日历接受统一参数 `start_time`、`end_time`，并兼容参考项目的 `startTime`、`endTime`；值必须是无符号 Unix 毫秒时间戳。为完整保留参考实现的运行时行为，任一时间参数省略时都会使用本次请求的当前毫秒时间，两个参数也允许同时省略。`account` 选择服务端保存的网易云登录态，也可省略账户并通过 `X-TuneWeave-Credential` 使用调用方托管凭证；两者不能同时用于网易云。端点固定使用 WeAPI 调用 `/api/mcalendar/detail`，成功时完整上游日历 JSON 位于统一包络的 `data` 中。

网易云匿名身份由服务端生成、保存和复用，不属于登录账户，也不会覆盖任一 `account` 别名。首次 GET 或 `refresh=true` 会生成参考格式的 52 位十六进制设备 ID，按客户端 DLL 的 XOR + MD5 + Base64 规则构造用户名，并通过 XEAPI `/api/register/anonimous` 注册；POST 始终强制刷新。成功结果包含 `device_id/cookie/registered/refreshed/extensions`，其中 `cookie` 为兼容参考响应而返回，但不会进入 Debug 或普通日志，也不能由调用方反向注入统一请求。设备 ID 与 `MUSIC_A` 作为一份身份原子写入私有数据目录，重启后继续用于默认公开请求；显式登录账户始终优先且保持隔离。实测 TuneWeave 与当前参考实现均收到上游业务码 400 且无 Cookie，因此代码完成但不伪造注册成功，待上游恢复后补成功态验收。

网易云 checkToken 同时提供通用 `/v1/extensions/netease/check-token`、旧参考语义别名 `/v1/extensions/netease/register/checktoken` 和固定版本的 `/v2`、`/v3` 路由。通用端点以 `version=v2|v3` 选择版本，缺省 v3；GET 缺省复用对应版本的进程内缓存，`refresh=1|true` 强制刷新，POST 始终强制刷新。返回 `version/token/registered/refreshed/extensions`，账户客户端共享缓存但 v2/v3 严格隔离；要求 v2 的 EAPI 和要求 v3 的 XEAPI 能力分别在服务端自动注册并以 `X-antiCheatToken` 请求头使用。token 不接受客户端注入，也不会进入 Debug 或普通日志；v2 注册响应严格校验成功 JSON 和非空 `result.conf`，v3 严格校验成功 JSONP，两者都校验为安全 HTTP 头值。

网易云通用扩展请求：

```json
{
  "uri": "/api/search/get",
  "data": {
    "s": "TuneWeave",
    "type": 1,
    "limit": 1,
    "offset": 0
  },
  "crypto": "eapi",
  "account": "default"
}
```

`crypto` 可取 `eapi`、`weapi`、`api`、`linuxapi`、`xeapi`，省略时使用 `eapi`；`protocol` 是 `crypto` 的输入别名。成功时上游 JSON 位于统一包络的 `data` 中。该端点用于覆盖参考项目自身的通用 `/api` 能力以及尚无合理统一语义的调试场景，不替代其余模块的逐项统一映射与验收。

为避免把通用入口变成凭据注入或 SSRF 接口，请求 `uri` 只能是非空 `/api/...` 路径，目标域名由服务端配置且不能由调用者覆盖；请求体拒绝 `cookie`、`domain`、`headers`、`proxy`、`ua` 等传输覆盖字段，`data.cookie` 也会被拒绝。登录态可通过 `account` 选择服务端保存的账户别名，或省略账户并通过 `X-TuneWeave-Credential` 提交 TuneWeave 版本化网易云凭证封装；两种来源同时出现会在访问上游前拒绝，平台原始 Cookie、密钥或 token 字段始终不接受。XEAPI 的公钥注册、X25519 会话密钥、anti-cheat token 请求头与加密响应解包均由适配器内部完成。

网易云传输身份只能由服务端启动配置选择：`TUNEWEAVE_NETEASE_PROXY` 接受 HTTP(S) 正向代理，`TUNEWEAVE_NETEASE_REAL_IP` 接受固定 IPv4 地址，`TUNEWEAVE_NETEASE_RANDOM_CN_IP=true` 则在 provider 启动时生成一个地址，并按照参考实现 `generateConfig()` 产生 `global.cnIp` 的实际作用域，由该实例后续的 EAPI、WeAPI、明文 API、LinuxAPI、XEAPI 及密钥注册请求共同复用，而不是逐请求重新随机。短信验证码发送前还会加载或注册持久匿名设备会话，发送成功后把该设备会话按国家码和手机号在内存中绑定 10 分钟；校验与登录复用它，登录成功即删除临时绑定。手机号和验证码不会持久化。固定和随机身份互斥；启用后同一个地址同时写入 `X-Real-IP` 与 `X-Forwarded-For`。为了保持小体积，随机生成器采用参考实现内置的 `116.25.0.0` 至 `116.94.255.255` 中国地址兜底范围，不把四千余条 CIDR 数据嵌进二进制。代理地址与 IP 不接受 HTTP 参数或 JSON 覆盖，代理认证信息不会进入错误和日志，媒体资源下载及对象存储上传也不会附加伪造来源头。

网易云批量扩展请求支持结构化容器：

```json
{
  "requests": {
    "/api/v2/banner/get": {
      "clientType": "pc"
    },
    "/api/search/get": {
      "s": "TuneWeave",
      "type": 1,
      "limit": 1
    }
  },
  "crypto": "eapi",
  "encrypted_response": true,
  "account": "default"
}
```

POST 也兼容参考项目把 `"/api/..."` 直接放在顶层的写法；GET 则兼容 `/v1/extensions/netease/batch?/api/v2/banner/get={"clientType":"pc"}`。查询中的 JSON 应正常进行 URL 编码。`protocol` 是 `crypto` 的别名，`e_r` 是 `encrypted_response` 的别名；布尔值兼容 `true/false` 与 `1/0`。五种 `crypto` 值与通用扩展相同。

上游真实批量协议要求每个子请求参数最终是 JSON 文本。调用者传入对象、数组、数字、布尔或 `null` 时适配器会自动序列化，已传入的字符串保持原样，因此参考项目的 GET 字符串形式和 POST 对象形式均可用。响应不重排或折叠子请求结果，上游顶层 `code` 及各 `/api/...` 键原样位于统一包络的 `data` 中。

每个批量键都会独立校验为固定网易云域名下的非空 `/api/...` 路径；空批次、重复键以及原始 Cookie、域名、代理、请求头、UA、伪造 IP、客户端超时或检查令牌覆盖都会被拒绝。账户凭据可通过 `account` 别名选择，或改用 `X-TuneWeave-Credential` 中的网易云封装；两种来源不能并用，批量体内也不能注入原始凭据。`e_r=true` 的响应解密由适配器内部完成。

QQ 通用扩展以结构化字段选择 CGI 服务，不接受 URL：

```json
{
  "module": "music.musicToplist.Toplist",
  "method": "GetAll",
  "param": {},
  "client": "android",
  "signed": false,
  "preserve_booleans": false,
  "allow_error_codes": [],
  "account": "default"
}
```

`client` 只接受 `android` 或 `web`；`signed=true` 仍由服务端固定选择 `musics.fcg`、生成时间戳和 `zzc` 签名，调用方不能覆盖目标域名或签名输入。Android 请求可以省略账户保持匿名、用 `account` 精确选择已保存的 `(qq, account)` 凭据，或通过 `X-TuneWeave-Credential` 使用调用方托管的 QQ 凭证；同平台两种来源不能并用。Web 档案当前只允许匿名，避免把未经真实确认的 Web 登录参数伪装成可用能力。`param` 省略时为空对象，兼容输入名 `params/data`；默认按 QQ 移动协议递归把布尔值转换为 `0/1`，只有已知需要原生 JSON 布尔的调用才设置 `preserve_booleans=true`。

批量请求把相同字段放在 `requests` 的有键对象中，并在批次顶层统一选择 `client/signed/account`。批次不能为空且最多 20 项，每个标签、模块和方法均采用有界安全字符集；每个参数对象限制为 1 MiB、32 层和 20000 个 JSON 节点。顶层与任意嵌套层的 Cookie、token、QQ 音乐密钥、OAuth 标识、`comm`、QIMEI、目标 URL/域名、代理、请求头及 UA 字段都会在读取账户或访问网络前拒绝。响应按调用方标签返回；其中的凭据、Cookie、会话密钥和设备身份字段会递归替换为 `[redacted]`，正常媒体/图片 URL 与分页等组合字段（例如 `pageToken`）不会被误删。

QQ 子请求的非零业务码默认映射为统一认证、参数、权限、冲突、限流或上游错误。只有调用方明确列入 `allow_error_codes` 的非零码才作为原始响应返回；成功码 `0`、重复项和超过 32 个显式码会被拒绝。该机制用于平台特有的“非零码也是可解释状态”，不能跳过顶层批请求失败，也不能改变统一错误语义。

## 跨平台回退流程

1. 从 `origin_track` 读取标准化标题、歌手、专辑、时长和 ISRC。
2. 按 `playback_platforms` 尝试；目标平台与来源平台不同则先搜索候选。
3. 计算匹配分数：ISRC、规范化标题、主要歌手、专辑、时长依次参与；伴奏、Live、翻唱、Remix、纯音乐等版本标签单独惩罚。
4. 严格模式低于阈值时拒绝候选，不因“同名”直接换源。
5. 使用该平台指定账户解析媒体地址；无 URL、仅取得试听、权益不足、地区限制或上游错误时进入下一平台。
6. 成功响应同时返回来源引用、命中引用、分数和所有尝试轨迹。

试听流属于可播放的最后兜底，不等同于完整授权。只要有后续回退平台，resolver 就先把该次尝试记为 `permission_denied` 并继续寻找完整流；后续任一平台成功时返回完整流。只有所有后续尝试都失败时，才恢复顺序中最早的试听流为最终 `success`，保留精确 `TrialWindow` 及后续失败轨迹。`fallback=false` 或精确平台序列只包含当前平台时不会额外搜索，平台已授权的试听可直接返回。该语义同时适用于普通歌曲、Uni Playlist 和调用方托管项目，避免会员试听片段提前截断跨平台替代播放。

网易云歌单中的歌曲使用 QQ 绿钻账户取流示例：

```http
GET /v1/tracks/netease:123456/stream?quality=lossless&playback_platform=qq&account=green-diamond&fallback=true
```

`attempts` 示例：

```json
[
  {
    "platform": "qq",
    "account": "green-diamond",
    "candidate": "qq:0039MnYb0qxYhV",
    "match_score": 0.98,
    "status": "success",
    "error": null
  }
]
```

默认音乐回退顺序不包含 B 站，以免把翻唱、现场或二创视频误当成录音室版本。调用者显式加入 `bilibili` 时仍执行严格版本匹配。
