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
    { "platform": "soda", "type": "favorite_tracks", "id": "<汽水账户UID>", "account": "personal" },
    { "platform": "bilibili", "type": "season", "id": "3629748" },
    { "platform": "bilibili", "type": "favorite_folder", "id": "2883236382", "account": "default" }
  ]
}
```

公开集合不需要账户。私有或账户可见集合可以为每个来源单独指定 `account`。来源按请求顺序展开，来源内部顺序和重复项目都会保留；任一来源失败时不会创建部分歌单。导入结果的 `sources` 会返回来源名称与 `cover_url`，`favorite_tracks` 使用支持平台的真实“喜欢”歌单元数据，当前包括网易云、QQ、汽水、咪咕和酷狗。汽水该来源的输入 `id` 是选中账户的 UID，返回元数据中的 ID 是目录实际指向的喜欢歌单 ID；当前仅支持所选账户本人。

汽水支持 `{"ref":"soda:<歌单ID>","type":"playlist","account":"personal"}` 账户歌单来源。使用调用方托管会话时，省略该来源的 `account`，通过 `X-TuneWeave-Credential` 请求头提供汽水凭证。这同时适用于服务器导入和 `/v1/uni/materialize/imports`；轮换后的会话由 `X-TuneWeave-Updated-Credential` 返回，不保存到 Uni 文档中。若提供者在歌单元数据中返回 `source_snapshot_id`，所有曲目页必须带相同版本；版本变化或缺失会终止整个导入，不创建部分歌单。汽水账户歌单每次元数据或曲目页调用都会进行有界的完整读取，较大歌单可能较慢，边界及验收状态见[登录与凭证](authentication.md)。

Provider 可以支持不同的 `type`。常用值包括 `playlist`、`favorite_tracks`、`season` 和 `favorite_folder`；请通过 `/v1/capabilities` 确认目标平台能力。

酷我的公开“最新”与“最热”精选歌单可通过 `GET /v1/playlists?platform=kuwo&catalog=latest|hot` 浏览。目录返回的 `kuwo:{id}` 可作为 `{ "ref": "kuwo:{id}", "type": "playlist" }` 来源用于 Server 导入或 Client materialize；曲目顺序和重复项由现有来源读取链保留。该目录只代表官网精选结果，不是酷我全部歌单，也不表示歌曲当前可播放或可下载。

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

酷我 FM 可用 `{"ref":"kuwo:fm:359","kind":"radio_station"}` 作为条目输入，频道编号应从当前广播目录选择。支持 Client 标准化及 Server 追加条目；不指定酷我账户。保存的是电台身份和展示资料，直播 URL 不写入快照，每次播放重新读取电台详情。使用 `quality=auto`，返回的 `transport=live_radio` 表示直播地址解析，实际码率、格式和时长未知时保持空；不把 FM 当作普通歌曲或有独立媒体的节目队列。该能力已接入元数据和 URL 解析，实际直播播放仍待验收。

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

Client 请求也可以使用重复的 `X-TuneWeave-Credential` 请求头，为每个平台提供调用方托管凭证。 条目生成 `/v1/uni/materialize/items`、服务器追加条目 `/v1/uni/playlists/{ref}/items` 和条目播放均使用指定的调用方来源；同一平台不能同时在 `accounts` 中选择服务器别名。会话轮换通过 `X-TuneWeave-Updated-Credential: 平台=完整凭证` 响应头返回，不保存到条目、导出文档或服务器账户中。更新头与媒体 `headers` 的使用规则见[登录与凭证](authentication.md)。

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


咪咕普通歌单与喜欢集合支持账户来源：普通集合使用 `{"ref":"migu:歌单ID","type":"playlist","account":"别名"}`，喜欢集合使用 `{"ref":"migu:本人UID","type":"favorite_tracks","account":"别名"}`。调用方持有凭据时省略 `account`，通过统一凭据请求头提交。两者均可用于 `/v1/uni/playlists/imports` 和 `/v1/uni/materialize/imports`。喜欢来源要求所选账户的个人导航提供稳定的歌单 ID；若上游没有提供，接口返回 `capability_not_supported`，不会从同名自建歌单推断。

咪咕单曲购买库支持 `{"ref":"migu:本人UID","type":"purchased_tracks","account":"别名"}`。来源 UID 必须与所选账户一致；服务器复用已购单曲的完整有序读取和 `source_snapshot_id`，每个项目必须有已解析的公开曲目资料，未解析购买记录会使整次 Uni 导入失败而不会被静默丢弃。购买记录本身仍不授予当前播放或下载权限；已购专辑另用 `purchased_albums` 来源展开，见下文。

咪咕每次详情或曲目窗口都先完整读取，单个歌单最多 10,000 首；大集合的多窗口导入有重复读取成本。`source_snapshot_id` 用于比较所读内容，后续页内容变化、缺少完整性证明或会话失效会使导入失败，保留歌曲顺序及重复条目。该内容标识不等于上游事务版本，也不授予播放权；账户播放和普通非加密歌曲的独立下载授权见[账户媒体说明](authentication.md#咪咕账户歌曲与播放授权)，真实账户兼容性与权益仍待验收。


酷狗原生标准版/概念版账户的普通歌单使用 `{"ref":"kugou:cloudlist:本人UID:类型:本地歌单ID","type":"playlist","account":"别名"}`，类型为自建 `0` 或收藏 `1`。喜欢集合使用 `{"ref":"kugou:本人UID","type":"favorite_tracks","account":"别名"}`，从本人库中定位真实喜欢歌单；调用方凭据模式省略 `account`。两类来源支持持久导入和临时 materialize，保留原始顺序及重复歌曲，跨页内容版本变化时不创建部分歌单。每次分页都完整读取来源，较大集合请求成本较高；此版本检测不提供上游事务隔离，也不代表账户播放权益已验证。Web Cookie 暂不支持这些酷狗账户来源。

酷狗旧版 Web“云收藏”只支持以原样目录引用导入单个普通歌曲来源：`{"ref":"kugou:legacy_web_collection:本人UID:opaque目录ID","type":"playlist","account":"别名"}`。调用方凭据模式省略 `account`。此来源支持持久导入和 Client materialize，保留歌曲顺序及重复 occurrence；每一项都必须有完整 canonical 歌曲引用 `kugou:{album_audio_id}`。无法解析或确认 ID 的 occurrence 会使整次来源失败，不静默过滤或缩短清单。Web type=17 没有远端版本号；由一次完整响应生成的 `source_snapshot_id` 只用于核对元数据和各曲目页之间的跨调用内容一致性。检测到不一致时 Server 不创建部分 Uni 歌单，Client 也不返回部分 materialize 结果。该指纹不提供上游事务快照，也不代表播放、试听或下载权益；旧版 Web 来源不支持 `favorite_tracks`、`collected_albums` 等分类或写入。实际旧版 PHP 会话兼容性仍待最终真实账户验收。

酷狗原生标准版／概念版已购歌曲支持 `{"ref":"kugou:本人UID","type":"purchased_tracks","account":"别名"}`，调用方凭据模式省略 `account`。来源 UID 必须属于所选账户；每次元数据或分页都比较两遍完整购买目录，保留购买记录顺序和不同记录指向同一歌曲的重复项。每遍最多 128 页、每页 50 项；跨调用使用 `source_snapshot_id` 检查一致性。任意购买记录缺少明确的曲目身份或标题时，整项 Uni 来源失败，不静默过滤；普通购买记录 API 仍可返回未解析记录。购买记录不授予当前播放或下载权益。当前不支持 Web Cookie；已购专辑使用下述 `purchased_albums`。

酷狗已购专辑支持 `{"ref":"kugou:本人UID","type":"purchased_albums","account":"别名"}`，同样适用于持久导入和 Client materialize。先完整读取所选账户的专辑购买记录，再按明确的 `album_id` 读取匿名公开专辑歌曲，最后再次读取原账户购买目录；前后目录变化、缺少专辑映射或任何目录读取失败都会使整项来源失败。每次公共目录请求也检查原登录代际，但不向匿名目录发送账户凭据。

专辑来源保留购买记录顺序；不同购买记录指向同一专辑时，各自展开完整歌曲，同次操作只读取一次该专辑。歌曲重复保留，不按 `buy_total` 复制；不推断数字专辑的赠品、视频或专题内容。标准版使用官方自定义购买目录排序，概念版保留其接口返回顺序。一次最多 128 条专辑购买记录、10,000 个展开曲目，购买与专辑目录的响应正文合计最多 16 MiB，总期限 120 秒。`source_snapshot_id` 覆盖购买记录与有序歌曲，供跨分页调用核对；不承诺上游事务快照，也不证明播放或下载权益。仅支持原生标准版／概念版，真实账户导入仍待最终验收。

酷我普通自建及收藏歌单支持所选服务器账户或调用方原生凭据导入。来源读取先独立认证、确认本人目录，并比较两遍完整内容和目录复核结果；跨调用的内容摘要不同则拒绝导入。顺序和重复歌曲保留，凭据不进入 Uni 文档；导入能力不等于酷我账户播放授权。同一歌单同时自建和收藏时优先自建；节目等非歌曲条目明确拒绝，不能过滤后冒称完整导入。喜欢歌曲也支持 `favorite_tracks` 来源，引用使用所选账户本人 UID，返回的源歌单引用为实际喜欢云歌单 ID；不把两者混用。更多账户来源和媒体授权继续扩展，真实账户留待最终验收。

酷我账户的普通自建歌单详情及曲目来源摘要包含两次核实的完整可编辑资料，因此标签单独变化也会使跨调用导入失败。已核实来源在导入响应及服务器 `import_sources` 的来源扩展中提供 `source_tags` 和 `source_metadata_verified=true`；显式空标签保留为空数组，未知标签不附加该标记。Client materialize 保留这些来源信息并移除账户别名。此扩展不改变 v1 文档的字段白名单，v1 文档仍不包含完整 `import_sources`。收藏与喜欢来源不假定具备同样的详情协议。


汽水收藏专辑支持 `{"ref":"soda:本人UID","type":"collected_albums","account":"personal"}`；咪咕已购专辑支持 `{"ref":"migu:本人UID","type":"purchased_albums","account":"personal"}`。两者均可用于持久导入和 Client materialize，调用方凭据模式省略 `account` 并提供凭据请求头。按目录顺序展开每张专辑的完整歌曲，保留重复歌曲；咪咕普通与数字专辑独立识别，不因数字 ID 相同而合并。

这两种来源每次元数据或曲目窗口调用都完整展开，最多 128 个专辑条目、10000 首歌曲，单次总期限 120 秒。前后完整读取账户目录，目录变化、无法证明完整、专辑不可读或会话变化时整次失败；跨调用使用内容快照核对，不提供上游事务隔离。大集合的多窗口导入有重复读取成本。收藏与购买只说明来源，实际播放和下载仍分别校验当前权益；真实账户验收待完成。

咪咕收藏专辑支持 `{"ref":"migu:本人UID","type":"favorite_albums","account":"别名"}`，可用于 `/v1/uni/playlists/imports` 和 `/v1/uni/materialize/imports`；调用方凭据模式省略 `account` 并提交咪咕凭据。服务按目录原序展开普通专辑（`resource_type=2003`）和数字专辑（`resource_type=5`），相同数值 ID 按资源类型区分，不合并；逐张读取完整曲目并保留专辑顺序及重复歌曲。收藏目录每页 10 项、最多 64 页；每张专辑曲目读取最多 64 页。读取前后完整核验收藏目录、本人 UID 和账户会话代际；目录漂移、不完整目录／专辑或会话变化都会使整次来源失败，不创建或返回部分结果。单次最多 128 张专辑、10,000 首歌曲、120 秒和 16 MiB 序列化快照材料。内容摘要只用于跨调用一致性检查，不是上游原子快照，也不代表播放或下载权益；真实账户验证仍待完成。

汽水已购数字专辑也支持 `{"ref":"soda:本人UID","type":"purchased_albums","account":"personal"}`，可用于持久导入和 Client materialize；调用方凭据模式省略 `account` 并提交汽水凭据。服务器按本人数字专辑目录顺序读取各专辑完整曲目，保留不同专辑间重复的歌曲；每次元数据或曲目窗口调用都会重新读取并比较完整目录，最多展开 128 张专辑和 10000 首曲目，总期限 120 秒。购买目录变化、专辑不可读或账户会话变化时整次失败。来源摘要用于跨调用一致性检查，但不提供上游事务快照；已购目录不代表当前播放或下载权益，返回曲目的可播放状态保持未知。
