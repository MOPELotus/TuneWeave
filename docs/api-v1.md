# TuneWeave HTTP API v1

状态：首版实现契约。平台适配器可以逐步接入，但已实现端点不得改变这里定义的字段语义。

## 基础约定

- 默认监听地址：`127.0.0.1:7832`。
- API 前缀：`/v1`；存活检查 `/healthz` 不带版本前缀。
- 请求与响应使用 UTF-8 JSON。媒体数据不经过 TuneWeave 中转，接口返回带有效期的 URL 与必要请求头。
- 所有平台原始 ID 按字符串处理。公开引用写成 `<platform>:<id>`，例如 `netease:123456`、`qq:0039MnYb0qxYhV`、`bilibili:BV1xx411c7mD`。
- 时间使用 RFC 3339；时长统一为毫秒；文件大小统一为字节；码率统一为 bit/s。

### 平台选择

| 参数 | 含义 |
| --- | --- |
| `platform` | 目录或账户所属平台。省略时使用服务配置的默认平台；搜索允许使用 `all` 做多平台聚合。 |
| `account` | 同一平台内的账户别名，默认 `default`。不会向客户端返回账户 Cookie/token。 |
| `playback_platform` | 首选播放来源。它只影响媒体解析，不改变原歌曲引用。 |
| `fallback` | 播放失败时是否继续尝试其他平台，默认 `true`。 |
| `fallback_platforms` | 本次请求的有序回退列表，逗号分隔；省略时使用服务器策略。 |

当路径中的引用已经带平台时，引用平台是内容来源；查询参数 `platform` 不能覆盖它。账户端点没有内容引用，因此通过 `platform` 选择账户平台。

账户别名的作用域是平台，同名的 `netease/personal` 与 `qq/personal` 是两份独立登录态。二维码、密码或验证码登录成功后，provider 只把后续请求所需的会话凭据写入服务端私有账户存储；重启时按 `platform/account` 恢复，所有账户端点和播放端点继续用请求中的 `account` 选择，不存在的非默认别名不会回退到 `default`。密码、验证码和二维码事务本身不持久化。

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
    "request_id": "01J...",
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
    "request_id": "01J..."
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

稳定歌手详情合并平台的身份详情与传记能力。网易云的 `/artist/detail` 提供名称、图片、身份与作品计数，`/artist/desc` 提供简介和分段传记；TuneWeave 将二者组合为一个 `Artist`。无法跨平台统一的认证、排行、黑名单和专题数据分别保留在 `extensions.detail_response` 与 `extensions.description_response`，不会因统一映射丢失。

歌手分类目录使用跨平台枚举：`type=all|male|female|group`，`area=all|chinese|western|japanese|korean|other`，`initial` 接受单个英文字母、`hot` 或 `other`。适配器负责转换平台数值；列表上游没有可靠总数时 `total=null`，通过 `has_more/next_offset` 继续翻页。

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

视频详情端点返回 `VideoDetail`，以 `kind=mv|video` 明确资源类型，`video` 承载统一元数据，`resolutions` 列出平台实际公布的清晰度及可用的宽高、大小和格式。网易云数值 ID 默认推断为 MV，不透明字符串 ID 默认推断为站内视频；调用方也可通过 `kind`（兼容 `type`）显式指定，避免依赖推断。

`VideoStats` 独立返回点赞态以及点赞、评论和分享计数。`VideoStream` 以 `available` 和可空 `url` 表达取流结果，同时保留备用地址、请求/实际清晰度、大小、时长、业务码、费用和平台原文；平台成功响应没有 URL 时仍返回可检查的成功数据，不会伪造播放地址。`resolution` 兼容 `res`，默认 1080；网易云接受 1–4320，并把上游实际命中的清晰度写入 `actual_resolution`。

已关注歌手的新视频与新曲时间线分别返回 `Video[]` 和 `Track[]`，但都属于账户资源。它们以毫秒时间戳 `before` 翻页，并在 `meta.pagination.extensions.next_before_ms` 返回下一页起点；`account` 只选择登录态，不改变内容平台。

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

两类资源都通过 `GET /v1/playlists/{ref}` 和 `GET /v1/playlists/{ref}/tracks` 访问。公开内容通常不需要账户；私有收藏夹由 `account` 选择 B 站登录态。由于 TuneWeave 的主要用途是音乐播放，列表中的 B 站视频会规范化为可播放的 `Track`，并在 `extensions.video_ref`、`extensions.bilibili_playlist_kind`、`extensions.aid`、`extensions.bvid`、`extensions.cid` 中保留完整视频身份；原始视频详情仍通过 `/v1/videos/{ref}` 读取。

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

普通音乐榜单目录使用独立 `ChartCatalog`，不再伪装成普通歌单数组。`view=overview|summary|modern` 分别表示平台的榜单介绍、经典内容摘要和新版分组摘要；默认 `summary`。新版中可播放榜单保留可用于 `/v1/charts/{ref}/tracks` 的引用，H5 等非歌单入口保持 `ref=null` 并通过 `target_kind/target_url` 表达。预览项只有平台给出真实歌曲 ID 时才返回 `track_ref`；完整目录、分组、榜单及排名原文均保留在对应 `extensions`。

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

`PUT /v1/account/podcasts/{ref}/episodes/order` 通过独立 `podcast_episode_order_write` 能力调整声音在账户声音歌单中的固定序号；路径引用是声音歌单，JSON `episode` 是声音本身，两者必须属于同一平台。`position` 从 1 开始，超出节目数时由上游移动至末尾；`limit/offset` 完整保留参考排序接口用于定位工作台页的控制，缺省分别为 200/0。网易云固定调用 EAPI `/api/voice/workbench/radio/program/trans`，精确提交 `limit/offset/radioId/programId/position`，要求 `account` 选择已登录隔离会话，成功结果保留完整响应。2026-07-22 真实匿名协议请求到达上游并返回 `code=400`“只允许操作自己的播客”，统一端点对缺失账户别名返回 401；真实拥有者的成功重排留待使用创作者账户验证。

`DELETE /v1/account/podcast-episodes/{ref}` 删除单条账户声音；`DELETE /v1/account/podcast-episodes` 以 JSON `refs` 或 `ids` 提供有序批量删除。`refs` 接受完整引用数组或逗号字符串，`ids` 接受所选 `platform` 的裸 ID 数组或逗号字符串，两者不能同时出现；输入顺序和重复项原样保留，不擅自去重。该操作通过独立 `podcast_episode_delete_write` 能力发现，网易云固定调用 EAPI `/api/content/voice/delete` 并以逗号拼接的 `ids` 精确复刻参考批量协议。参考文档把该字段误称为 `voiceListId`，但实际服务方法、路径和同协议调用均是声音 ID，因此统一模型不会把它误作删除整个声音歌单。删除要求 `account` 选择已登录隔离会话；2026-07-22 使用空凭据目录的真实服务器验证缺失别名在发网前返回 401，破坏性成功分支留待可丢弃的自有声音验收。

`POST /v1/account/podcasts/{ref}/episodes` 接收原始音频请求体并完成账户声音上传，最大 500 MiB；`Content-Type` 是音频类型，查询必须给出 `filename/cover_image_id/category_id/second_category_id/description`，并可给出 `name/privacy/publish_time_ms/auto_publish/auto_publish_text/order_no/composed_songs/account`。所有字段兼容参考 camelCase 名称；布尔值接受 `true/false/1/0`，`order_no` 缺省 1 且最小 1，发布时间缺省 0（立即发布），`composed_songs` 接受逗号分隔的同平台裸歌曲 ID 或完整引用并保留顺序和重复项。音频字节只存在于脱敏请求模型和上传事务中，不进入 JSON、Debug 或扩展。网易云实现完整执行 WeAPI `/api/nos/token/alloc`（`ymusic`）→ 固定 `ymusic.nos-hz.163yun.com` 的 10 MiB NOS 分片上传与 XML 完成 → EAPI `/api/voice/workbench/voice/batch/upload/preCheck` → EAPI `/api/voice/workbench/voice/batch/upload/v2`；两次提交按参考行为使用不同 RFC 4122 v4 `dupkey`，并携带 NOS token 请求头，但 token 不会写入结果或日志。2026-07-22 空凭据真实服务器验证完整输入在发网前稳定返回 401；真实上传、发布后详情与播放验收留待创作者账户及可丢弃音频。

