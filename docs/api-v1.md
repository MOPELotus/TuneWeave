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
  "available_qualities": ["standard", "high", "lossless"],
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

`Video` 同时承载音乐平台 MV、站内视频和 B 站视频信息，创作者不被强制假设为音乐歌手。歌手视频目录以 `type=mv|all` 选择范围；`mv` 使用偏移分页，支持游标的平台通过 `cursor` 与分页扩展返回下一游标。平台状态、备用封面和内部资源类型保留在条目扩展。

已关注歌手的新视频与新曲时间线分别返回 `Video[]` 和 `Track[]`，但都属于账户资源。它们以毫秒时间戳 `before` 翻页，并在 `meta.pagination.extensions.next_before_ms` 返回下一页起点；`account` 只选择登录态，不改变内容平台。

混合作品流返回 `ArtistWorkUpdate[]`：`kind=track|video|unknown` 明确资源类型，已识别内容分别进入 `tracks` 或 `videos`，`source_type`、作品标题、歌手、封面和发布时间使用稳定字段。尚未识别的平台来源仍返回 `kind=unknown`，完整负载保留在 `extensions.artist_work`，不会静默丢弃。

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
  "available_qualities": ["standard", "high", "lossless", "hires"],
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

会员摘要只把平台明确给出的值放入稳定字段。公开资料若只有等级、年费次数和图标，则 `active/expires_at` 保持 `null`，不会根据等级猜测当前是否仍在有效期。查询当前账户而上游未返回用户 ID 时，`user_ref` 也允许为 `null`；平台动态图标、会员种类和完整响应保留在扩展中。

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

