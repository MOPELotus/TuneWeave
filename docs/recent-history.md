# 当前账号的最近歌曲、专辑与歌单记录

最低版本：`0.1.0-alpha.11`。三项能力目前仅网易云实现，使用运行实例的 `/v1/capabilities?platform=netease` 判断是否可用。它们独立于 `listening_history`（`GET /v1/account/history` 的平台听歌历史；网易云返回周榜/累计结果，酷狗 Standard 返回原生账号播放记录）及 `recent_podcast_episode_history`。

| 请求 | capability | Provider 方法 | 记录模型 |
| --- | --- | --- | --- |
| `GET /v1/account/history/tracks` | `recent_track_history` | `recent_track_history` | `RecentTrackHistoryEntry` |
| `GET /v1/account/history/albums` | `recent_album_history` | `recent_album_history` | `RecentAlbumHistoryEntry` |
| `GET /v1/account/history/playlists` | `recent_playlist_history` | `recent_playlist_history` | `RecentPlaylistHistoryEntry` |

## 请求和账号

```http
GET /v1/account/history/tracks?platform=netease&limit=100&offset=0
X-TuneWeave-Credential: twc1_<opaque-value>
```

- `platform` 可省略，使用实例默认平台。
- `account` 是可选的服务器托管账号别名，省略时使用 `default`。使用调用方凭证时不传 `account`；同时传入会返回 `400 invalid_request`。
- 请求作用域只使用所选账号的凭证，不回退到另一账号；不缓存最近记录，也不持久化调用方凭证。
- 匿名、缺少账号或登录失效返回 `401 authentication_required`。只读历史不会修改账号、公共播放会话或播放队列。
- 不支持的平台返回 `422 capability_not_supported`，错误的 `details.capability` 是相应独立能力名，不能把不支持当作空记录。

## 数量与继续读取

`limit` 默认 100，接受整数 1–100；`offset` 默认 0，只接受 0。不认识的字段、重复参数、负数、溢出及格式错误返回 `400 invalid_request`。

网易云的三个内部端点只有 `limit` 参数，没有已知的 offset/游标。TuneWeave 的 100 是当前 API 接受的请求上限，依据上游公开文档的默认窗口保守限定；不是声称平台的全部历史只有 100 条，也不代表已经证实平台能返回 300 条。平台返回的数量可以小于请求值。若响应超出请求 limit，仅返回前 limit 条。

响应使用标准列表包络，`data` 是记录数组，`meta.pagination` 如下：

```json
{
  "limit": 100,
  "offset": 0,
  "total": 8,
  "next_offset": null,
  "has_more": false,
  "extensions": {
    "continuation_supported": false,
    "limit_applied": true
  }
}
```

`total` 原样表达平台提供的总数，缺失时为 `null`；不从请求数推算。即使 total 大于本次返回数量也没有可用的下一页。返回平台给定的最近记录顺序，不二次排序、不按资源 ID 去重、不从歌曲推导最近专辑或歌单。这不是完整逐次播放事件日志，也不保证同一资源每次播放都留下一条记录；平台可能只保留资源的最近记录。

## 记录字段

- `track` / `album` / `playlist` 是既有规范化实体，`ref`、平台、字符串 ID、名称可直接用于展示和详情请求。歌曲保留歌手、专辑、时长；专辑保留歌手、封面；歌单保留作者、封面。
- `played_at` 来源于记录的 Unix 毫秒 `playTime`，转换成 UTC RFC 3339 并保留三位毫秒。缺失、null 或 0 表示未知，返回 `null`；负数、非整数、无法解析或超出四位年份范围返回 `502 upstream_error`。不会用接收时间、发行时间或歌单更新时间填充。
- `device` 可为 null，存在时包含可选 `operating_system`、`name`、`icon_url` 和空 `extensions`。
- 资源缺失/删除导致 `data` 为空、ID 无效、名称为空、记录 ID 与实体 ID 不一致、资源类型错误等情况，整个请求返回 `502 upstream_error`，不静默丢弃条目。平台若返回可识别的下架歌曲，保留其既有 `playable` 等状态。
- 不把上游原始响应、任意私有字段或账号凭证放入记录/分页/device 的 extensions。错误只返回规范化消息和可选的数字 `upstream_code`，不透传上游错误原文。