`GET /v1/account/podcasts/created` 返回所选登录账户创建的播客/声音歌单，通过 `account_created_podcasts` 能力与订阅库 `/v1/account/library/podcasts` 分开。当前网易云固定以 WeAPI 调用 `/api/social/my/created/voicelist/v1`，只接受参考实现真实支持的 `limit`（缺省 20），不接收或伪造 offset；统一分页因此固定 `offset=0/next_offset=null/has_more=false`，并以 `continuation_supported=false` 明示这是不可续页快照。空的旧列表包装不会遮蔽后续非空兼容列表，创作者状态与完整包装字段保留在 `Podcast.extensions`。

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
| GET | `/v1/search` | `q`（也接受 `keywords`）、`type?`（也接受 `kind`）、`variant?`、`platform?`、`account?`、`search_id?`（也接受 `searchid`）、`highlight?`、分页 | 带 `type/data` 判别字段的统一 `SearchItem[]`；未知查询字段会被拒绝而不会静默采用缺省类型 |
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
| GET | `/v1/tracks/{ref}` | `account?` | `Track` |
| GET | `/v1/tracks/{ref}/availability` | `account?`、`bitrate?`（默认 999000，也接受 `br`） | `TrackAvailability`；不可播仍返回成功包络与 `playable=false` |
| GET | `/v1/albums` | `platform?`、`account?`、`catalog=new|newest`、`area?`、分页 | `Album[]` |
| GET | `/v1/albums/{ref}` | `account?` | `Album` |
| GET | `/v1/albums/{ref}/tracks` | 分页、`account?` | `Track[]` |
| GET | `/v1/albums/{ref}/track-entitlements` | 分页、`account?` | `TrackEntitlement[]` |
| GET | `/v1/albums/{ref}/stats` | `account?` | `AlbumStats` |
| GET | `/v1/digital-albums` | `platform?`、`account?`、`catalog=latest|style`、`area?`、`type?`、分页 | `DigitalAlbum[]`；上游不返回可靠总数时 `total=null` |
| GET | `/v1/digital-albums/{ref}` | `account?` | `DigitalAlbum` |
| GET | `/v1/charts` | `platform?`、`account?`、`view=overview|summary|modern`（也接受 `catalog` 及网易模块名别名） | `ChartCatalog`；默认经典内容摘要 |
| GET | `/v1/charts/podcasts` | `platform?`、`account?`、`kind?=new|hot|paid`（默认 `new`，也接受 `type`）、`limit?`、`offset?` | `PodcastChartEntry[]`；排名包装与完整播客分离，不伪造上游实际不支持的续页 |
| GET | `/v1/charts/podcast-creators` | `platform?`、`account?`、`kind?=newcomer|popular|trending24_hours`（默认 `newcomer`，也接受 `type/new/hot/hours/24h`）、`limit?`、`offset?` | `PodcastCreatorChartEntry[]`；排名、粉丝数与完整用户身份分离，不伪造榜单续页 |
| GET | `/v1/charts/artists` | `platform?`、`account?`、`area=chinese|western|korean|japanese`（也接受 `type=1|2|3|4`） | 完整 `ArtistChart` 快照 |
| GET | `/v1/charts/{ref}/tracks` | 分页、`account?` | `Track[]`；引用来自可播放榜单项 |
| GET | `/v1/charts/digital-albums` | `platform?`、`account?`、`period=daily|week|year|total`、`type=album|single`、`year?`、分页 | `DigitalAlbumChartEntry[]` |
| GET | `/v1/charts/dimensions/{chart_code}` | `target_id`、`target_type`、`platform?`、`account?` | `DimensionChart`；也接受参考字段 `targetId/targetType` |
| GET | `/v1/charts/dimensions/{chart_code}/tracks` | `target_id`、`target_type`、`platform?`、`account?` | 完整 `DimensionChartTrackSnapshot`；无分页元数据 |
| GET | `/v1/artists` | `platform?`、`account?`、`type`、`area`、`initial`、分页 | `Artist[]`；分类歌手目录 |
| GET | `/v1/artists/{ref}` | `account?` | `Artist`；身份详情与分段传记，平台原始附加信息保留在扩展字段 |
| GET | `/v1/artists/{ref}/overview` | `account?` | `ArtistOverview`；歌手摘要、精选 `Track[]` 与是否仍有更多曲目 |
| GET | `/v1/artists/{ref}/stats` | `account?` | `ArtistStats`；关注态、视频分类计数与在线演出计数 |
| GET | `/v1/artists/{ref}/tracks` | `order=hot|time`、分页、`account?` | `Track[]`；默认按热度排序，完整平台曲目字段保留在单项扩展 |
| GET | `/v1/artists/{ref}/top-tracks` | `account?` | 热门 `Track[]` 固定快照；不接受伪分页，`has_more=false` |
| GET | `/v1/artists/{ref}/albums` | 分页、`account?` | `Album[]`；歌手级上游信息保留在分页扩展 |
| GET | `/v1/artists/{ref}/fans` | 分页、`account?` | `User[]`；上游无可靠总数时 `total=null` |
| GET | `/v1/artists/{ref}/videos` | `type=mv|all`、分页、`cursor?`、`order?`、`account?` | `Video[]` |
| GET | `/v1/videos` | `platform?`、`account?`、`catalog=all|latest|exclusive|timeline_all|timeline_recommended|group`、`area?`、`type?`、`order?`、`group_id?`、分页 | `Video[]`；MV 与站内视频目录按后端真实能力约束筛选及续页 |
| GET | `/v1/videos/taxonomy` | `platform?`、`account?`、`kind/type=categories|groups`、分页 | `VideoCatalogOption[]`；视频分类或完整标签目录 |
| PUT/DELETE | `/v1/account/library/videos/{ref}` | `account?`、`kind/type?=mv|video` | `SubscriptionResult`；收藏或取消收藏视频资源，平台只支持其中一种资源时明确拒绝其余类型 |
| GET | `/v1/account/library/videos` | `platform?`、`account?`、分页 | 已收藏 `Video[]`；MV 与普通视频按平台真实返回共同映射，来源类型及完整条目保留在扩展 |
| GET | `/v1/playlists/{ref}` | `account?` | `Playlist`；`uni:` 复用同一实体，混合项目数位于 `extensions.uni_item_count`，不伪装成纯歌曲数 |
| GET | `/v1/playlists/{ref}/items` | 分页、`account?` | `PlaylistPlayableEntry[]`；统一返回 `track/mv/video/podcast_episode/radio_station`、资源引用、位置与紧凑快照；Uni 项提供稳定 `item_id`，外部只读项目为 `null` |
| GET | `/v1/playlists/{ref}/tracks` | 分页、`account?` | `Track[]`；混合 Uni 歌单先过滤非歌曲再计算真实分页，B 站合集/收藏夹视频按可播放音频内容归一并保留 `video_ref` |
| GET | `/v1/playlists/{ref}/items/{item_id}/stream` | 音质/后端/码率、`playback_platform?`、`fallback?`、`fallback_platforms?`、`unblock?`、`source?`、`account?`、`accounts?`、视频 `resolution?` | `UniPlaylistItemStream`；稳定项目身份、原始资源和统一 `MediaStream` 分离 |
| GET | `/v1/playlists/{ref}/items/{item_id}/stream/redirect` | 同上 | 成功解析真实 URL 后返回 302 |
| GET | `/v1/resources/{type}/{ref}/comments` | `account?`、`view?`、`sort?`、评论分页参数 | `target/comments/hot_comments/top_comments/current_comment/extensions`；统一评论目录，分页位于 `meta.pagination` |
| GET | `/v1/resources/{type}/comments/stats` | `platform?`、`ids?`、`account?` | `CommentThreadStatsBatch`；同类型资源的批量评论、分享、点赞及最新条目统计 |
| GET | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `target_user_ref`、`account?`、`limit/page/cursor/id_cursor?` | `target/comment_id/target_user_ref/kind/reactions/current_comment/extensions`；评论反应用户目录 |
| PUT | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；启用评论反应 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；停用评论反应 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/reports` | 查询参数 `account?`；JSON `{reason}` | `CommentReportResult`；举报评论 |
| GET | `/v1/users/{ref}` | `account?`、`backend?=modern|legacy`（也接受 `variant/source`） | 指定用户的完整 `UserProfile`；引用决定平台 |
| GET | `/v1/users/{ref}/favorites/tracks` | 分页、`account?` | 指定用户公开引用下的 `Track[]`；需要平台登录态时由 `account` 选择 |
| GET | `/v1/users/{ref}/membership` | `account?`、`backend=front|client` | 指定用户的 `MembershipSummary`；引用决定平台，客户端后端要求登录 |
| GET | `/v1/users/{ref}/history` | `period=all_time|week`、分页、`account?` | 指定用户的 `PlaybackHistoryEntry[]` |
| GET | `/v1/recommendations/tracks` | `platform?`、`account?`、`source?=daily|personalized`、`refresh?`、`area_id?`、分页 | `Track[]`；推荐理由和首页包装保存在扩展 |
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

搜索类型缺省为 `track`，既接受统一名称，也接受网易云参考数字：`track|song|1`、`album|10`、`artist|100`、`playlist|1000`、`user|1002`、`mv|1004`、`lyric|lyrics|1006`、`podcast|dj|dj_radio|1009`、`radio_station|radio|broadcast`、`video|1014`、`mixed|complex|1018`、`voice|2000`。`podcast` 表示可点播的播客目录，`radio_station` 表示直播广播频道；两者不会因为平台字段名含 `radio` 而混为同一实体，平台没有直播广播搜索时会明确返回能力不支持。`variant` 支持 `default|legacy|cloud`，也兼容 `backend` 字段以及 `search/cloudsearch/auto` 值；缺省时由 provider 使用推荐后端。网易云缺省播客搜索精确对应参考 `/voicelist/search`，使用 EAPI `/api/search/voicelist/get`；`legacy` 精确对应参考 `/search`：普通类型使用 `/api/search/get`，声音使用独立 `/api/search/voice/get`；`cloud` 对应 `/cloudsearch`。每一项统一序列化为 `{type,data}`；歌曲、专辑、歌手、歌单、用户、MV/视频、播客及广播电台使用对应统一实体，其中 MV 与视频均为 `video`，歌词搜索以 `track` 返回并把命中的平台歌词原文保存在曲目扩展。网易云 1009/`djRadios` 按 `Podcast` 映射；专用播客响应的 `baseInfo` 提升为稳定实体，外层算法与命中理由保留在 `extensions.search_item`。综合搜索、声音或上游出现尚无稳定公共结构的条目使用 `opaque`，保留平台、搜索类型、可提取的 ID/标题及完整原文。声音响应同时出现专用 `voices/voiceCount` 与通用 `resources/resourceCount` 时优先专用字段；空的旧数组或空 `result` 不会遮住后续非空数组或旧版 `data`。实际后端和上游路径位于分页扩展 `variant/request_path`，完整上游响应也保存在分页扩展；上游若不应用请求 `limit`，TuneWeave 返回真实条目并显式写入 `limit_applied=false`，不会截断后伪装成已应用分页。

QQ 的歌曲、歌手、专辑、歌单、MV、歌词、用户、节目专辑和节目九类分类固定使用 Android `music.search.SearchCgiService/DoSearchForQQMusicMobile`，启动时生成并在 `TUNEWEAVE_DATA_DIR/qq-device.json` 私有持久化 GUID、Android ID、IMEI、QIMEI 和匿名会话。QQ 会在过大的 `num_per_page` 下以 `code=0` 静默返回空目录，因此 TuneWeave 对已真实验收的类别使用歌曲/专辑/MV/歌词 60、歌手 40、歌单 30 的页宽；用户/节目专辑/节目使用上游公开测试明确覆盖的 10，并用同批子请求按上游逻辑槽位实现统一 `limit=1..100` 与任意 `offset`。`search_id/searchid` 可复用 QQ 搜索会话，省略或留空时按参考算法自动生成；`highlight` 缺省为 `false` 以保持统一标题可直接使用，显式启用时原样传给 QQ，实际会话 ID 与高亮开关均返回在分页扩展。歌单结果桶还可能比其逻辑页少一项；TuneWeave 不会从窗口外补项或造成下一页重复，而是按逻辑窗口推进 `next_offset`，在分页扩展以 `pagination_basis=upstream_slots`、`omitted_slots` 和 `upstream_item_counts` 透明说明，因此返回数允许小于 `limit`。非稀疏分类在窗口内缺项、非零或缺失的顶层/子请求/数据码、缺失总数/列表均不会被当作成功。歌曲公开引用优先使用 MID，数字 ID、MID、媒体 MID、`songType`、文件规格和付费信息分别保存在扩展；歌词命中仍返回完整歌曲，并额外保存实际命中文本；歌手/专辑优先使用 MID，歌单使用 `dissid`，MV 使用 VID；用户优先使用可访问主页的加密 UIN 并另存数字 UIN；节目专辑映射为 `podcast`，节目映射为包含完整可播放 `Track` 的 `podcast_episode`，所属节目专辑引用与音频引用保持分离。创建者标识、计数、日期、标签及每个分类的完整原项分别保留。`variant` 暂只接受 `default`，QQ 账户别名在登录能力接入前会明确返回认证前置错误而不会被静默忽略。2026-07-22 前六类已通过真实 provider 与 release HTTP 验收；最后三类的代码和离线映射测试完成，但真实单批复验当前被 QQ 匿名风控以 `code=2001` 拒绝，未伪造成功态。

默认搜索词与搜索结果分离：`keyword` 是应提交给搜索端点的真实词，`display_text` 是可直接展示的文案，`kind` 仅在平台类型可映射时返回，图片允许为空。网易云固定使用 EAPI `/api/search/defaultkeyword/get`；空白 `showKeyword` 会继续回退 `styleKeyword.keyWord`，算法、样式词和业务意图等动态字段完整保留在 `extensions.response`，调用方不应解析它们来替代稳定字段。

热搜目录按 `rank` 从 1 开始排序，`keyword` 必填，说明、分数、图标类型、图标 URL 和目标 URL 均按平台实际返回可空。`detail` 缺省为 `full`，也接受 `brief`，并兼容 `mode` 查询名及 `simple/detail/detailed` 值。网易云简略榜固定使用 EAPI `/api/search/hot` 和 `type=1111`，详细榜固定使用 WeAPI `/api/hotsearchlist/get`；两套响应不会互相补造缺失字段，完整原文位于列表与条目扩展。

搜索建议的 `client` 缺省为 `web`。统一条目始终给出可直接重新搜索的 `keyword`，可选 `kind/display_text/icon_url`；web 建议中的歌曲、专辑、歌手、歌单等实际资源同时以统一 `SearchItem` 放入 `resource`，mobile/PC 纯关键词不会伪造资源。PC 的 `recs` 与普通 `suggests` 分别位于 `recommendations/suggestions`。网易云 web/mobile 分别固定使用 WeAPI `/api/search/suggest/web`、`/api/search/suggest/keyword`，PC 固定使用 EAPI `/api/search/pc/suggest/keyword/get`；未知或零 `type` 不会遮住可映射的 `resourceType`，为兼容参考输入，`type=mobile` 等同 `client=mobile`。

QQ 的 `client=mobile` 精确对应 Android `music.smartboxCgi.SmartBoxCgi/GetSmartBoxResult`：普通 `items` 与 `vec_related_items` 分别进入 `suggestions/recommendations`，`vec_direct_items` 依据 `insert_pos` 插回建议序列并尽可能提升为统一资源。搜索会话 ID 位于列表扩展；高亮展示、图标、跳转、分值、预搜索标志、关联资源 ID 及完整上游包装逐项保留。当前 `client=web` 留给参考 `quick_search` 的独立 Smartbox HTTP 链，`pc` 没有对应上游分支；两者在接入前明确报错，不会偷换成移动端结果。2026-07-22 release 统一 HTTP 真实返回 21 项“周杰伦”建议，首项为歌手直达资源。

多重搜索的 `kind` 缺省为 `track`，接受与普通搜索相同的统一名称和网易云数字类型，参考字段 `type` 是其别名。网易云固定使用 WeAPI `/api/search/suggest/multimatch` 并精确提交 `s/type`；非空 `result.orders` 决定分区顺序，值为 `null` 时回退兼容字段 `order`，未列入顺序但实际返回的数组仍会追加保留。`artist/playlist/new_mlog` 等已知分区分别规范化为统一歌手、歌单与视频资源，空/零视频或创作者 ID 和零时长不会遮蔽有效兼容值，未知分区以不透明条目表达；完整上游响应位于结果扩展。

本地歌曲匹配的 `md5` 必填并按 32 位十六进制校验，标题、专辑和歌手允许为空以保留参考模块的默认分支；时长省略时按参考行为使用 0。若同时提供毫秒与秒数，两者四舍五入到毫秒后必须一致。网易云固定使用未加密直连 API `/api/search/match/new`，把一项标签记录序列化进 `songs`；上游 `result.ids/songs` 分别映射为匹配 ID 和统一候选曲目，空数组原样表达无命中。

用户完整资料的 `backend` 缺省为 `modern`，也接受 `new/eapi/v2`；网易云精确对应参考 `user_detail_new`，以 EAPI 调用 `/api/w/v1/user/detail/{uid}` 并提交字符串 `all=true/userId`。`backend=legacy`（也接受 `old/weapi/v1`）精确对应 `user_detail`，以空载荷 WeAPI 调用 `/api/v1/user/detail/{uid}`。两条路径共用 `UserProfile`，但通过独立能力和 `extensions.backend/response` 保留实际后端及完整响应。空包装、空文本、零时间戳不会遮蔽后续有效兼容字段，返回用户 ID 与请求不一致时按上游错误拒绝。`/v1/account/profile` 先从指定 `platform/account` 的持久登录态取得用户 ID，再以同一账户请求资料，不会借用默认账户或把登录凭据写入响应。2026-07-22 已真实验证公开 legacy/modern 及持久账户 modern 三条统一 HTTP 路径。

会员摘要同时提供公开用户和当前账户两条统一路径。`backend` 缺省为 `front`，也接受 `public/v1`；网易云固定使用 WeAPI `/api/music-vip-membership/front/vip/info`，公开用户把引用 ID 作为 `userId`，当前账户按参考默认分支提交空字符串。`redVipLevel/redVipAnnualCount/redVipLevelIcon` 分别映射为等级、年费次数和图标；该公开接口没有可靠有效期和激活态，因此相关字段保持可空。

`backend=client`（也接受 `detail/v2`，字段名兼容 `variant/source`）通过独立 `user_membership_client_info` 能力精确对应参考 `/vip/info/v2`，固定使用 WeAPI `/api/music-vip-membership/client/vip/info`。该分支无论是否指定用户都要求 `account` 指向已登录会话，不会静默回退公开摘要；`redplus/musicPackage/associator/voiceBookVip/albumVip` 的最长有效期驱动稳定激活态和到期时间，等级、年费次数和非空动态图标映射到稳定字段，全部权益包及未来平台字段保留在 `extensions.response`。

广告换听目录的 `type_ids` 缺省为 `400002_0`，既接受逗号列表，也兼容参考项目的 JSON 字符串数组；顺序与重复项保留，最多 100 项。网易云固定使用带实时 v3 checkToken 的 XEAPI `/api/ad/get`，精确把类型数组序列化进 `type_ids` 字符串。对象或数组广告包装统一为稳定条目，逐项解析 `extJson.contextInfo.req_id`；无效或空的前一项不会遮蔽后续有效请求 ID，无法解析的 `extJson` 仍随原广告完整保留。匿名设备可能合法返回空目录，不伪造成错误或虚构请求 ID。

广告换听领取的 `creative_type/rights_gain_method` 均默认 2；曝光和点击时间都省略时使用同一次读取的当前 Unix 毫秒。参考 GET 的显式时间值原样保留为 JSON 字符串，统一 POST 同时接受整数毫秒和字符串，以免改变上游参考协议的类型分支。四个可选时长/方式、来源、权益扩展文本、任意 JSON `app_info` 和安装态都会进入 `reqParam` 内层 JSON；缺省字段不会伪造。省略或传空 `request_uid` 时，provider 先按同一 `platform/account/type_ids` 取广告目录；目录失败、无投放或无 `req_id` 时依照参考行为继续提交空 ID，并以扩展字段明确来源。网易云领取固定使用带 v3 checkToken 的 XEAPI `/api/ad/listening/rights/gain`；匿名真实请求当前返回业务码 2001，统一映射为 `authentication_required`，不会误报已领取。

私人 FM 与每日推荐目录分离。`backend` 缺省为 `classic`，也接受 `default/personal_fm`；网易云固定使用 WeAPI `/api/v1/radio/get` 且不提交伪分页参数。`backend=mode`（也接受 `personal_fm_mode`）使用 EAPI 同路径，完整保留可选 `mode/subMode/limit`，其中 `sub_mode` 也接受 `submode/subMode`。模式字符串只做长度和空白边界校验，不把平台将来增加的模式限制在本地枚举中。响应是当前队列快照：`total` 为本次返回数量，`has_more=false/next_offset=null/continuation_supported=false`；`limit` 只控制本次映射上限，不会伪造上游分页。2026-07-18 匿名真实联网已分别验证经典和模式后端均返回非空统一 `Track` 队列。

首页个性化和登录账户每日推荐共用按资源类型稳定的端点，但以显式 `source` 分支区分，不静默互换。`source` 缺省为 `daily`，也接受 `default`；`personalized` 兼容 `homepage/home/personalised`。网易云个性化新歌固定使用 WeAPI `/api/personalized/newsong`，提交 `type=recommend/limit/areaId`，其中 `area_id` 缺省为 0 且只允许用于该分支；个性化歌单固定使用 `/api/personalized/playlist`，精确提交 `limit/total=true/n=1000`。两者均为不支持 offset 的首页快照，非零 offset 会明确拒绝；推荐算法、文案、是否可反馈及完整包装保存在单项扩展，当前上游可能把歌单播放量返回为浮点 JSON，TuneWeave 会无损保留而不强制降格为整数。

首页视频推荐以 `kind/view` 保留三个不同上游能力：`mv/featured` 对应 WeAPI `/api/personalized/mv`，`exclusive/featured`（别名 `privatecontent/entry`）对应 `/api/personalized/privatecontent`，二者都是不可续页快照；`exclusive/catalog`（也接受 `view=list/all`）对应 `/api/v2/privatecontent/list`，精确提交 `offset/limit/total="true"` 并按真实 `more` 生成下一偏移。平台没有个性化 MV 分页目录，因此 `mv/catalog` 会明确拒绝，不会拿独家放送替代。条目统一为 `Video`，MV 艺人、封面、正时长、播放量、收藏态和独家放送时间按可用字段映射，入口与分页包装完整保留在扩展。

网易云独立 MV 目录精确覆盖 `mv_all/mv_first/mv_exclusive_rcmd`。`all` 把 `area=all|mainland_china|hong_kong_taiwan|western|japan|korea`、`type=all|official|original|live|netease`、`order=rising|hot|new` 序列化为参考 `tags` JSON 字符串；相应中文值也可直接输入。`latest` 只提交 `area/limit/total=true`，明确拒绝非零 offset 及类型/排序；`exclusive` 只提交 `offset/limit` 并拒绝所有虚构筛选。`count`、`hasMore/more` 分别驱动真实总数和续页；最新目录没有续页控制时固定 `next_offset=null/has_more=false/continuation_supported=false`，不会把一屏数据伪装成完整分页。空白备用描述/封面会继续读取有效字段，零时长表达为未知而非有效 0 毫秒。2026-07-22 三个统一目录均真实 HTTP 返回 200 和非空 `Video[]`。

网易云视频收藏统一按 `kind` 分派而不混用协议：MV 精确覆盖 `mv_sub`，PUT/DELETE 分别调用 WeAPI `/api/mv/sub|unsub` 并同时提交数值 `mvId` 和参考格式字符串 `mvIds=["..."]`；普通视频精确覆盖 `video_sub`，调用 `/api/cloudvideo/video/sub|unsub` 并提交不透明字符串 `id`。数值引用省略类型时按统一规则推断为 MV，非数值引用推断为普通视频，也可显式指定。`mv_sublist` 固定调用 WeAPI `/api/cloudvideo/allvideo/sublist`，提交 `limit/offset/total=true`，将上游混合返回的字符串 `vid`、创作者、封面、时长、播放量和来源 `type` 映射为已收藏 `Video[]`，完整单项及去除大数组后的分页响应分别保存在扩展。2026-07-22 持久化真实账户已验证列表读取，并分别完成 MV 未收藏→收藏→取消收藏及普通视频已收藏→取消收藏→恢复收藏的写入闭环，最终状态均与测试前一致。

网易云站内视频分类与标签分别精确覆盖 `video_category_list`、`video_group_list`：分类提交参考 `offset/total="true"/limit`，标签接口提交空对象且不伪造其不存在的 offset；上游即使不应用请求 limit 也返回完整目录，并以 `limit_applied=false` 明示。`catalog=timeline_all`、`timeline_recommended`、`group` 分别覆盖 `video_timeline_all`、`video_timeline_recommend`、`video_group`，完整保留参考固定字段 `groupId/need_preview_url/filterLives/withProgramInfo/needUrl/resolution`。时间线不提交虚构 limit，按 `hasmore` 与实际返回数推进下一 offset；外层算法包装和内层视频均不丢失。上游合法的 `datas=null` 分类响应规范化为空页，完全缺失或错误类型仍作为协议错误。2026-07-22 分类 9 项、标签 107 项、全部/推荐时间线各 8 项均真实返回，累计 63 次实际 group 请求均为 200 空页。

推荐节目的 `source=personalized` 固定使用 WeAPI `/api/personalized/djprogram`，外层推荐包装中的 `program` 映射为完整 `PodcastEpisode`；节目、所属播客和承载音频三种引用保持分离，并可直接复用节目取流链路。该接口不接受分页控制，因此只允许 `offset=0`，`limit` 是本地快照上限，分页扩展明确 `continuation_supported=false/limit_applied=false`。`source=category` 固定使用 WeAPI `/api/program/recommend/v1` 并精确提交 `cateId/limit/offset`；省略 `source` 但提供分类字段时会自动选择该分支，省略分类则完整复刻参考模块未提供 `type` 的调用。上游当前 `offset` 确实生效，但即使后续偏移仍返回不同节目也可能报告 `more=false`，因此 TuneWeave 如实保留 `more`、允许调用方显式偏移，却不会伪造 `next_offset`。2026-07-22 匿名真实 provider 与统一 HTTP 已验证分类 `2` 的偏移 0/2 分别返回不同的两期完整节目及可播放音频，两个响应均保留上游 `code=200/more=false`；同一次联网测试也覆盖既有个性化首页六分支，七个分支均返回非空类型化资源。

推荐反馈要求完整曲目引用，引用决定平台，`account` 选择该平台的持久账户别名。网易云固定使用 WeAPI `/api/v2/discovery/recommend/dislike`，精确提交 `resId`、`resType=4`、`sceneType=1`；未知平台、跨平台冲突及空 ID 会在请求前拒绝。匿名真实联网会把上游登录边界映射为 401 `authentication_required`，成功写入留到 Basic 末尾用持久化账户集中验收。

为兼容网易云参考项目，横幅端点也接受 `type=0|1|2|3`，依次对应 PC、Android、iPhone、iPad；响应始终使用统一字段与客户端名称。`catalog` 缺省为 `music`，也接受 `scope` 别名；`podcast`（别名 `dj`）使用平台播客横幅目录，网易云该目录没有客户端选择能力，因此只允许缺省的 PC 值并把目标 `60001` 映射为 `podcast_episode`。

广播电台目录同时接受参考项目的 `categoryId/regionId/lastId` 命名。网易云以 `last_id+score` 作为真实游标；两者可独立传入，另一项分别按 `0/-1` 补齐。参考接口类型虽公开 `offset`，但模块实现与真实上游都不应用它，因此 TuneWeave 仍接收并在分页扩展返回 `requested_offset` 与 `offset_applied=false`，不会把首屏伪装成偏移页。首屏还可能插入推荐电台，实际 `data` 数量可以大于请求 `limit`，TuneWeave 保留完整上游结果并以真实末项生成下一游标。

播客分类与直播广播分类保持不同实体和端点，避免把点播节目目录误当成地区电台。网易云 `kind=all` 固定使用空负载 WeAPI `/api/djradio/category/get`，`kind=non_hot`（兼容 `exclude_hot`）固定使用 `/api/djradio/category/excludehot`；`id` 统一为不透明字符串，图标依次从网页、尺寸和客户端专用字段选择，全部平台字段保存在单项扩展，整份上游响应保存在目录扩展。`/v1/podcasts/category-recommendations` 对应空负载 WeAPI `/api/djradio/home/category/recommend`，返回分组而非扁平播客列表：每个分组完整保留分类、三项推荐播客、算法/推荐文案和原始包装，不会只抽取分类而丢弃内容。`platform` 选择内容平台，`account` 只选择该平台的持久账户别名。2026-07-22 真实统一 HTTP 分别返回 19 个完整分类、13 个非热门分类及 12 个推荐分组，首组为分类 `3`“情感”并含 3 个完整播客。

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
| GET | `/v1/tracks/{ref}/lyrics` | `platform?` 不覆盖引用平台 | `Lyrics` |
| GET | `/v1/episodes/{ref}/lyrics` | `account?` | `PodcastEpisodeLyrics`；真实无歌词分支也返回可检查的成功数据 |
| GET | `/v1/episodes/{ref}/stream` | 与歌曲流相同的音质、后端、播放平台、回退、解灰和账户参数 | `PodcastEpisodeStream`；节目、原音频和最终解析资源身份分离 |
| GET | `/v1/episodes/{ref}/stream/redirect` | 同上 | 成功解析节目音频后返回 302，不向客户端暴露账户凭据 |
| GET | `/v1/tracks/{ref}/stream` | `quality?`、`variant?`、`bitrate?`、`immersive_type?`、`playback_platform?`、`fallback?`、`fallback_platforms?`、`unblock?`、`source?`、`account?` | `MediaStream` |
| GET | `/v1/tracks/streams` | `refs` 或 `ids`（兼容 `id`）、`platform?`、同上播放控制参数 | `StreamBatch`；逐项成功或失败，保留输入顺序与重复项 |
| POST | `/v1/tracks/streams` | JSON `{refs?|ids?, platform?, quality?, variant?, bitrate?, immersive_type?, playback_platform?, fallback?, fallback_platforms?, unblock?, source?, account?}` | `StreamBatch`；`refs/ids` 可为字符串或字符串数组 |
| GET | `/v1/tracks/{ref}/download` | `quality?`、`variant?`、`bitrate?`、`account?`；兼容 `level/backend/br` | `MediaDownload`；无可用 URL 仍是可检查的成功数据 |
| GET | `/v1/tracks/{ref}/download/redirect` | 同上 | 有专用下载 URL 时返回 302；否则尝试同音质播放 URL 后再返回 302 |
| POST | `/v1/resolve` | 完整解析请求，见下文 | `Stream` |
| GET | `/v1/videos/{ref}` | `kind/type=mv|video`、`account?` | `VideoDetail`，含统一视频信息和平台公布的清晰度 |
| GET | `/v1/videos/{ref}/stats` | `kind/type=mv|video`、`account?` | `VideoStats` |
| GET | `/v1/videos/{ref}/stream` | `kind/type=mv|video`、`resolution/res?`、`account?` | `VideoStream`；默认请求 1080，允许无可用 URL 的业务成功态 |
| GET | `/v1/videos/{ref}/stream/redirect` | 同上 | 有可用 URL 时返回 302，否则返回 404 |
| GET | `/v1/videos/{ref}/parts` | 分页；B 站 Basic 阶段接入 | `VideoPart[]` |

为兼容参考项目调用方，音频识别请求也接受 `audio_fp`/`audioFP` 作为 `fingerprint` 的别名、`duration` 作为 `duration_seconds` 的别名；响应只使用统一字段名。

音频流的统一音质为 `auto/low/standard/higher/high/lossless/hires/surround/spatial/dolby/master`。网易云兼容字段 `level` 是 `quality` 的别名，并完整接受 `standard/higher/exhigh/lossless/hires/jyeffect/sky/dolby/jymaster`，其中 `exhigh/jyeffect/sky/jymaster` 分别映射为 `high/surround/spatial/master`。`variant=default|legacy|modern` 选择 provider 推荐后端、旧版码率后端或新版等级后端；兼容字段 `backend` 接受 `v0/song_url` 与 `v1/song_url_v1` 等别名。网易云缺省使用现代 v1；`variant=legacy` 时 `bitrate`（兼容 `br`）按原始无符号 bit/s 精确提交，省略时再由 `quality` 映射默认码率；现代后端按参考行为忽略 `br`。

`immersive_type=c51|ste|aac` 选择沉浸声音频类型，并兼容网易云字段名 `immerse_type`/`immerseType`。省略时网易云 `spatial/sky` 使用上游默认 `c51`；显式选择仅在现代 `song_url_v1` 且音质为 `spatial/sky` 时写入 `immerseType`，其他音质和旧版协议不会误发该字段。该控制与音质、账户和跨平台路由一同贯穿单曲、批量、播客、Uni Playlist 项播放及 `POST /v1/resolve`；不支持的值返回 `invalid_request`，不会静默降级为另一种沉浸声类型。

`available_qualities` 始终按上述能力层级从低到高返回，不依赖平台响应数组的偶然顺序。网易云歌曲元数据中的 192 kbps `m` 档映射为 `higher`，320 kbps `h` 档才映射为 `high`；最高码率字段为零时不会遮住有效的兼容码率。当逐字 YRC 与逐行 LRC 同时存在时，`Lyrics.format` 标记能力更高的 `yrc`，但 `plain` 与 `word_synced` 两份内容都会保留；歌词贡献者的无效旧 ID 也不会遮住有效 `userId`。

批量 GET 的 `refs` 是逗号分隔完整资源引用；`ids/id` 是平台内 ID，`platform` 省略时使用服务默认平台，且只能与 `ids` 一起使用。POST 的 `refs/ids` 既可为单个字符串或逗号字符串，也可为字符串数组。两种输入都不折叠重复项；混合平台引用按来源 provider 分组调用原生批量能力，再严格还原原顺序。`StreamBatch.outcomes` 为每个输入返回独立 `status/stream/error_code/error/extensions`，单项不可用不会把整个 HTTP 请求变成失败；各 provider 的完整批量响应位于 `extensions.provider_batches`。

`unblock=true` 是参考 `/song/url/v1?unblock=true` 与 `/song/url/match` 的统一兼容预设，不另建第二套解灰逻辑。指定 `source=qq|kugou|kuwo|migu|...` 时先在该平台严格匹配，再回到原平台；省略时依次尝试 QQ、酷狗、酷我、咪咕和原平台。该模式始终保留原平台兜底，所以兼容输入中的 `fallback=false` 不会关闭兜底；为避免两套路由规则冲突，不能同时提交 `playback_platform` 或 `fallback_platforms`。`account` 绑定首个目标来源，所有尝试及失败原因都返回在 `attempts`。

Uni Playlist 项流复用同一音质与路由参数，并额外以 `accounts` 为每个平台选择独立账户。查询值可写成 `netease=main,qq=green-diamond`，也可传 URL 编码后的 JSON 对象；同一平台不能同时由兼容 `account` 和 `accounts` 指定。歌曲直接进入严格解析器；播客先取得节目承载音频；MV/视频按同一平台顺序选择原生视频流或严格匹配到其他平台的音频；广播刷新原平台直播地址或动态队列，不会用标题把直播频道错配成歌曲。`resolution=1..4320` 只适用于 MV/视频，请求档位和上游实际档位分开保留。响应中的 `item_id/source_ref/kind` 始终指向歌单项目，`stream.resolved_track/resolved_platform/attempts` 表达本次实际媒体来源。

节目流复用上述同一套解析器，不单独维护低能力播放分支。provider 先把节目 ID 解析为原音频 `Track`，随后才应用 `playback_platform/fallback/unblock/source/account`；因此网易云节目可以在原音频权益不足时严格匹配到 QQ 等平台，但节目引用本身不会被替换成歌曲引用。网易云 Basic 已真实验证公开节目 JSON 取流和 302；跨平台成功命中仍随 QQ Basic 接入后补验。

下载端点复用同一套音质、后端、精确码率和账户参数，但不会把播放 URL 冒充专用下载 URL。`MediaDownload.available` 与可空 `url` 明确表达下载能力，平台返回的实际音质、码率、大小、时长、业务码、费用和完整原文均保留；空白编码不会遮住有效容器格式，零响应时长不会遮住歌曲元数据时长，播放流遵循同一规则。例如网易云新版下载顶层可能为 `code=200`，但某档条目是 `code=-110/url=null`，此时 JSON 端点仍以 HTTP 200 返回 `available=false`。`/download/redirect` 对应参考 `/song/url/v1/302` 的两段逻辑：先请求下载地址，缺失时再请求播放地址；只有取得非空 URL 才发出 302 `Location`，客户端凭据和上游 Cookie不会进入重定向响应。

网易云的 `/v1/videos/{ref}`、`/stats` 和 `/stream` 分别精确覆盖 `mv_detail/video_detail`、`mv_detail_info/video_detail_info` 与 `mv_url/video_url`。MV 详情、统计和平台公布的 240/480/720/1080 四档播放地址已经完成真实 HTTP 验收；站内视频 ID 是不透明字符串，失效资源的 404 以及 `code=200` 但空 URL 列表都保持原始业务语义。2026-07-22 又以账户收藏中的当前有效普通视频真实验证详情、4 档资源、统计、480p 非空播放 URL 与统一 302 重定向。

`POST /v1/resolve` 同时接受已有引用或纯元数据：

```json
{
  "track": {
    "ref": "netease:123456"
  },
  "quality": "lossless",
  "playback_platforms": ["qq", "netease", "kugou"],
  "accounts": {
    "qq": "green-diamond",
    "netease": "default"
  },
  "strict_match": true
}
```

也可以把 `track` 写成 `{ "name": "...", "artists": ["..."], "album": "...", "duration_ms": 0, "isrc": null }`。没有引用时不会产生 `origin_track`，但仍返回最终 `resolved_track`。

### 登录与账户

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| GET | `/v1/auth/country-codes` | `platform?`、`account?` | `CountryCallingCodeGroup[]`；登录可选国家/地区及电话区号目录 |
| POST | `/v1/auth/qr` | `{platform, account?, login_type?}` | 二维码事务 ID、二维码 URL/图片、过期时间 |
| GET | `/v1/auth/qr/{transaction_id}` | 无 | `waiting/scanned/confirmed/expired/failed`；成功时保存登录态 |
| POST | `/v1/auth/password` | `{platform, account?, principal_type, principal, password}` | 登录状态和脱敏账户摘要 |
| POST | `/v1/auth/principals/status` | `{platform, account?, principal_type?, principal, country_code?}` | `AuthPrincipalStatus`；查询主体是否已注册，不创建登录态 |
| POST | `/v1/auth/challenges` | `{platform, account?, method?, principal, country_code?}` | 短信等挑战事务 |
| POST | `/v1/auth/challenges/validate` | `{platform, account?, method?, principal, code, country_code?}` | `AuthChallengeValidation`；仅校验挑战码，不创建登录态 |
| POST | `/v1/auth/challenges/{transaction_id}/verify` | `{code}` | 验证状态；成功时保存登录态；网易云兼容 `{captcha}` |
| POST | `/v1/auth/session/refresh` | `{platform, account?}` | 刷新状态和脱敏账户摘要 |
| GET | `/v1/auth/session` | `platform`、`account?` | 当前会话状态，不返回凭据 |
| DELETE | `/v1/auth/session` | `platform`、`account?` | 删除结果 |
| GET | `/v1/account` | `platform`、`account?` | 脱敏账户资料与权益摘要 |
| GET | `/v1/account/playlists` | `platform`、`account?`、分页 | `Playlist[]` |
| GET | `/v1/account/library/albums` | `platform`、`account?`、分页 | 已收藏的 `Album[]`；收藏时间保留在条目扩展，付费专辑计数等保留在分页扩展 |
| GET | `/v1/account/library/radio-stations` | `platform`、`account?`、分页；`catalog=broadcast|styled`、`sources?` | 已收藏的 `RadioStation[]`；缺省为普通广播，`styled`/`difm` 返回 DiFM 风格频道收藏 |
| GET | `/v1/account/library/podcasts` | `platform`、`account?`、分页 | 已订阅的 `Podcast[]`；列表身份明确使 `subscribed=true`，完整平台条目与分页响应保留在扩展 |
| GET | `/v1/account/following/artists` | `platform`、`account?`、分页 | 已关注的 `Artist[]`；关注时间和平台原始资料保留在条目扩展 |
| GET | `/v1/account/following/artists/new-videos` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Video[]`；`before` 与 `next_before_ms` 均为毫秒时间戳 |
| GET | `/v1/account/following/artists/new-tracks` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Track[]`；上游新曲总数保留为分页 `total` |
| GET | `/v1/account/following/artists/new-works` | `platform`、`account?`、`limit?`、`before?`、`source_type?`、`first_request?` | `ArtistWorkUpdate[]`；歌曲/MV 混合更新流，未知来源保留原文 |
| GET | `/v1/account/following/artists/new-tracks/play-all` | `platform`、`account?` | 最近至多 50 首新 `Track[]`；固定快照，不伪装成可翻页目录 |
| GET | `/v1/account/favorites/tracks` | `platform`、`account?`、分页 | `Track[]` |
| GET | `/v1/account/history` | `platform`、`account?`、`period=all_time|week`、分页 | `PlaybackHistoryEntry[]`，含 `track`、`play_count`、`score`、`last_played_at` |
| GET | `/v1/account/history/podcast-episodes` | `platform?`、`account?`、`limit?`、`offset?=0` | `PodcastEpisodePlaybackHistoryEntry[]`；完整分离节目、承载音频、播放时间与终端信息 |
| GET | `/v1/account/cloud/tracks` | `platform?`、`account?`、`limit?`、`offset?` | `CloudTrack[]` 分页及云盘容量统计 |
| GET / POST | `/v1/account/cloud/tracks/details` | 查询或 JSON `refs?|ids?`、`platform?`、`account?` | 保持输入顺序和重复项的 `CloudTrack[]` |
| GET | `/v1/account/cloud/tracks/{ref}/download` | `account?` | 云盘源文件的统一 `Stream`；不可用时返回明确业务错误 |
| GET | `/v1/account/cloud/tracks/{ref}/download/redirect` | `account?` | 302 到云盘源文件 URL；源文件 URL 缺失时回退到同平台同账户普通取流 |
| GET | `/v1/account/cloud/lyrics` | `platform?`、`account?`、`user_id`、`track_id` | 云盘文件标签中的统一 `Lyrics` |

`principal_type` 至少允许平台实际支持的 `email`、`phone` 或平台账号类型；密码默认按明文接收并立即在适配器内完成平台要求的摘要，也可用 `password_format: "md5"` 明确提交已有摘要。`method` 至少允许 `sms`，并可由平台扩展。上游存在多种登录方式时必须全部接入，不能只保留二维码这一条流程。

网易云播客订阅列表固定使用 WeAPI `/api/djradio/get/subed`，提交 `limit/offset/total=true`，并将 `count/hasMore`（兼容 `more`）映射为统一分页。列表本身比条目内可能陈旧的 `subed=false` 更明确，因此返回项稳定标记 `subscribed=true`，不会让低层默认值遮住账户资料库语义。订阅与取消订阅分别使用 `/api/djradio/sub` 和 `/api/djradio/unsub`，统一为同一资源路径的 PUT/DELETE，并由 `account` 选择隔离的持久登录态。

网易云最近声音固定使用 WeAPI `/api/play-record/voice/list` 并提交 `limit`（默认 100、范围 1–100）。平台包装中的 `data.pubDJProgramData` 映射为完整节目，节目引用、所属播客与 `mainTrackId/mainSong` 对应的承载音频引用不会混淆；`playTime` 转换为 RFC 3339 的 `played_at`，`os/multiTerminalInfo` 映射为独立终端对象，完整记录和响应仍保存在扩展。上游没有 offset 或续页控制，所以只接受 `offset=0`，即使 `total` 大于本次条目数也保持 `next_offset=null/has_more=false/continuation_supported=false`。2026-07-22 使用隔离持久账户通过真实统一 HTTP 返回两条记录，节目 `netease:2059302984` 与音频 `netease:1342589772` 保持分离，上游 `code=200`。

网易云 DiFM 频道收藏复用 `/v1/account/library/radio-stations`：`catalog=styled`（别名 `style/difm`）选择风格频道目录，`sources` 默认 `0`，也接受 `[0,1,2]`、`0,1,2` 或单值，仅允许电子/古典/爵士 `0/1/2`。上游返回的是固定收藏快照，没有分页控制，因此只接受 `offset=0`，`limit` 仅保留调用意图并以 `limit_applied=false/continuation_supported=false` 明示平台没有应用；频道引用保持 `netease:difm:{source}:{channelId}`。同一引用可用于账户资料库的 PUT/DELETE 订阅端点，`account` 始终选择隔离登录态。2026-07-22 持久化真实账户通过统一 HTTP 请求三源目录，上游返回 `code=200` 和空收藏快照；写入路径未用于改变该账户现有收藏。

国家区号目录允许省略 `platform` 并使用服务默认平台；`account` 只选择该平台的请求会话。网易云固定以 EAPI 调用 `/api/lbs/countries/v1`，公开目录不要求登录；统一结果保留上游分组顺序、电话区号、地区代码和中英文名称。不存在的非默认账户别名仍按账户隔离规则返回认证错误，不会静默退回默认会话。

`/v1/auth/principals/status` 只查询注册状态，不发送验证码、不登录。`principal_type` 省略时默认 `phone`；网易云兼容参考字段 `phone/countrycode`，分别作为 `principal/country_code` 的别名，也接受 `countryCode`，手机号和区号均可为字符串或数字，区号缺省或为空时使用 `86`。统一结果用 `exists` 表示是否注册，并保留 `has_password`、平台已脱敏的 `display_name`、`avatar_url` 和 `platform_code`；完整上游响应位于 `extensions.response`，原始手机号不进入稳定字段或日志。

`/v1/auth/challenges/validate` 与事务验证端点语义不同：它只调用平台的验证码校验能力，不登录、不保存 Cookie，也不要求先发送验证码。`method` 省略时默认为 `sms`；网易云还兼容参考字段 `phone/captcha/ctcode`，分别作为 `principal/code/country_code` 的别名，手机号和区号都接受字符串或数字，区号缺省或为空时使用 `86`。`valid=false` 是正常业务结果，仍以 HTTP 200 返回，并通过 `platform_code`、`message` 和 `extensions.response` 保留平台信息；空白上游 `message` 不会遮蔽有效 `msg`。手机号和验证码不会回显。需要验证码登录时仍使用 `/v1/auth/challenges` 创建不透明事务，再调用 `/{transaction_id}/verify`。

验证码登录事务同样允许省略 `method`（默认 `sms`），并兼容 `phone/ctcode` 与后续验证请求中的 `captcha`；这些标量既可为字符串也可为数字。发送端点只发送一次，不会自动重试；事务验证成功后才保存对应 `platform/account` 的登录态。

二维码与验证码端点返回的 `transaction_id` 是 TuneWeave 生成的随机不透明标识，不是上游二维码 key、手机号或 token。敏感字段仅在请求生命周期或短期事务仓库内使用，保存后的平台凭据只通过账户别名引用；密码、验证码、Cookie 与上游事务标识不会写入普通响应。

`POST /v1/auth/qr` 的 `image_data_url` 是可直接显示的自包含图片；网易云当前返回 `data:image/svg+xml;base64,...`，二维码编码在进程内完成，不会把登录 URL 发送给第三方图片服务。二维码 key 和业务码按首个可解析的非空候选映射，空顶层兼容字段不会遮住 `data` 中的有效值。调用方也可使用同一响应中的 `url` 自行渲染。

文件账户后端默认位于 `.local/data/accounts`，可用 `TUNEWEAVE_DATA_DIR` 改变其父目录。账号别名在路径中使用 UTF-8 十六进制编码，不能构造路径穿越；每次更新先在同目录写入私有临时文件并同步，再以原子重命名发布新代际，启动只读取最新完整代际。Unix 权限为目录 `0700`、文件 `0600`，Windows 继承数据目录 ACL。文件内的平台会话凭据目前不做静态加密，因此运维必须保护该目录且不得同步或提交；它从不进入 Debug、普通错误、HTTP 响应或日志。`DELETE /v1/auth/session` 会删除对应 `platform/account` 的本地持久凭据；即使上游退出请求不可达，本地凭据仍会清除，错误详情以 `local_session_removed` 明确结果。

### Uni Playlist

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/uni/playlists` | JSON `{name, description?}` | 新建的空 `UniPlaylist` |
| POST | `/v1/uni/playlists/imports` | JSON `{name?, description?, sources:[{ref?, platform?, type?, id?, account?}]}` | `UniPlaylistImportResult`，完整分页后原子创建的多来源合并歌单 |
| GET | `/v1/uni/playlists/{ref}` | 完整 `uni:<opaque-id>` 引用 | 持久化的 `UniPlaylist` 元数据 |
| GET | `/v1/uni/playlists/{ref}/items` | `limit?`、`offset?` | 分页 `UniPlaylistItem[]`，严格保留位置和重复来源项 |
| POST | `/v1/uni/playlists/{ref}/items` | JSON `{items:[{ref,kind}], accounts?}` | `UniPlaylistItemAddResult`，原子追加类型化混合项目 |
| DELETE | `/v1/uni/playlists/{ref}/items/{item_id}` | 稳定的单次出现项目 ID | `UniPlaylistItemDeleteResult`，仅删除该项目并重编号后续位置 |
| PATCH | `/v1/uni/playlists/{ref}/items/order` | JSON `{item_ids:[...]}` | `UniPlaylistItemOrderResult`，原子提交完整显式顺序 |