推广横幅的稳定目标类型为 `track`、`album`、`artist`、`playlist`、`video`、`web`、`unknown`。网页活动通常没有资源 ID，因此 `target_ref=null`，仍保留 `url`；未知平台类型不会被猜成歌曲。曝光/点击监测、颜色、广告来源、内嵌歌曲和平台追踪字段完整保留在 `extensions.banner`。

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
| GET | `/v1/search` | `q`（也接受 `keywords`）、`type?`、`variant?`、`platform?`、`account?`、分页 | 带 `type/data` 判别字段的统一 `SearchItem[]` |
| GET | `/v1/search/default` | `platform?`、`account?` | `SearchDefaultKeyword`；实际查询词、展示文案、搜索类型与可选图片 |
| GET | `/v1/search/trending` | `platform?`、`account?`、`detail=brief|full` | `SearchTrendingList`；有序热搜关键词及可用的分数、说明和图标 |
| GET | `/v1/search/suggestions` | `q`（也接受 `keywords/keyword`）、`client=web|mobile|pc`、`platform?`、`account?` | `SearchSuggestionList`；关键词建议、可选统一资源及独立推荐项 |
| GET | `/v1/search/multimatch` | `q`（也接受 `keywords/keyword`）、`kind?`（也接受 `type`）、`platform?`、`account?` | `SearchMultiMatch`；按平台顺序分组的跨类型高置信匹配资源 |
| GET | `/v1/search/match` | 参考查询 `title/album/artist/duration/md5`，另支持 `duration_ms/duration_seconds`、`platform?`、`account?` | `LocalTrackMatchResult`；兼容参考项目调用形态 |
| POST | `/v1/search/match` | JSON `{title?, album?, artist?, duration_ms? | duration_seconds? | duration?, md5, platform?, account?}` | `LocalTrackMatchResult`；统一结构化调用形态 |
| GET | `/v1/banners` | `platform?`、`account?`、`client=pc|android|iphone|ipad` | `Banner[]`；省略客户端时使用 PC |
| GET | `/v1/radio/taxonomy` | `platform?`、`account?` | `RadioTaxonomy`；广播/播客目录可用的分类与地区 |
| GET | `/v1/radio/stations` | `platform?`、`account?`、`category_id?`、`region_id?`、`limit?`、`last_id?`、`score?`、`offset?` | `RadioStation[]`；游标下一页信息位于分页扩展 `next_cursor={id,score}` |
| GET | `/v1/radio/stations/{ref}` | `account?` | `RadioStation`；当前节目与直播音频地址按上游实时响应返回，未提供的收藏态保持 `null` |
| GET | `/v1/tracks/{ref}` | `account?` | `Track` |
| GET | `/v1/tracks/{ref}/availability` | `account?`、`bitrate?`（默认 999000，也接受 `br`） | `TrackAvailability`；不可播仍返回成功包络与 `playable=false` |
| GET | `/v1/albums` | `platform?`、`account?`、`catalog=new|newest`、`area?`、分页 | `Album[]` |
| GET | `/v1/albums/{ref}` | `account?` | `Album` |
| GET | `/v1/albums/{ref}/tracks` | 分页、`account?` | `Track[]` |
| GET | `/v1/albums/{ref}/track-entitlements` | 分页、`account?` | `TrackEntitlement[]` |
| GET | `/v1/albums/{ref}/stats` | `account?` | `AlbumStats` |
| GET | `/v1/digital-albums` | `platform?`、`account?`、`catalog=latest|style`、`area?`、`type?`、分页 | `DigitalAlbum[]`；上游不返回可靠总数时 `total=null` |
| GET | `/v1/digital-albums/{ref}` | `account?` | `DigitalAlbum` |
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
| GET | `/v1/playlists/{ref}` | `account?` | `Playlist` |
| GET | `/v1/playlists/{ref}/tracks` | 分页、`account?` | `Track[]`；B 站合集/收藏夹视频按可播放音频内容归一并保留 `video_ref` |
| GET | `/v1/resources/{type}/{ref}/comments` | `account?`、`view?`、`sort?`、评论分页参数 | `target/comments/hot_comments/top_comments/current_comment/extensions`；统一评论目录，分页位于 `meta.pagination` |
| GET | `/v1/resources/{type}/comments/stats` | `platform?`、`ids?`、`account?` | `CommentThreadStatsBatch`；同类型资源的批量评论、分享、点赞及最新条目统计 |
| GET | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `target_user_ref`、`account?`、`limit/page/cursor/id_cursor?` | `target/comment_id/target_user_ref/kind/reactions/current_comment/extensions`；评论反应用户目录 |
| PUT | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；启用评论反应 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`；停用评论反应 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/reports` | 查询参数 `account?`；JSON `{reason}` | `CommentReportResult`；举报评论 |
| GET | `/v1/users/{ref}/favorites/tracks` | 分页、`account?` | 指定用户公开引用下的 `Track[]`；需要平台登录态时由 `account` 选择 |
| GET | `/v1/users/{ref}/membership` | `account?` | 指定用户的公开 `MembershipSummary`；引用决定平台 |
| GET | `/v1/users/{ref}/history` | `period=all_time|week`、分页、`account?` | 指定用户的 `PlaybackHistoryEntry[]` |
| GET | `/v1/charts` | `platform?` | `Playlist[]`，其中榜单仍用歌单模型表示 |
| GET | `/v1/charts/{ref}/tracks` | 分页 | `Track[]` |
| GET | `/v1/recommendations/tracks` | `platform?`、`account?`、`refresh?`、分页 | `Track[]`；推荐理由保存在 `extensions.recommendation` |
| GET | `/v1/recommendations/playlists` | `platform?`、`account?`、分页 | `Playlist[]` |
| GET | `/v1/account/membership` | `platform?`、`account?` | 当前登录账户的 `MembershipSummary`；缺少登录态时返回 401 |

搜索类型缺省为 `track`，既接受统一名称，也接受网易云参考数字：`track|song|1`、`album|10`、`artist|100`、`playlist|1000`、`user|1002`、`mv|1004`、`lyric|lyrics|1006`、`radio_station|radio|dj|1009`、`video|1014`、`mixed|complex|1018`、`voice|2000`。`variant` 支持 `default|legacy|cloud`，也兼容 `backend` 字段以及 `search/cloudsearch/auto` 值；缺省时由 provider 使用推荐后端。网易云 `legacy` 精确对应参考 `/search`：普通类型使用 `/api/search/get`，声音使用独立 `/api/search/voice/get`；`cloud` 对应 `/cloudsearch`。每一项统一序列化为 `{type,data}`；歌曲、专辑、歌手、歌单、用户、MV/视频及广播电台使用对应统一实体，其中 MV 与视频均为 `video`，歌词搜索以 `track` 返回并把命中的平台歌词原文保存在曲目扩展。综合搜索、声音或上游出现尚无稳定公共结构的条目使用 `opaque`，保留平台、搜索类型、可提取的 ID/标题及完整原文。实际后端和上游路径位于分页扩展 `variant/request_path`，完整上游响应也保存在分页扩展；上游若不应用请求 `limit`，TuneWeave 返回真实条目并显式写入 `limit_applied=false`，不会截断后伪装成已应用分页。

