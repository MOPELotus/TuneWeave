# Uni Playlist

Uni Playlist 把不同平台的歌曲、MV、视频、播客节目和广播整理到同一有序列表中。内容来源与播放来源相互独立：原平台无法播放歌曲时，调用方可以让 resolver 按指定顺序寻找其他平台的严格匹配资源。

Uni Playlist 支持两种数据所有权模式：

- Server：TuneWeave 保存歌单和项目，调用方持有 `uni:<id>` 引用。
- Client：调用方保存版本化文档或项目，TuneWeave 只执行来源展开、标准化和播放。

两种模式通过显式导入和导出复制数据，不进行自动双向同步。

## Server 模式

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `GET` | `/v1/uni/playlists` | 分页列出服务端歌单 |
| `POST` | `/v1/uni/playlists` | 创建空歌单 |
| `POST` | `/v1/uni/playlists/imports` | 合并一个或多个平台集合并创建歌单 |
| `GET` | `/v1/uni/playlists/{ref}` | 读取歌单元数据 |
| `PATCH` | `/v1/uni/playlists/{ref}` | 修改名称或描述 |
| `DELETE` | `/v1/uni/playlists/{ref}` | 删除歌单和全部项目 |
| `GET` | `/v1/uni/playlists/{ref}/items` | 分页读取项目 |
| `POST` | `/v1/uni/playlists/{ref}/items` | 追加资源 |
| `DELETE` | `/v1/uni/playlists/{ref}/items/{item_id}` | 删除一次具体出现 |
| `PATCH` | `/v1/uni/playlists/{ref}/items/order` | 提交完整项目顺序 |
| `GET` | `/v1/uni/playlists/{ref}/export` | 导出 V1 文档 |
| `POST` | `/v1/uni/playlists/import-document` | 从 V1 文档创建服务端副本 |

创建空歌单：

```http
POST /v1/uni/playlists
Content-Type: application/json

{
  "name": "通勤",
  "description": "跨平台收藏"
}
```

添加可播放资源：

```http
POST /v1/uni/playlists/{ref}/items
Content-Type: application/json

{
  "items": [
    { "ref": "netease:1859245776", "kind": "track" },
    { "ref": "bilibili:bvid:BV1xx411c7mD", "kind": "video" }
  ],
  "accounts": {
    "netease": "personal",
    "bilibili": "default"
  }
}
```

同一个来源可以出现多次，每次都有独立 `item_id`。删除和排序使用项目 ID，不使用来源引用代替项目身份。

## 导入平台集合

来源可以用完整 `ref`，也可以用 `platform + type + id`：

```http
POST /v1/uni/playlists/imports
Content-Type: application/json

{
  "name": "跨平台合并",
  "sources": [
    { "platform": "netease", "type": "playlist", "id": "3778678" },
    { "platform": "netease", "type": "favorite_tracks", "id": "499129857", "account": "personal" },
    { "platform": "qq", "type": "favorite_tracks", "id": "<uin>", "account": "personal" },
    { "platform": "bilibili", "type": "season", "id": "3629748" },
    { "platform": "bilibili", "type": "favorite_folder", "id": "2883236382", "account": "default" }
  ]
}
```

公开集合不需要账户。私有或账户可见集合可以为每个来源单独指定 `account`。来源按请求顺序展开，来源内部顺序和重复项目都会保留；任一来源失败时不会创建部分歌单。导入结果的 `sources` 会返回来源名称与 `cover_url`，`favorite_tracks` 使用网易云或 QQ 的真实“喜欢”歌单元数据。

Provider 可以支持不同的 `type`。常用值包括 `playlist`、`favorite_tracks`、`season` 和 `favorite_folder`；请通过 `/v1/capabilities` 确认目标平台能力。

## Client 模式

客户端交换格式为 `tuneweave_uni_playlist_v1`。文档包含歌单身份、名称、描述、有序项目、稳定项目 ID、外部平台来源引用和紧凑元数据快照，不包含账户凭证或临时媒体信息。

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `POST` | `/v1/uni/materialize/imports` | 展开平台集合并分页返回客户端项目，不持久化 |
| `POST` | `/v1/uni/materialize/items` | 验证并标准化一组资源，不持久化 |
| `POST` | `/v1/uni/items/stream` | 播放一个客户端托管项目 |

标准化资源：

```http
POST /v1/uni/materialize/items
Content-Type: application/json

{
  "items": [
    { "ref": "qq:0039MnYb0qxYhV", "kind": "track" }
  ],
  "accounts": { "qq": "personal" }
}
```

播放返回的单个 V1 项目：

```http
POST /v1/uni/items/stream
Content-Type: application/json

{
  "item": { "...": "materialized item" },
  "quality": "lossless",
  "playback_platform": "qq",
  "fallback": true,
  "fallback_platforms": "netease,kugou,migu,kuwo,soda",
  "accounts": {
    "qq": "green-diamond",
    "netease": "personal"
  }
}
```

Client 请求也可以使用重复的 `X-TuneWeave-Credential` 请求头，为每个平台提供调用方托管凭证。

## 播放 Server 项目

```http
GET /v1/playlists/{uni-ref}/items/{item_id}/stream?quality=lossless&fallback=true&fallback_platforms=qq,netease,kugou
```

响应包含最终 `MediaStream`、实际平台和按顺序记录的尝试结果。跳转端点为：

```http
GET /v1/playlists/{uni-ref}/items/{item_id}/stream/redirect
```

媒体 URL 可能要求 `Referer` 或 `User-Agent`。302 无法附带这些请求头；此时应使用 JSON 流端点读取 `headers`，再由客户端请求媒体地址。

## 文档安全

V1 文档拒绝未知字段，并限制项目数量、文本长度、引用、时间和项目顺序。调用方不得在文档中放入 Cookie、token、`X-TuneWeave-Credential`、账户别名、密码、验证码、临时媒体 URL、签名或任意请求头。元数据快照用于展示和严格匹配，播放时仍会重新检查平台资源与账户权益。