`UniPlaylist` 使用独立 `uni:` 命名空间，不归属于网易云等外部 provider。稳定字段包含同值的 `ref/platform/id` 身份、名称、描述、`item_count` 以及毫秒级 `created_at_ms/updated_at_ms`；新建歌单的项目数为 0。名称去除首尾空白后必须为 1–200 字节，描述去除首尾空白后最多 4000 字节，未知 JSON 或查询字段会被拒绝。`GET` 必须提交完整引用，错误平台、畸形 ID 和不存在的歌单分别返回统一错误包络。

生产服务把数据保存到 `TUNEWEAVE_DATA_DIR/uni-playlists.json`，与 `accounts` 凭据目录分离。文件后端在内存维护已验证快照，创建时在同目录写入并同步临时文件后发布；Unix 使用原子替换，Windows 使用可在下次启动恢复的同目录备份切换。未知数据库版本或畸形记录会阻止启动而不会被静默覆盖。该文件只保存歌单结构与后续的必要元数据快照，不保存媒体字节或平台凭据。

混合项目 `kind` 当前完整区分 `track`、`mv`、`video`、`podcast_episode` 和 `radio_station`。添加端点逐项按 `ref` 的来源平台调用已注册 provider 获取真实元数据，而不是信任调用方伪造标题：歌曲快照包含标题、艺人、专辑、时长、ISRC、封面、版本标签和播放能力摘要；MV/视频包含创作者、时长、封面、平台视频类型与发布时间；播客节目包含主播、时长、封面、所属播客、独立音频引用、发布时间和期号；广播电台包含名称、封面、分类、地区、当前节目及是否具有直接流。只保存这些播放匹配所需的紧凑字段，不复制整份上游原文或易过期的流地址。