默认搜索词与搜索结果分离：`keyword` 是应提交给搜索端点的真实词，`display_text` 是可直接展示的文案，`kind` 仅在平台类型可映射时返回，图片允许为空。网易云固定使用 EAPI `/api/search/defaultkeyword/get`；算法、样式词和业务意图等动态字段完整保留在 `extensions.response`，调用方不应解析它们来替代稳定字段。

热搜目录按 `rank` 从 1 开始排序，`keyword` 必填，说明、分数、图标类型、图标 URL 和目标 URL 均按平台实际返回可空。`detail` 缺省为 `full`，也接受 `brief`，并兼容 `mode` 查询名及 `simple/detail/detailed` 值。网易云简略榜固定使用 EAPI `/api/search/hot` 和 `type=1111`，详细榜固定使用 WeAPI `/api/hotsearchlist/get`；两套响应不会互相补造缺失字段，完整原文位于列表与条目扩展。

搜索建议的 `client` 缺省为 `web`。统一条目始终给出可直接重新搜索的 `keyword`，可选 `kind/display_text/icon_url`；web 建议中的歌曲、专辑、歌手、歌单等实际资源同时以统一 `SearchItem` 放入 `resource`，mobile/PC 纯关键词不会伪造资源。PC 的 `recs` 与普通 `suggests` 分别位于 `recommendations/suggestions`。网易云 web/mobile 分别固定使用 WeAPI `/api/search/suggest/web`、`/api/search/suggest/keyword`，PC 固定使用 EAPI `/api/search/pc/suggest/keyword/get`；为兼容参考输入，`type=mobile` 等同 `client=mobile`。

多重搜索的 `kind` 缺省为 `track`，接受与普通搜索相同的统一名称和网易云数字类型，参考字段 `type` 是其别名。网易云固定使用 WeAPI `/api/search/suggest/multimatch` 并精确提交 `s/type`；`result.orders` 决定分区顺序，未列入顺序但实际返回的数组仍会追加保留。`artist/playlist/new_mlog` 等已知分区分别规范化为统一歌手、歌单与视频资源，未知分区以不透明条目表达；完整上游响应位于结果扩展。

本地歌曲匹配的 `md5` 必填并按 32 位十六进制校验，标题、专辑和歌手允许为空以保留参考模块的默认分支；时长省略时按参考行为使用 0。若同时提供毫秒与秒数，两者四舍五入到毫秒后必须一致。网易云固定使用未加密直连 API `/api/search/match/new`，把一项标签记录序列化进 `songs`；上游 `result.ids/songs` 分别映射为匹配 ID 和统一候选曲目，空数组原样表达无命中。

会员摘要同时提供公开用户和当前账户两条统一路径。网易云两者都固定使用 WeAPI `/api/music-vip-membership/front/vip/info`：公开用户把引用 ID 作为 `userId`，当前账户按参考默认分支提交空字符串并由 `account` 选择登录态。`redVipLevel/redVipAnnualCount/redVipLevelIcon` 分别映射为等级、年费次数和图标；该公开接口没有可靠有效期和激活态，因此相关字段保持可空。更完整的客户端会员信息由独立 `/vip/info/v2` 模块后续映射，不会借公开摘要伪造。

为兼容网易云参考项目，横幅端点也接受 `type=0|1|2|3`，依次对应 PC、Android、iPhone、iPad；响应始终使用统一字段与客户端名称。

广播电台目录同时接受参考项目的 `categoryId/regionId/lastId` 命名。网易云以 `last_id+score` 作为真实游标；两者可独立传入，另一项分别按 `0/-1` 补齐。参考接口类型虽公开 `offset`，但模块实现与真实上游都不应用它，因此 TuneWeave 仍接收并在分页扩展返回 `requested_offset` 与 `offset_applied=false`，不会把首屏伪装成偏移页。首屏还可能插入推荐电台，实际 `data` 数量可以大于请求 `limit`，TuneWeave 保留完整上游结果并以真实末项生成下一游标。

