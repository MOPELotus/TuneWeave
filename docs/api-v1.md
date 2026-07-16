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
  "has_more": true
}
```

上游只提供页码或游标时，由适配器换算并在内部保存游标。无法可靠获得总数时，`total` 为 `null`。

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
| GET | `/v1/search` | `q`、`type=track|album|artist|playlist|video`、`platform?`、分页 | 对应统一实体列表 |
| GET | `/v1/tracks/{ref}` | `account?` | `Track` |
| GET | `/v1/albums/{ref}` | `account?` | `Album` |
| GET | `/v1/albums/{ref}/tracks` | 分页、`account?` | `Track[]` |
| GET | `/v1/albums/{ref}/stats` | `account?` | `AlbumStats` |
| GET | `/v1/digital-albums` | `platform?`、`account?`、`catalog=latest|style`、`area?`、`type?`、分页 | `DigitalAlbum[]`；上游不返回可靠总数时 `total=null` |
| GET | `/v1/digital-albums/{ref}` | `account?` | `DigitalAlbum` |
| GET | `/v1/artists/{ref}` | 无 | `Artist` |
| GET | `/v1/artists/{ref}/tracks` | 分页 | `Track[]` |
| GET | `/v1/artists/{ref}/albums` | 分页 | `Album[]` |
| GET | `/v1/playlists/{ref}` | `account?` | `Playlist` |
| GET | `/v1/playlists/{ref}/tracks` | 分页、`account?` | `Track[]`；B 站合集/收藏夹视频按可播放音频内容归一并保留 `video_ref` |
| GET | `/v1/users/{ref}/favorites/tracks` | 分页、`account?` | 指定用户公开引用下的 `Track[]`；需要平台登录态时由 `account` 选择 |
| GET | `/v1/users/{ref}/history` | `period=all_time|week`、分页、`account?` | 指定用户的 `PlaybackHistoryEntry[]` |
| GET | `/v1/charts` | `platform?` | `Playlist[]`，其中榜单仍用歌单模型表示 |
| GET | `/v1/charts/{ref}/tracks` | 分页 | `Track[]` |
| GET | `/v1/recommendations/tracks` | `platform?`、`account?`、`refresh?`、分页 | `Track[]`；推荐理由保存在 `extensions.recommendation` |
| GET | `/v1/recommendations/playlists` | `platform?`、`account?`、分页 | `Playlist[]` |

### 媒体与跨平台解析

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| GET | `/v1/tracks/{ref}/lyrics` | `platform?` 不覆盖引用平台 | `Lyrics` |
| GET | `/v1/tracks/{ref}/stream` | `quality`、`playback_platform?`、`fallback?`、`fallback_platforms?`、`account?` | `Stream` |
| POST | `/v1/resolve` | 完整解析请求，见下文 | `Stream` |
| GET | `/v1/videos/{ref}` | `account?` | `Video`，含封面、UP 主和分 P 摘要 |
| GET | `/v1/videos/{ref}/parts` | 分页 | `VideoPart[]` |
| GET | `/v1/videos/{ref}/stream` | `part?`、`kind=audio|video`、`quality?`、`account?` | `Stream` 或视频流结构 |

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
| POST | `/v1/auth/qr` | `{platform, account?, login_type?}` | 二维码事务 ID、二维码 URL/图片、过期时间 |
| GET | `/v1/auth/qr/{transaction_id}` | 无 | `waiting/scanned/confirmed/expired/failed`；成功时保存登录态 |
| POST | `/v1/auth/password` | `{platform, account?, principal_type, principal, password}` | 登录状态和脱敏账户摘要 |
| POST | `/v1/auth/challenges` | `{platform, method, principal, account?}` | 短信等挑战事务 |
| POST | `/v1/auth/challenges/{transaction_id}/verify` | `{code}` | 验证状态；成功时保存登录态 |
| POST | `/v1/auth/session/refresh` | `{platform, account?}` | 刷新状态和脱敏账户摘要 |
| GET | `/v1/auth/session` | `platform`、`account?` | 当前会话状态，不返回凭据 |
| DELETE | `/v1/auth/session` | `platform`、`account?` | 删除结果 |
| GET | `/v1/account` | `platform`、`account?` | 脱敏账户资料与权益摘要 |
| GET | `/v1/account/playlists` | `platform`、`account?`、分页 | `Playlist[]` |
| GET | `/v1/account/favorites/tracks` | `platform`、`account?`、分页 | `Track[]` |
| GET | `/v1/account/history` | `platform`、`account?`、`period=all_time|week`、分页 | `PlaybackHistoryEntry[]`，含 `track`、`play_count`、`score`、`last_played_at` |

`principal_type` 至少允许平台实际支持的 `email`、`phone` 或平台账号类型；密码默认按明文接收并立即在适配器内完成平台要求的摘要，也可用 `password_format: "md5"` 明确提交已有摘要。`method` 至少允许 `sms`，并可由平台扩展。上游存在多种登录方式时必须全部接入，不能只保留二维码这一条流程。

二维码与验证码端点返回的 `transaction_id` 是 TuneWeave 生成的随机不透明标识，不是上游二维码 key、手机号或 token。敏感字段仅在请求生命周期或短期事务仓库内使用，保存后的平台凭据只通过账户别名引用；密码、验证码、Cookie 与上游事务标识不会写入普通响应。

### 写操作

| 方法 | 端点 | 主要输入 | `data` |
| --- | --- | --- | --- |
| POST | `/v1/playlists` | `{platform, account?, name, description?, privacy?}` | 新 `Playlist` |
| PATCH | `/v1/playlists/{ref}` | `{account?, name?, description?, privacy?}` | 更新后的 `Playlist` |
| DELETE | `/v1/playlists/{ref}` | `account?` | 删除结果 |
| POST | `/v1/playlists/{ref}/tracks` | `{account?, operation: "add"|"remove", tracks: ["platform:id"]}` | 每首歌的写入结果 |
| PUT | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 收藏结果 |
| DELETE | `/v1/account/favorites/tracks/{ref}` | `platform`、`account?` | 取消收藏结果 |

写入目标平台与歌曲引用平台不同时，TuneWeave 先执行严格匹配；低于阈值时返回 `match_rejected`，不得把同名但不同版本的歌曲写进歌单。

### 平台扩展

不能合理统一的功能放在 `/v1/extensions/{platform}`，仍使用统一包络和错误码。

| 方法 | 端点 | 用途 |
| --- | --- | --- |
| GET | `/v1/extensions/netease/partner/tasks` | 查询音乐合伙人当日任务与待评作品 |
| POST | `/v1/extensions/netease/partner/run` | 按服务端策略执行合伙人任务并返回逐账户报告 |

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