`accounts` 是按平台键控的账户别名对象，例如 `{ "netease": "default", "qq": "green-diamond" }`；每个来源只使用自己的别名，不能提交 `uni` 账户或本批次未出现的平台。一次可追加 1–100 项，所有资源完成解析后才执行一次存储发布；失败不会留下半批数据。来源引用可以重复，存储不会静默去重：每次出现都会生成独立 `item_...` ID 和连续零基 `position`，后续删除/重排按项目 ID 工作。读取默认 `limit=50/offset=0`、范围 1–100，分页 `total/next_offset/has_more` 基于实际项目序列。2026-07-22 真实二进制 HTTP 将网易云同一歌曲两次、一个 MV 和一期播客按 `track,track,mv,podcast_episode` 写入同一 Uni Playlist，位置为 `0,1,2,3`、重复引用对应不同项目 ID，真实快照及独立节目音频引用均保留，单文件数据库为 2309 字节。

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

一次导入接受 1–100 个来源，按请求中的来源顺序逐一完整读取所有分页，再按各来源内部位置生成新的稳定项目 ID；来源和条目都允许重复，不进行隐式去重。任何来源、分页或身份检查失败时目标歌单完全不创建；全部读取成功后，歌单级 `extensions.import_sources` 来源摘要、条目级来源索引/引用/类型和所有项目通过一次存储发布原子创建。未指定名称时使用来源名称按 `A + B` 派生并安全限制为 200 UTF-8 字节；单来源未指定描述时沿用其描述。2026-07-22 真实 release 二进制在无账户情况下把网易云“热歌榜”200 项与“飙升榜”100 项完整合并为 300 项，重启后来源摘要、来源边界及总数保持一致，单文件数据库为 188431 字节。

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
| PUT | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 收藏结果 |
| DELETE | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 取消收藏结果 |