评论读取与写入共用目标类型和平台边界：`type` 接受 `track/mv/playlist/album/radio_episode/video/event/radio_station`、网易云参考数字 `0..7` 以及写操作一节列出的名称别名；`ref` 决定内容平台，`account` 只选择该平台登录态。`view` 缺省为 `all`，也可取 `hot` 或 `replies`；提供 `parent_comment_id` 而省略 `view` 时自动选择 `replies`。`view=all` 不带 `sort` 时使用普通历史目录及 `limit/offset/before_time_ms`，带 `sort=recommended|hot|time` 时使用现代目录并接受 `page`，只有时间排序接受 `cursor`；`view=hot` 返回热门目录，`view=replies` 要求父评论 ID。`limit` 范围是 1–100。兼容字段包括 `sortType`、`pageSize`、`pageNo`、`before/beforeTime/time`、`parentCommentId` 和 `showInner`，排序数字 `1/99/2/3` 分别映射推荐/推荐/热门/时间。

评论响应把普通、热门、置顶和当前父评论分别放在 `comments/hot_comments/top_comments/current_comment`，不会把不同语义的条目混入同一数组。平台若没有应用请求页大小，TuneWeave 保留真实返回数量，并在 `meta.pagination.extensions.limit_applied=false` 明示；例如网易云现代推荐评论实测请求 2 条仍返回 10 条。事件评论的网易引用必须使用动态接口给出的完整 `A_EV_2_...` thread ID。

批量统计端点的 `type` 使用同一套评论目标名称、别名和数字 `0..7`；`ids` 是逗号分隔的平台资源 ID，兼容单个 `id`，保留顺序与重复项，省略或过滤空项后为空时返回成功空批次。网易云固定使用 WeAPI `/api/resource/commentInfo/list`，账户不是必需，但提供 `account` 后可取得对应点赞态。该平台的视频统计可能把公开哈希转换为内部评论资源 ID；动态统计则要求主资源数值 ID，并把 canonical 目标返回为完整 `A_EV_2_{id}_0`，不能在该端点提交评论目录所用的完整动态 thread ID。调用方应以 `requested_ref` 关联原请求、以 `target` 调用后续评论线程能力。

评论反应路径把反应类型作为可扩展段；平台按读写能力分别声明并只执行自己实际支持的类型。`GET` 的统一输入使用与评论同平台的 `target_user_ref` 指向评论作者。网易云“抱一抱”目录使用 `reaction=hug`，要求登录态，并兼容参考字段 `uid`/`target_user_id`、`pageSize`、`pageNo`、`idCursor`；其两个续页值分别以不透明 `cursor/id_cursor` 接收，并在 `meta.pagination.extensions.next_cursor/next_id_cursor` 返回，调用方不得解析其中本地化日期文本。默认 `limit=100`、`page=1`，`uid` 会按评论资源平台构造成用户引用；同时提交引用和 ID 时两者必须一致。`PUT/DELETE` 分别启用和停用 `reaction`；网易云当前支持 `reaction=like`，精确映射参考 `t=1/0` 两个分支，并使用同一套八种评论目标和动态完整 thread ID。

