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

请求 `word_synced=true` 可获取逐字歌词。`word_synced` 存在时客户端应优先显示它，再回退到 `plain`；`translated` 和 `romanized` 是独立轨道。

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

## 资源详情

| 资源 | 常用端点 |
| --- | --- |
| 歌曲 | `/v1/tracks/{ref}`、`/lyrics`、`/availability`、`/versions`、`/similar` |
| 专辑 | `/v1/albums/{ref}`、`/tracks`、`/stats` |
| 歌手 | `/v1/artists/{ref}`、`/tracks`、`/albums`、`/videos`、`/stats` |
| 用户 | `/v1/users/{ref}`、`/playlists/created`、`/favorites/*`、`/history` |
| 歌单 | `/v1/playlists/{ref}`、`/items`、`/tracks` |
| 视频 | `/v1/videos/{ref}`、`/parts`、`/subtitles`、`/playback`、`/stats` |
| 播客节目 | `/v1/episodes/{ref}`、`/lyrics` |

`GET/POST /v1/artists/details` 批量读取歌手描述。默认返回扩展资料、百科、组合成员、主图和相册；调用方可分别通过 `ex_singer`、`wiki_singer`、`group_singer`、`pic`、`photos` 控制，POST 也接受对应的 `include_*` 别名。QQ 主图和相册使用强类型字段，所有图片地址在返回前校验并统一为 HTTPS。

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
| `GET/POST` | `/v1/tracks/streams` | 批量解析歌曲流 |
| `GET/POST` | `/v1/videos/streams` | 批量解析视频流 |
| `GET` | `/v1/videos/{ref}/audio-stream` | 选择视频的音频轨 |
| `GET` | `/v1/videos/{ref}/video-stream` | 选择视频轨 |
| `GET` | `/v1/videos/{ref}/playback` | 返回完整播放清单 |

常用 `quality` 值包括 `auto`、`standard`、`higher`、`high`、`lossless`、`hires`、`spatial`、`dolby` 和 `master`。响应中的 `actual_quality` 是最终取得的档位。

跨平台回退使用标题、歌手、专辑、时长、ISRC 和版本信息进行匹配。`attempts` 按执行顺序记录每个平台的结果。试听片段只在没有更合适完整资源时作为结果返回。

歌曲播放默认同时启用解锁阶段。对网易云音乐歌曲，服务端先尝试原始音源；原始 URL 缺失、只有试听片段或权益不足时，再按首选平台、手动回退平台和公开音源顺序匹配。默认公开音源顺序为 QQ 音乐、酷狗、酷我、咪咕、汽水。`unblock=false` 才会关闭该附加阶段；`fallback=false` 不会关闭默认解锁，这是两个独立控制项。`source` 可以把某个公开音源提升到解锁阶段首位，但不会改变歌曲的原始引用。

网易云返回的 `m数字.music.126.net` 媒体 URL 会同时提供兼容的 `m数字c.music.126.net` URL 和原始 URL，调用方应按 `url`、`backup_urls` 顺序尝试；重定向端点会发送 `Referrer-Policy: no-referrer`，避免网易 CDN 因来源页拒绝播放。服务端不会接受调用方注入的媒体 URL、代理或请求头。

歌曲下载端点接受相同的 `playback_platform`、`fallback`、`fallback_platforms`、`unblock` 和 `source` 控制项。服务端优先使用原平台提供的完整下载 URL；原生下载不可用时复用统一播放解析链，并将实际来源、匹配结果和尝试记录写入下载响应。显式指定其他 `source` 时直接解析该公开音源；仅取得试听片段时不会将其作为完整下载返回。

## 歌单与写操作

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

账户资料库和收藏操作位于 `/v1/account/*`，包括个人歌单、喜欢歌曲、收藏专辑、播客、视频、广播、历史、会员状态和云盘。写操作需要目标平台凭证，并可能受平台权益、频率和风控限制。

## 登录与 Uni Playlist

登录端点位于 `/v1/auth/*`，详见[登录与凭证](authentication.md)。

Uni Playlist 端点位于 `/v1/uni/*`，服务端歌单也可通过统一 `/v1/playlists/{uni-ref}` 读取和播放，详见 [Uni Playlist](uni-playlist.md)。

## 平台扩展

无法映射为稳定统一语义的能力位于：

```text
/v1/extensions/{platform}/...
```

扩展端点仍使用统一响应包络、凭证选择和错误模型，但请求与 `data` 可能包含平台专属结构。它们不允许调用方覆盖目标域名、通用代理、Cookie、任意请求头或重定向策略。完整扩展路由请查阅 [`routes.json`](routes.json)。