创建歌单时 `visibility=public|private` 与参考 `privacy=0|10` 等价，`kind=normal|video|shared` 与参考 `type=NORMAL|VIDEO|SHARED` 等价；同一语义的统一字段和参考字段不得同时提交。元数据更新的 `variant=default|batch|individual` 分别表示自动选择、参考批量模块和独立字段模块；批量分支必须同时包含名称、描述和标签。标签既可用字符串数组，也可用参考分号字符串，空数组或空字符串表示清除。

歌单写入的 `refs` 是完整 `platform:id`，`ids` 是由路径或显式 `platform` 绑定的平台 ID；两者均接受单值、数组和逗号分隔字符串，但不能同时出现，输入顺序和重复项原样保留。批量删除和账户歌单排序不能混合平台。`/tracks` 只操作普通歌曲；`/videos` 只操作视频项目；`/items` 以 `kind=track|video` 选择，兼容参考 `type=0|3`。网易云创建结果会跳过零 ID，项目写入与排序结果会跳过空快照 ID，再采用后续有效兼容字段；`playlist_track_add/delete` 实际是 VIDEO 歌单的 `type=3` 项目接口，不会被错误复用为普通歌曲增删。

当前直接写入平台歌单要求资源已经能被目标 provider 接受；网易云因此要求项目引用属于网易云。Uni Playlist 与后续跨平台导入层在目标平台和歌曲来源平台不同时，必须先执行严格匹配；低于阈值时返回 `match_rejected`，不得把同名但不同版本的歌曲写入目标歌单。

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
| GET/POST | `/v1/extensions/netease/check-token` | `version?=v2|v3`（默认 v3）、`refresh?`；读取缓存或注册/刷新网易云易盾 anti-cheat token |
| GET/POST | `/v1/extensions/netease/register/checktoken/v2` | 固定读取或刷新 v2 token；另有固定 v3 路由，旧 `/register/checktoken` 别名默认 v3 |
| POST | `/v1/extensions/netease/api` | 在固定网易云域名上调用指定 `/api/...` 路径，支持 `eapi/weapi/api/linuxapi/xeapi` |
| GET | `/v1/extensions/netease/batch` | 以参考项目的查询参数形式批量调用网易云 `/api/...` 路径 |
| POST | `/v1/extensions/netease/batch` | 以 JSON 对象批量调用网易云 `/api/...` 路径 |
| GET | `/v1/extensions/netease/partner/tasks` | 查询音乐合伙人当日任务与待评作品 |
| POST | `/v1/extensions/netease/partner/run` | 按服务端策略执行合伙人任务并返回逐账户报告 |