### 媒体与跨平台解析

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/audio/recognize` | `{platform?, account?, fingerprint, duration_seconds}`；指纹最大 131072 字节，时长 1–300 秒 | `AudioRecognition` |
| GET | `/v1/tracks/{ref}/lyrics` | `platform?` 不覆盖引用平台 | `Lyrics` |
| GET | `/v1/tracks/{ref}/stream` | `quality?`、`variant?`、`bitrate?`、`playback_platform?`、`fallback?`、`fallback_platforms?`、`unblock?`、`source?`、`account?` | `MediaStream` |
| GET | `/v1/tracks/streams` | `refs` 或 `ids`（兼容 `id`）、`platform?`、同上播放控制参数 | `StreamBatch`；逐项成功或失败，保留输入顺序与重复项 |
| POST | `/v1/tracks/streams` | JSON `{refs?|ids?, platform?, quality?, variant?, bitrate?, playback_platform?, fallback?, fallback_platforms?, unblock?, source?, account?}` | `StreamBatch`；`refs/ids` 可为字符串或字符串数组 |
| POST | `/v1/resolve` | 完整解析请求，见下文 | `Stream` |
| GET | `/v1/videos/{ref}` | `account?` | `Video`，含封面、UP 主和分 P 摘要 |
| GET | `/v1/videos/{ref}/parts` | 分页 | `VideoPart[]` |
| GET | `/v1/videos/{ref}/stream` | `part?`、`kind=audio|video`、`quality?`、`account?` | `Stream` 或视频流结构 |

为兼容参考项目调用方，音频识别请求也接受 `audio_fp`/`audioFP` 作为 `fingerprint` 的别名、`duration` 作为 `duration_seconds` 的别名；响应只使用统一字段名。

音频流的统一音质为 `auto/low/standard/higher/high/lossless/hires/surround/spatial/dolby/master`。网易云兼容字段 `level` 是 `quality` 的别名，并完整接受 `standard/higher/exhigh/lossless/hires/jyeffect/sky/dolby/jymaster`，其中 `exhigh/jyeffect/sky/jymaster` 分别映射为 `high/surround/spatial/master`。`variant=default|legacy|modern` 选择 provider 推荐后端、旧版码率后端或新版等级后端；兼容字段 `backend` 接受 `v0/song_url` 与 `v1/song_url_v1` 等别名。网易云缺省使用现代 v1；`variant=legacy` 时 `bitrate`（兼容 `br`）按原始无符号 bit/s 精确提交，省略时再由 `quality` 映射默认码率；现代后端按参考行为忽略 `br`。

批量 GET 的 `refs` 是逗号分隔完整资源引用；`ids/id` 是平台内 ID，`platform` 省略时使用服务默认平台，且只能与 `ids` 一起使用。POST 的 `refs/ids` 既可为单个字符串或逗号字符串，也可为字符串数组。两种输入都不折叠重复项；混合平台引用按来源 provider 分组调用原生批量能力，再严格还原原顺序。`StreamBatch.outcomes` 为每个输入返回独立 `status/stream/error_code/error/extensions`，单项不可用不会把整个 HTTP 请求变成失败；各 provider 的完整批量响应位于 `extensions.provider_batches`。

`unblock=true` 是参考 `/song/url/v1?unblock=true` 与 `/song/url/match` 的统一兼容预设，不另建第二套解灰逻辑。指定 `source=qq|kugou|kuwo|migu|...` 时先在该平台严格匹配，再回到原平台；省略时依次尝试 QQ、酷狗、酷我、咪咕和原平台。该模式始终保留原平台兜底，所以兼容输入中的 `fallback=false` 不会关闭兜底；为避免两套路由规则冲突，不能同时提交 `playback_platform` 或 `fallback_platforms`。`account` 绑定首个目标来源，所有尝试及失败原因都返回在 `attempts`。

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
| POST | `/v1/auth/challenges` | `{platform, method, principal, account?}` | 短信等挑战事务 |
| POST | `/v1/auth/challenges/validate` | `{platform, account?, method?, principal, code, country_code?}` | `AuthChallengeValidation`；仅校验挑战码，不创建登录态 |
| POST | `/v1/auth/challenges/{transaction_id}/verify` | `{code}` | 验证状态；成功时保存登录态 |
| POST | `/v1/auth/session/refresh` | `{platform, account?}` | 刷新状态和脱敏账户摘要 |
| GET | `/v1/auth/session` | `platform`、`account?` | 当前会话状态，不返回凭据 |
| DELETE | `/v1/auth/session` | `platform`、`account?` | 删除结果 |
| GET | `/v1/account` | `platform`、`account?` | 脱敏账户资料与权益摘要 |
| GET | `/v1/account/playlists` | `platform`、`account?`、分页 | `Playlist[]` |
| GET | `/v1/account/library/albums` | `platform`、`account?`、分页 | 已收藏的 `Album[]`；收藏时间保留在条目扩展，付费专辑计数等保留在分页扩展 |
| GET | `/v1/account/library/radio-stations` | `platform`、`account?`、分页 | 已收藏的 `RadioStation[]`；收藏条目原文保留在单项扩展，完整响应及平台分页字段保留在分页扩展 |
| GET | `/v1/account/following/artists` | `platform`、`account?`、分页 | 已关注的 `Artist[]`；关注时间和平台原始资料保留在条目扩展 |
| GET | `/v1/account/following/artists/new-videos` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Video[]`；`before` 与 `next_before_ms` 均为毫秒时间戳 |
| GET | `/v1/account/following/artists/new-tracks` | `platform`、`account?`、`limit?`、`before?` | 已关注歌手的新 `Track[]`；上游新曲总数保留为分页 `total` |
| GET | `/v1/account/following/artists/new-works` | `platform`、`account?`、`limit?`、`before?`、`source_type?`、`first_request?` | `ArtistWorkUpdate[]`；歌曲/MV 混合更新流，未知来源保留原文 |
| GET | `/v1/account/following/artists/new-tracks/play-all` | `platform`、`account?` | 最近至多 50 首新 `Track[]`；固定快照，不伪装成可翻页目录 |
| GET | `/v1/account/favorites/tracks` | `platform`、`account?`、分页 | `Track[]` |
| GET | `/v1/account/history` | `platform`、`account?`、`period=all_time|week`、分页 | `PlaybackHistoryEntry[]`，含 `track`、`play_count`、`score`、`last_played_at` |
| GET | `/v1/account/cloud/lyrics` | `platform?`、`account?`、`user_id`、`track_id` | 云盘文件标签中的统一 `Lyrics` |