下面是 `data` 数组中单条记录的完整示例，使用虚构资源而非真实账号数据。

### 歌曲

```json
{
  "track": {
    "ref": "netease:123", "platform": "netease", "id": "123",
    "name": "Example track", "aliases": [],
    "artists": [{"ref": "netease:7", "name": "Example artist"}],
    "album": {"ref": "netease:456", "name": "Example album", "cover_url": "https://example.test/album.jpg"},
    "duration_ms": 180123, "isrc": null, "mv_ref": null,
    "playable": null, "available_qualities": [], "extensions": {}
  },
  "played_at": "2024-01-01T00:00:00.123Z",
  "device": {"operating_system": "android", "name": "Android", "icon_url": "https://example.test/android.png", "extensions": {}},
  "extensions": {}
}
```

### 专辑

```json
{
  "album": {
    "ref": "netease:456", "platform": "netease", "id": "456",
    "name": "Example album", "aliases": [],
    "artists": [{"ref": "netease:7", "name": "Example artist"}],
    "description": "", "cover_url": "https://example.test/album.jpg",
    "published_at": null, "track_count": 10, "company": null,
    "kind": null, "extensions": {}
  },
  "played_at": "2024-01-01T00:00:00.123Z",
  "device": {"operating_system": "android", "name": "Android", "icon_url": "https://example.test/android.png", "extensions": {}},
  "extensions": {}
}
```

### 歌单

```json
{
  "playlist": {
    "ref": "netease:789", "platform": "netease", "id": "789",
    "name": "Example playlist", "description": "",
    "cover_url": "https://example.test/playlist.jpg",
    "creator": {"ref": "netease:8", "name": "Example owner"},
    "track_count": 12, "tags": [], "subscribed": null,
    "created_at": null, "updated_at": null, "extensions": {}
  },
  "played_at": "2024-01-01T00:00:00.123Z",
  "device": {"operating_system": "android", "name": "Android", "icon_url": "https://example.test/android.png", "extensions": {}},
  "extensions": {}
}
```

## 错误

| HTTP | `error.code` | 例子 |
| --- | --- | --- |
| 400 | `invalid_request` | `limit=101`、`offset=1`、调用方凭证与 account 同时提供 |
| 401 | `authentication_required` | 缺少账号、网易云返回 301/401 |
| 403 | `permission_denied` | 已登录账号被平台拒绝访问 |
| 422 | `capability_not_supported` | 当前平台不支持该类最近记录 |
| 429 | `rate_limited` | 平台限流 |
| 502 | `upstream_error` | 非法记录、坏 JSON、平台错误 |
| 504 | `upstream_timeout` | 上游请求超时 |

只有平台成功返回空 list 才是成功的空历史。

## 协议来源与测试

内部 WeAPI 路径是 `/api/play-record/song/list`、`/api/play-record/album/list`、`/api/play-record/playlist/list`，请求 payload 为 `{"limit": N}`；使用既有网易云 WeAPI 加密和调用方账号选择。公开包装接口 `/record/recent/*` 不是 TuneWeave 的内部请求路径。

核对来源：[api-enhanced 最近歌曲模块](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/a8c781fd64faab17fedfd46e0615a2609307f163/module/record_recent_song.js)、[专辑模块](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/a8c781fd64faab17fedfd46e0615a2609307f163/module/record_recent_album.js)、[歌单模块](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/blob/a8c781fd64faab17fedfd46e0615a2609307f163/module/record_recent_playlist.js)。

固定 fixture 使用虚构数据，覆盖三类实体、时间、设备、空记录、重复/顺序、未知 total 和 malformed/boundary 输入；本地 HTTP 模拟端点覆盖真实 WeAPI 发送路径和并发账号隔离。这些可重复测试不等于真实账号在线验收。