网易云日历接受统一参数 `start_time`、`end_time`，并兼容参考项目的 `startTime`、`endTime`；值必须是无符号 Unix 毫秒时间戳。为完整保留参考实现的运行时行为，任一时间参数省略时都会使用本次请求的当前毫秒时间，两个参数也允许同时省略。`account` 选择服务端保存的网易云登录态。端点固定使用 WeAPI 调用 `/api/mcalendar/detail`，成功时完整上游日历 JSON 位于统一包络的 `data` 中。

网易云匿名身份由服务端生成、保存和复用，不属于登录账户，也不会覆盖任一 `account` 别名。首次 GET 或 `refresh=true` 会生成参考格式的 52 位十六进制设备 ID，按客户端 DLL 的 XOR + MD5 + Base64 规则构造用户名，并通过 XEAPI `/api/register/anonimous` 注册；POST 始终强制刷新。成功结果包含 `device_id/cookie/registered/refreshed/extensions`，其中 `cookie` 为兼容参考响应而返回，但不会进入 Debug 或普通日志，也不能由调用方反向注入统一请求。设备 ID 与 `MUSIC_A` 作为一份身份原子写入私有数据目录，重启后继续用于默认公开请求；显式登录账户始终优先且保持隔离。2026-07-18 实测 TuneWeave 与当前参考实现均收到上游业务码 400 且无 Cookie，因此代码完成但不伪造注册成功，待上游恢复后补成功态验收。

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