`principal_type` 至少允许平台实际支持的 `email`、`phone` 或平台账号类型；密码默认按明文接收并立即在适配器内完成平台要求的摘要，也可用 `password_format: "md5"` 明确提交已有摘要。`method` 至少允许 `sms`，并可由平台扩展。上游存在多种登录方式时必须全部接入，不能只保留二维码这一条流程。

国家区号目录允许省略 `platform` 并使用服务默认平台；`account` 只选择该平台的请求会话。网易云固定以 EAPI 调用 `/api/lbs/countries/v1`，公开目录不要求登录；统一结果保留上游分组顺序、电话区号、地区代码和中英文名称。不存在的非默认账户别名仍按账户隔离规则返回认证错误，不会静默退回默认会话。

`/v1/auth/principals/status` 只查询注册状态，不发送验证码、不登录。`principal_type` 省略时默认 `phone`；网易云兼容参考字段 `phone/countrycode`，分别作为 `principal/country_code` 的别名，也接受 `countryCode`，手机号和区号均可为字符串或数字，区号缺省或为空时使用 `86`。统一结果用 `exists` 表示是否注册，并保留 `has_password`、平台已脱敏的 `display_name`、`avatar_url` 和 `platform_code`；完整上游响应位于 `extensions.response`，原始手机号不进入稳定字段或日志。

`/v1/auth/challenges/validate` 与事务验证端点语义不同：它只调用平台的验证码校验能力，不登录、不保存 Cookie，也不要求先发送验证码。`method` 省略时默认为 `sms`；网易云还兼容参考字段 `phone/captcha/ctcode`，分别作为 `principal/code/country_code` 的别名，手机号和区号都接受字符串或数字，区号缺省或为空时使用 `86`。`valid=false` 是正常业务结果，仍以 HTTP 200 返回，并通过 `platform_code`、`message` 和 `extensions.response` 保留平台信息；手机号和验证码不会回显。需要验证码登录时仍使用 `/v1/auth/challenges` 创建不透明事务，再调用 `/{transaction_id}/verify`。

二维码与验证码端点返回的 `transaction_id` 是 TuneWeave 生成的随机不透明标识，不是上游二维码 key、手机号或 token。敏感字段仅在请求生命周期或短期事务仓库内使用，保存后的平台凭据只通过账户别名引用；密码、验证码、Cookie 与上游事务标识不会写入普通响应。

### 写操作

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/playlists` | `{platform, account?, name, description?, privacy?}` | 新 `Playlist` |
| PATCH | `/v1/playlists/{ref}` | `{account?, name?, description?, privacy?}` | 更新后的 `Playlist` |
| DELETE | `/v1/playlists/{ref}` | `account?` | 删除结果 |
| POST | `/v1/playlists/{ref}/tracks` | `{account?, operation: "add"|"remove", tracks: ["platform:id"]}` | 每首歌的写入结果 |
| POST | `/v1/resources/{type}/{ref}/comments` | 查询参数 `account?`；JSON `{content}` | `CommentMutationResult`，创建评论 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/replies` | 查询参数 `account?`；JSON `{content}` | `CommentMutationResult`，回复指定评论 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}` | `account?` | `CommentMutationResult`，删除指定评论 |
| PUT | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`，启用评论反应 |
| DELETE | `/v1/resources/{type}/{ref}/comments/{comment_id}/reactions/{reaction}` | `account?` | `CommentReactionMutationResult`，停用评论反应 |
| POST | `/v1/resources/{type}/{ref}/comments/{comment_id}/reports` | 查询参数 `account?`；JSON `{reason}` | `CommentReportResult`，提交评论举报 |
| PUT | `/v1/account/library/albums/{ref}` | `account?` | `SubscriptionResult`，收藏专辑 |
| DELETE | `/v1/account/library/albums/{ref}` | `account?` | `SubscriptionResult`，取消收藏专辑 |
| PUT | `/v1/account/library/radio-stations/{ref}` | `account?` | `SubscriptionResult`，收藏广播电台 |
| DELETE | `/v1/account/library/radio-stations/{ref}` | `account?` | `SubscriptionResult`，取消收藏广播电台 |
| PUT | `/v1/account/following/artists/{ref}` | `account?` | `SubscriptionResult`，关注歌手 |
| DELETE | `/v1/account/following/artists/{ref}` | `account?` | `SubscriptionResult`，取消关注歌手 |
| PUT | `/v1/account/avatar` | 查询参数 `platform?`、`account?`、`filename?`、`image_size?`、`crop_x?`、`crop_y?`；请求体为图片字节，`Content-Type: image/*`，最大 20 MiB | `ImageUploadResult` |
| POST | `/v1/account/cloud/uploads` | 查询参数 `platform?`、`account?`、`filename`、`bitrate?`、`song_name?`、`artist?`、`album?`；请求体为原始音频字节，最大 500 MiB | `CloudUploadResult`，由 TuneWeave 代理检查、上传、登记并发布 |
| POST | `/v1/account/cloud/uploads/ticket` | 查询参数 `platform?`、`account?`；JSON `{md5, file_size, filename, bitrate?, content_type?}` | `CloudUploadTicket`，含是否需要上传、临时曲目 ID、资源 ID 及受限对象存储请求信息 |
| POST | `/v1/account/cloud/uploads/complete` | 查询参数 `platform?`、`account?`；JSON `{provisional_track_id, resource_id, md5, filename, song_name?, artist?, album?, bitrate?}` | `CloudUploadResult`，登记并发布后的云盘曲目引用 |
| POST | `/v1/account/cloud/imports` | 查询参数 `platform?`、`account?`；JSON `{md5, source_track_id?, bitrate, file_size, file_type, song_name, artist?, album?}` | `CloudImportResult`，免上传导入结果及云盘曲目引用 |
| POST | `/v1/account/cloud/matches` | 查询参数 `platform?`、`account?`；JSON `{user_id, cloud_track_id, target_track_id?}` | `CloudMatchResult`；目标为 `0` 或省略时取消匹配 |
| PUT | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 收藏结果 |
| DELETE | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 取消收藏结果 |

写入目标平台与歌曲引用平台不同时，TuneWeave 先执行严格匹配；低于阈值时返回 `match_rejected`，不得把同名但不同版本的歌曲写进歌单。

评论目标类型接受统一名称 `track/mv/playlist/album/radio_episode/video/event/radio_station`，也兼容网易云参考数字 `0..7`；`song/music`、`dj/program`、连字符形式分别是对应统一类型的输入别名。`ref` 决定评论所属平台，`account` 只选择该平台的隔离登录态，评论 ID 始终按不透明字符串处理。事件评论的网易引用 ID 必须是从动态接口取得的完整 `A_EV_2_...` thread ID。创建、回复和删除使用同一评论写结果结构，明确返回目标、`create/reply/delete` 动作、可用的新评论 ID 和平台扩展；空白内容会被拒绝，但合法内容的首尾空格不会被静默改写。反应启用与停用则使用独立的 `CommentReactionMutationResult`，避免混淆评论本体动作和评论反应状态。

举报端点只把目标和账户选择统一化，不扩张平台能力。理由必填且只以去除空白后的结果判空，合法文本原样提交。网易云参考模块仅支持歌曲评论，因此该适配器只接受 `type=track`，固定构造 `R_SO_4_{id}` 并以 EAPI 调用 `/api/report/reportcomment`；其他目标在上游请求前返回 `invalid_request`。

头像请求省略 `filename` 与 `Content-Type` 时分别使用 `avatar.jpg` 和 `image/jpeg`。为兼容网易云参考项目，查询参数也接受 `imgSize/imgX/imgY` 与 `img_size/img_x/img_y`；该参考实现从首次引入起就没有把这三个裁剪参数传给上游，因此网易云适配器会接受并在扩展中标记 `applied=false`，不会虚假执行或声明裁剪。调用方应在上传前自行生成目标方形图片。