为避免把通用入口变成凭据注入或 SSRF 接口，请求 `uri` 只能是非空 `/api/...` 路径，目标域名由服务端配置且不能由调用者覆盖；请求体拒绝 `cookie`、`domain`、`headers`、`proxy`、`ua` 等传输覆盖字段，`data.cookie` 也会被拒绝。登录态只能通过 `account` 选择服务端保存的账户别名。XEAPI 的公钥注册、X25519 会话密钥、anti-cheat token 请求头与加密响应解包均由适配器内部完成，不接受调用方注入密钥或 token。

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

每个批量键都会独立校验为固定网易云域名下的非空 `/api/...` 路径；空批次、重复键以及原始 Cookie、域名、代理、请求头、UA、伪造 IP、客户端超时或检查令牌覆盖都会被拒绝。账户凭据只能通过 `account` 别名选择，`e_r=true` 的响应解密由适配器内部完成。

## 跨平台回退流程

1. 从 `origin_track` 读取标准化标题、歌手、专辑、时长和 ISRC。
2. 按 `playback_platforms` 尝试；目标平台与来源平台不同则先搜索候选。
3. 计算匹配分数：ISRC、规范化标题、主要歌手、专辑、时长依次参与；伴奏、Live、翻唱、Remix、纯音乐等版本标签单独惩罚。
4. 严格模式低于阈值时拒绝候选，不因“同名”直接换源。
5. 使用该平台指定账户解析媒体地址；无 URL、只有不允许的试听、权益不足、地区限制或上游错误时进入下一平台。
6. 成功响应同时返回来源引用、命中引用、分数和所有尝试轨迹。

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