`POST /v1/account/cloud/uploads` 是兼容代理流程：调用方提交原始音频字节和必填安全文件名，`Content-Type` 省略时由 provider 按扩展名推断。TuneWeave 计算 MD5、解析音频标签、检查是否需要上传、上传 NOS、登记云盘信息并发布；显式 `song_name/artist/album` 优先于文件标签，标签缺失时曲名取安全化文件主名，歌手和专辑分别使用“未知艺术家/未知专辑”。查询字段 `song`、`songName` 是 `song_name` 的兼容别名。该端点保持参考服务的 500 MiB 上限，并在单次请求期间持有一份音频缓冲；适合兼容和较小文件，不会把 NOS token 返回给调用方。

云盘大文件优先采用三段直传事务，避免让 TuneWeave 服务端持有整份音频：调用方先计算文件 MD5 与字节数并申请 `CloudUploadTicket`；仅当 `upload_required=true` 时，按返回的 `upload_method`、`upload_url` 和 `upload_headers` 原样上传音频字节；随后用票据中的临时曲目 ID 与资源 ID 调用完成端点。`upload_required=false` 时跳过字节上传，直接完成登记和发布。文件大小统一为字节，码率统一为 bit/s，省略码率时使用 `999000`。为兼容网易云参考参数，票据端点接受 `fileSize/contentType`，完成端点接受 `songId/resourceId/song`。

直传票据中的 `x-nos-token` 是短期敏感凭据，只能发送给同一票据给出的受限对象存储地址，不得写入日志、持久化或转发给其他来源。provider 必须限制上传目标域名和查询参数；网易云当前只接受无凭据、无自定义端口的 `http(s)://*.127.net` NOS 地址，并固定使用 `offset=0&complete=true&version=1.0`。普通 Debug 输出与 `extensions` 不包含该 token。

云盘免上传导入适用于文件已经被其他用户上传，或文件本身是目标平台音源的场景。TuneWeave 的 `bitrate` 仍统一使用 bit/s；网易云参考接口内部使用 kbps，因此 provider 执行 `floor(bit/s / 1000)`，调用方不得自行预除。省略 `source_track_id` 时使用参考默认 `-2`；歌手和专辑缺省时由网易 provider 使用“未知”。兼容字段为 `id/fileSize/fileType/song`。

云盘歌词兼容查询字段 `uid/sid`。云盘匹配兼容 JSON 字段 `uid/sid/asid`，ID 可为字符串或数字；`target_track_id=0`、`asid=0` 或省略目标均表示取消现有匹配，而不是匹配到曲目 0。两项操作都只作用于查询参数选中的平台账户，不会改变其他平台登录态。

### 平台扩展

不能合理统一的功能放在 `/v1/extensions/{platform}`，仍使用统一包络和错误码。

| 方法 | 端点 | 用途 |
| --- | --- | --- |
| GET | `/v1/extensions/netease/calendar` | 查询指定毫秒时间范围内的网易云账户音乐日历 |
| POST | `/v1/extensions/netease/api` | 在固定网易云域名上调用指定 `/api/...` 路径，支持 `eapi/weapi/api/linuxapi/xeapi` |
| GET | `/v1/extensions/netease/batch` | 以参考项目的查询参数形式批量调用网易云 `/api/...` 路径 |
| POST | `/v1/extensions/netease/batch` | 以 JSON 对象批量调用网易云 `/api/...` 路径 |
| GET | `/v1/extensions/netease/partner/tasks` | 查询音乐合伙人当日任务与待评作品 |
| POST | `/v1/extensions/netease/partner/run` | 按服务端策略执行合伙人任务并返回逐账户报告 |

网易云日历接受统一参数 `start_time`、`end_time`，并兼容参考项目的 `startTime`、`endTime`；值必须是无符号 Unix 毫秒时间戳。为完整保留参考实现的运行时行为，任一时间参数省略时都会使用本次请求的当前毫秒时间，两个参数也允许同时省略。`account` 选择服务端保存的网易云登录态。端点固定使用 WeAPI 调用 `/api/mcalendar/detail`，成功时完整上游日历 JSON 位于统一包络的 `data` 中。

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

为避免把通用入口变成凭据注入或 SSRF 接口，请求 `uri` 只能是非空 `/api/...` 路径，目标域名由服务端配置且不能由调用者覆盖；请求体拒绝 `cookie`、`domain`、`headers`、`proxy`、`ua` 等传输覆盖字段，`data.cookie` 也会被拒绝。登录态只能通过 `account` 选择服务端保存的账户别名。XEAPI 的公钥注册、X25519 会话密钥与加密响应解包均由适配器内部完成，不向调用者暴露密钥材料。

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
