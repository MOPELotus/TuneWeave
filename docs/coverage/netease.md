# 网易云 API 全量覆盖账本

上游快照：`NeteaseCloudMusicApiEnhanced/api-enhanced@35d1c61cb4dccd1c55c25bf791a915cd29f7fedf`

本表由该快照的 `module/*.js` 文件生成，共 416 项。它是完成度验收清单，不是功能推荐列表；实际实施优先级见 [`docs/implementation-plan.md`](../implementation-plan.md)，网易云 Basic 的独立聚合进度见 [`netease-basic.md`](netease-basic.md)。状态含义：

- `pending`：尚未完成统一映射或平台扩展端点。
- `partial`：已有一部分统一能力，但仍缺输入、输出、分支或真实验证。
- `implemented`：代码和离线测试已完成，仍需要带真实前置条件的联网验证。
- `verified`：统一端点、测试和对应真实网络路径均已验证。

当前统计：`pending=226`、`partial=2`、`implemented=64`、`verified=124`。只有所有条目都达到 `verified`，或以证据明确标为上游已失效，网易云阶段才算完成。

| 上游模块 | 参考路由 | 状态 | TuneWeave 映射/缺口 |
| --- | --- | --- | --- |
| `activate_init_profile` | `/activate/init/profile` | `pending` | — |
| `ad_get` | `/ad/get` | `implemented` | `GET /v1/listening-rights/ads`（以独立 `ListeningRightsAds` 能力和稳定 `ListeningRightsAdCatalog` 表达广告换听目录；`type_ids` 缺省 `400002_0`，兼容逗号列表、`typeIds` 与参考 JSON 字符串数组，保留顺序/重复项并限制 1–100 个非空值；固定调用带自动实时 checkToken 的 XEAPI `/api/ad/get`，精确把类型数组序列化为 `type_ids` JSON 字符串；兼容对象/数组/空广告包装，逐项解析字符串或对象 `extJson.contextInfo.req_id`，空/畸形前项不遮蔽后续有效请求 ID，原广告、可解析 extJson、类型和完整响应均保留；核心类型、能力、默认/多类型协议、空/非空/数组包装、无效 extJson、消息优先级、畸形容器、类型边界、账户选择、统一/参考查询和未知字段均有测试；2026-07-18 匿名真实 XEAPI 请求自动注册 checkToken 并返回上游 `code=200` 的合法空投放，待持久化账户验证非空广告及真实 `req_id`） |
| `ad_listening_rights_gain` | `/ad/listening/rights/gain` | `implemented` | `GET/POST /v1/listening-rights/gains`（独立 `ListeningRightsGain` 能力和稳定请求/结果模型；GET 完整兼容参考 `reqUid/creativeType/exposureTime/clickTime/rightsGainMethod`、四个可选时长/方式、`source/rightsExtJson/appInfo/installed/type_ids`，POST 提供 snake_case JSON 并接受 camelCase 别名；缺省创意类型/领取方式均为 2，缺省曝光/点击时间取同一当前毫秒值，参考 GET 的显式时间字符串保持字符串，所有可选整数、应用 JSON 和扩展文本进入精确 `reqParam` JSON 字符串；未给 `reqUid` 时先以同账户和类型调用广告目录，目录失败或无投放按参考行为继续提交空 ID，不伪造请求 ID；固定以 v3 checkToken 调用 XEAPI `/api/ad/listening/rights/gain`，只把明确布尔或 0/1 `gainFlag` 映射为可空 `granted`，其他值不猜测，完整请求/响应及 ID 来源保留；核心契约、完整/缺省载荷、字符串/数字时间、自动 ID 来源、未知 flag、边界、账户、GET/POST 和输入拒绝均有测试；2026-07-18 匿名真实自动广告 + v3 XEAPI 链返回上游登录边界 `code=2001` 并稳定映射 401，待持久化真实账户验证成功领取） |
| `aidj_content_rcmd` | `/aidj/content/rcmd` | `pending` | — |
| `album` | `/album` | `verified` | `GET /v1/albums/{ref}`、`GET /v1/albums/{ref}/tracks`（2026-07-16 HTTP 实测 `netease:18915` 返回《范特西》及 10 首曲目） |
| `album_detail` | `/album/detail` | `verified` | `GET /v1/digital-albums/{ref}`（与 `/digitalAlbum/detail` 共用上游协议；`artistNames` 多艺人名称优先于 `artistName` 单艺人摘要；2026-07-16 HTTP 实测 `netease:120605500` 返回《冀西南林路行》及 22 CNY 商品信息） |
| `album_detail_dynamic` | `/album/detail/dynamic` | `verified` | `GET /v1/albums/{ref}/stats`（2026-07-16 HTTP 实测 `netease:32311` 返回收藏状态、71671 收藏、1989 评论及 9306 分享） |
| `album_list` | `/album/list` | `verified` | `GET /v1/digital-albums`（`area/type` 筛选；2026-07-16 HTTP 实测返回 2 项、首项 `netease:387169747`《小海子村儿》，上游未给总数故 `total=null`） |
| `album_list_style` | `/album/list/style` | `verified` | `GET /v1/digital-albums?catalog=style`（`ZH/EA` 统一区域值映射到上游 `Z_H/E_A`；2026-07-16 HTTP 实测返回 2 项并保留销量与购买状态） |
| `album_new` | `/album/new` | `verified` | `GET /v1/albums?catalog=new`（`area` 筛选；2026-07-16 匿名 HTTP 实测返回 2 项、总数 500） |
| `album_newest` | `/album/newest` | `verified` | `GET /v1/albums?catalog=newest`（2026-07-16 匿名 HTTP 实测首页共 12 项，统一分页返回前 2 项） |
| `album_privilege` | `/album/privilege` | `verified` | `GET /v1/albums/{ref}/track-entitlements`（可用音质固定按能力升序去重，192/320 kbps 不混档，零 `playMaxbr` 不遮住有效 `maxbr`；2026-07-16 匿名 HTTP 实测 `netease:168223858` 共 10 项，首项 `netease:2058263030` 可播 320 kbps、最高 999 kbps，并保留无损及 Hi-Res 权益） |
| `album_songsaleboard` | `/album/songsaleboard` | `verified` | `GET /v1/charts/digital-albums`（完整支持 `daily/week/year/total` 与数字专辑/数字单曲；2026-07-16 HTTP 实测 2025 年数字单曲榜共 10 项，首项 `netease:83848829`《好想爱这个世界啊》，销量 316218） |
| `album_sub` | `/album/sub` | `implemented` | `PUT/DELETE /v1/account/library/albums/{ref}`（收藏与取消收藏路径均已实现；2026-07-16 匿名 HTTP 实测正确映射为 401 `authentication_required`，待真实账户验证成功写入） |
| `album_sublist` | `/album/sublist` | `verified` | `GET /v1/account/library/albums`（分页、收藏时间与 `paidCount` 等元数据已完整映射；2026-07-17 持久化真实账户 HTTP 实测返回 5 项） |
| `api` | `/api` | `verified` | `POST /v1/extensions/netease/api`（仅允许固定网易云域名与 `/api/...` 路径，登录态由 `account` 别名选择；完整支持默认 EAPI 及 `weapi/api/linuxapi/xeapi`，拒绝原始 Cookie、域名、代理和请求头覆盖；2026-07-16 五种协议均以搜索请求联网实测成功，另实测 XEAPI 公钥注册/加密响应及 `e_r=true` EAPI 响应解密） |
| `artist_album` | `/artist/album` | `verified` | `GET /v1/artists/{ref}/albums`（统一分页并保留歌手级元数据与单项原始字段；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 5 张周杰伦专辑，首项 `netease:274336916`《即兴曲》，`has_more=true`、`next_offset=5`） |
| `artist_desc` | `/artist/desc` | `verified` | `GET /v1/artists/{ref}`（与 `/artist/detail` 合并为统一 `Artist`，映射简介及分段传记，并在扩展字段保留专题原始响应；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回周杰伦简介、6 段传记及 3 项专题数据） |
| `artist_detail` | `/artist/detail` | `verified` | `GET /v1/artists/{ref}`（映射名称、别名、身份、头像、封面及作品计数，并保留完整原始响应；2026-07-16 匿名 HTTP 实测返回 44 张专辑、568 首曲目、9 个 MV 与 8 个视频） |
| `artist_detail_dynamic` | `/artist/detail/dynamic` | `verified` | `GET /v1/artists/{ref}/stats`（统一关注态、视频分类计数与在线演出计数，未知平台类别保留原始标识，演出及推荐对象保留完整响应；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 `followed=false`、分类 `0:9/1:9`、在线演出数 0） |
| `artist_fans` | `/artist/fans` | `verified` | `GET /v1/artists/{ref}/fans`（统一为分页 `User[]`，昵称、头像、签名和关系状态进入稳定字段，地区、认证、VIP 等完整资料保留在单项扩展；2026-07-16 匿名 HTTP 实测 `netease:2116` 返回 2 位粉丝、`next_offset=2`、`has_more=true`，上游无总数故 `total=null`） |
| `artist_follow_count` | `/artist/follow/count` | `verified` | `GET /v1/artists/{ref}/stats`（统一粉丝总数和账户关注态，日增量等附加字段保留完整响应；2026-07-16 匿名 HTTP 实测 `netease:2116` 返回 `follower_count=13704933`、`followed=false`，统一值与上游 `fansCnt/isFollow` 一致） |
| `artist_list` | `/artist/list` | `verified` | `GET /v1/artists`（统一 `type=all/male/female/group`、六类 `area` 与 `initial=a..z/hot/other`，条目映射为 `Artist` 并保留完整目录字段；2026-07-16 匿名 HTTP 实测 `type=male&area=western&initial=b&limit=2` 返回 Bruno Mars 与 bbno$，首项 50 张专辑/959 首歌曲，`next_offset=2`、`has_more=true`） |
| `artist_mv` | `/artist/mv` | `verified` | `GET /v1/artists/{ref}/videos?type=mv`（统一为分页 `Video[]`，优先保留非空完整 `artists[]` 而不被单个 `artist` 摘要覆盖；完整数组只有空白项时会回退有效单项创作者，空白 16:9 封面也会回退普通封面，并映射时长、发布日期、播放数和收藏态；完整 MV 与响应时间保留；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 2 项，首项 `netease:22695250`《任性 (5525 Live版)》、266000 ms、100726 播放，`next_offset=2`、`has_more=true`） |
| `artist_new_mv` | `/artist/new/mv` | `verified` | `GET /v1/account/following/artists/new-videos`（以 `platform/account` 选择登录态，`before` 毫秒时间戳翻页，统一映射为 `Video[]`；专用 `mvId/mvName/mvCoverUrl` 优先于作品包装摘要，空完整描述再回退简述，完整响应不丢失；2026-07-17 持久化真实账户 HTTP 实测返回 1 项） |
| `artist_new_song` | `/artist/new/song` | `verified` | `GET /v1/account/following/artists/new-tracks`（以 `platform/account` 选择登录态，`before` 以作品块翻页；兼容当前上游 `songLists` 分组并按块顺序展开为 `Track[]`，保留新曲总数、作品块计数及完整原文；2026-07-17 持久化真实账户 HTTP 实测 5 个作品块展开为 7 首歌曲） |
| `artist_new_song_mv_list_v2` | `/artist/new/song/mv/list/v2` | `verified` | `GET /v1/account/following/artists/new-works`（完整支持 `before/source_type/first_request`，以 `ArtistWorkUpdate` 区分歌曲、MV、两者并存及未知来源；实际非空资源优先于类型提示，空的旧别名数组不会遮住非空新字段，未知结构完整保留；2026-07-17 修正 EAPI 登录 Cookie 编码后，持久化真实账户 HTTP 实测展开返回 6 项） |
| `artist_new_song_playall` | `/artist/new/song/playall` | `verified` | `GET /v1/account/following/artists/new-tracks/play-all`（固定返回最近至多 50 首 `Track[]` 和上游 `count`，完整歌曲字段保留在扩展；2026-07-17 修正 EAPI 登录 Cookie 编码后，持久化真实账户 HTTP 实测返回 50 项） |
| `artist_songs` | `/artist/songs` | `verified` | `GET /v1/artists/{ref}/tracks`（完整支持 `order=hot/time`、分页及 `account` 登录态选择，统一为 `Track[]` 并在 `extensions.artist_track` 保留完整歌曲原文；2026-07-16 真实上游与统一 HTTP 均实测成功，`/v1/artists/netease:6452/tracks?order=time&limit=2` 首项为 `netease:2712553851`《即兴曲》，总数 566、`next_offset=2`、`has_more=true`） |
| `artist_sub` | `/artist/sub` | `implemented` | `PUT/DELETE /v1/account/following/artists/{ref}`（关注与取消关注共用统一 `SubscriptionResult`，登录态由引用平台和 `account` 别名选择，完整上游响应保留在扩展；请求构造、成功态映射、非法 ID、账户别名及 HTTP 端点测试已完成；2026-07-16 匿名 WeAPI HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功写入及回滚） |
| `artist_sublist` | `/artist/sublist` | `verified` | `GET /v1/account/following/artists`（统一为分页 `Artist[]`，支持 `platform/account/limit/offset`，名称、别名、封面及作品计数进入稳定字段，关注时间和完整歌手原文保留在 `extensions.following_item`；2026-07-17 持久化真实账户 HTTP 实测返回 5 项） |
| `artist_top_song` | `/artist/top/song` | `verified` | `GET /v1/artists/{ref}/top-tracks`（固定热门 50 首快照，不接收伪分页参数；歌曲与独立权益按 ID 合并为统一 `Track[]`，`has_more=false`，单项原文和完整响应均保留；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 50 首，首项 `netease:210049`《布拉格广场》（蔡依林 / 周杰伦），`total=50`、`next_offset=null`） |
| `artist_video` | `/artist/video` | `verified` | `GET /v1/artists/{ref}/videos?type=all`（统一为游标分页 `Video[]`，实际 `mlogBaseData.id` 优先于外层记录摘要 ID，并映射标题、创作者、封面、时长、发布时间和播放数，原始 Mlog 资源完整保留；2026-07-16 匿名 HTTP 实测 `netease:2116` 连续两页各返回 2 项，游标由 `2` 前进至 `4` 且资源无重复，首项 `netease:34702399`《K歌之王 AIR (Day Version / Lyric Video / China Version)》） |
| `artists` | `/artists` | `verified` | `GET /v1/artists/{ref}/overview`（统一为 `ArtistOverview`，明确分离歌手摘要、50 首精选 `Track[]` 和 `has_more_tracks`，不与 `/artist/list` 或完整曲目目录误合并；歌手、曲目及完整响应原文分别保留；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回周杰伦、568 首总曲目计数、50 首精选，首项 `netease:210049`《布拉格广场》，`has_more_tracks=true`） |
| `audio_match` | `/audio/match` | `implemented` | `POST /v1/audio/recognize`（统一 `platform/account/fingerprint/duration_seconds` 输入，兼容参考项目 `audioFP/duration` 字段；多候选曲目、命中起点、查询 ID、无匹配原因和完整上游响应均已映射，`startTime` 无效时继续读取可解析的 `start_time`；离线成功命中样本、冲突别名、输入边界及 HTTP 端点测试已完成；2026-07-16 匿名 HTTP 实测无匹配路径返回 `code=200`、空 `matches`、`no_match_reason=10` 与真实查询 ID，待有效音频指纹验证真实成功命中） |
| `avatar_upload` | `/avatar/upload` | `implemented` | `PUT /v1/account/avatar`（统一以 `platform/account/filename` 查询参数、`Content-Type: image/*` 和原始图片请求体写入，最大 20 MiB；完整实现 WeAPI 申请 `yyimgs` NOS 凭据、原始字节上传及 EAPI 提交 `imgId` 三段流程，统一返回 URL/图片 ID，空首选 URL 会回退后续有效字段，NOS token 不进入响应或日志；兼容 `imgSize/imgX/imgY` 参数并明确记录参考实现未实际应用裁剪；离线映射、冲突 URL、认证前置、参数别名、大小边界、标准错误包络及 token 防泄漏测试已完成；2026-07-16 匿名 HTTP 实测在 NOS 分配前稳定返回 401 `authentication_required`，待真实账户验证最终写入） |
| `banner` | `/banner` | `verified` | `GET /v1/banners`（完整支持 `client=pc/android/iphone/ipad`，并兼容参考项目 `type=0/1/2/3`；同时存在时优先使用非空大图 `bigImageUrl` 与主标题 `mainTitle`，空白值不会遮住普通图片/类型标题；横幅 ID、跳转 URL、独家标志及歌曲/专辑/歌手/歌单/MV/网页/未知目标进入稳定字段，监测和广告等完整原文保留在 `extensions.banner`；2026-07-16 适配器与统一 HTTP 均逐分支联网实测成功，PC 7 项、Android 8 项、iPhone 8 项、iPad 6 项，首项目标分别正确映射为网页或 `netease:384808686` 专辑） |
| `batch` | `/batch` | `verified` | `GET/POST /v1/extensions/netease/batch`（完整保留任意 `/api/...` 子请求及逐项原始响应，支持参考 GET 查询键、POST 顶层动态键和 `requests` 结构化容器；对象值自动序列化为上游真实要求的 JSON 文本，预序列化字符串原样保留；完整支持 `eapi/weapi/api/linuxapi/xeapi`、`crypto/protocol`、`e_r/encrypted_response` 与 `account`，逐路径限制固定网易云域名并拒绝 Cookie、域名、代理、请求头和 IP 等传输注入；2026-07-16 适配器及统一 HTTP 对五种协议均联网实测顶层/子请求 `code=200`，每种取得 7 条横幅，参考 GET 形态加 `e_r=true` 亦成功解密并返回 7 条，不存在的账户别名实测为 401） |
| `broadcast_category_region_get` | `/broadcast/category/region/get` | `verified` | `GET /v1/radio/taxonomy`（统一为 `RadioTaxonomy`，分类与地区 ID 均保持平台不透明字符串，单项及完整响应原文保留在扩展中，可直接供后续广播电台列表筛选；支持 `platform/account` 选择且公开响应无需登录；2026-07-16 适配器与统一 HTTP 均联网实测成功，返回 12 个分类和 32 个地区，首项分别为 `1`“音乐台”与 `407`“网络台”，原始上游 `code=200`） |
| `broadcast_channel_collect_list` | `/broadcast/channel/collect/list` | `verified` | `GET /v1/account/library/radio-stations`（以 `platform/account` 选择登录态，完整提交参考实现的 `contentType/timeReverseOrder/startDate/limit`，并补齐参考接口声明的 `offset` 分页；统一为 `RadioStation[]`，兼容对象及 JSON 字符串嵌套条目，空的旧列表、空对象/JSON 包装和 `null` 分页别名不会遮住后续有效结构；收藏项、频道原文和完整分页响应分别保留在扩展中；2026-07-17 持久化真实账户 HTTP 实测成功返回空收藏列表） |
| `broadcast_channel_currentinfo` | `/broadcast/channel/currentinfo` | `verified` | `GET /v1/radio/stations/{ref}`（以资源引用选择平台、`account` 选择可选登录态，统一为 `RadioStation`；名称、封面、地区、当前节目与直播音频地址进入稳定字段，第三方频道/节目 ID、时间窗口及完整响应保留在扩展中，公开响应未给收藏态时严格保持 `null`；无符号整数 ID 在网络请求前校验；2026-07-16 provider 与统一 HTTP 均联网实测 `netease:362` 成功，返回“金山区广播电视台综合广播”、地区“上海”、可用的 `https://lhttp.qtfm.cn/live/4022/64k.mp3...` 音频地址及上游 `code=200`） |
| `broadcast_channel_list` | `/broadcast/channel/list` | `verified` | `GET /v1/radio/stations`（完整支持 `categoryId/regionId/limit/lastId/score` 及 snake_case 别名；分类、地区和电台 ID 保持字符串，`lastId+score` 统一为成对游标并在分页扩展返回 `next_cursor`，两字段独立出现时分别补参考默认 `0/-1`；参考类型公开但实现忽略的 `offset` 仍被接收，并明确返回 `requested_offset` 与 `offset_applied=false`；首屏推荐插入导致返回数大于 `limit` 时不截断，完整频道和响应原文保留；2026-07-16 provider 与统一 HTTP 均联网实测：音乐分类 `categoryId=1` 首/二页各 20 项、总数 105、两页零重复，首屏下一游标 `{id:965,score:1139}`；网络台 `regionId=407` 返回 4/4 项且全部地区为“网络台”、`has_more=false`；`offset=100` 实测不改变上游首屏并正确标记未应用，上游均为 `code=200`） |
| `broadcast_sub` | `/broadcast/sub` | `implemented` | `PUT/DELETE /v1/account/library/radio-stations/{ref}`（参考 `t=1` 的收藏分支完整映射为 `contentType=BROADCAST`、`cancelCollect=false`，其余 `t` 值的取消分支映射为 `cancelCollect=true`；统一端点以 HTTP 方法明确表达两种语义，电台 ID 在网络前校验，`account` 选择隔离登录态，统一返回 `SubscriptionResult` 并保留完整上游响应；离线请求构造、成功响应映射、非法 ID、缺失账户别名、PUT/DELETE 路由与响应契约均已测试；2026-07-16 provider 和统一 HTTP 对收藏/取消两条匿名路径均联网实测为 401 `authentication_required` 与上游码 301，待真实账户验证成功写入） |
| `calendar` | `/calendar` | `implemented` | `GET /v1/extensions/netease/calendar`（固定使用 WeAPI 调用 `/api/mcalendar/detail`；完整支持参考 `startTime/endTime`、统一 `start_time/end_time`、任一或全部时间参数缺省时取当前 Unix 毫秒，以及 `account` 登录态别名；显式值归一化为无符号整数，非法或负数在上游请求前返回标准 400；完整上游 JSON 保留在统一包络中；离线测试已覆盖参考/统一参数名、账户隔离、协议与路径、双缺省和非法值；2026-07-16 匿名统一 HTTP 实测正确映射为 401 `authentication_required` 与上游码 301，待真实账户验证成功内容态） |
| `captcha_sent` | `/captcha/sent` | `implemented` | `POST /v1/auth/challenges`（为避免误发短信不做自动联网测试） |
| `captcha_verify` | `/captcha/verify` | `implemented` | `POST /v1/auth/challenges/validate`（与验证码登录事务明确分离，仅校验而不登录或保存 Cookie；统一支持 `platform/account/method/principal/code/country_code`，并完整兼容参考 `phone/captcha/ctcode`，手机号和区号接受数字或字符串，`method` 默认 SMS、区号缺省或空值默认 `86`；统一返回 `valid/platform_code/message` 并在 `extensions.response` 保留完整上游响应，空白 `message` 会回退有效 `msg`，错误验证码作为 HTTP 200 的 `valid=false` 正常结果，手机号和验证码不回显；core、适配器冲突映射、参考/统一输入、默认值、非法值及敏感数据边界均有离线测试；2026-07-16 provider 与统一 HTTP 以假验证码真实联网验证成功，返回 `valid=false`、平台码 503 和“验证码错误”，待真实验证码验证 `valid=true` 分支） |
| `cellphone_existence_check` | `/cellphone/existence/check` | `verified` | `POST /v1/auth/principals/status`（统一为不创建会话的 `AuthPrincipalStatus`，网易云限定 `principal_type=phone` 并默认该类型；完整支持统一 `platform/account/principal_type/principal/country_code`、参考 `phone/countrycode`、camelCase `countryCode`，手机号和区号接受数字或字符串，区号缺省或空值默认 `86`；固定 EAPI 调用 `/api/cellphone/existence/check`，严格把已实测 `exist=1/-1` 映射为 `exists=true/false`，统一保留 `has_password/display_name/avatar_url/platform_code` 及 `extensions.response`，不把未知 `exist` 值猜成布尔值，输入请求和 Debug 均脱敏；core、适配器两分支、能力声明、参考/统一输入、默认值、非手机号与非标量拒绝、敏感数据边界均有测试；2026-07-16 provider 与统一 HTTP 真实联网验证已注册 `13800138000` 返回 `exists=true`、`has_password=true`、上游 `exist=1`，未注册输入 `1` 返回 `exists=false`、`has_password=false`、上游 `exist=-1`，两者平台码均 200 且手机号保持上游脱敏） |
| `chart_detail` | `/chart/detail` | `verified` | `GET /v1/charts/dimensions/{chart_code}`（完整支持必填 `target_id/target_type`、参考 `targetId/targetType` 别名及 `platform/account`；统一为 `DimensionChart`，映射榜单引用、维度、名称、说明、封面、更新时间、计数和评论支持态，完整响应保留在扩展；2026-07-16 provider 与匿名 HTTP 均真实联网验证 `CITY_SONG_CHART + 110000 + CITY` 成功，返回 `netease:CITY_SONG_CHART#110000@CITY#`“北京榜”、上游 `code=200`） |
| `chart_song_detail` | `/chart/song/detail` | `verified` | `GET /v1/charts/dimensions/{chart_code}/tracks`（统一为不可分页的 `DimensionChartTrackSnapshot`；列表顺序映射为从 1 开始的当前排名，保留上期排名、升降、理由、理由 ID、分数、比例、收藏态、分组、平台权益与每项/整份原始响应，不伪造 `limit/offset`；2026-07-16 provider 与匿名 HTTP 真实联网验证 `CITY_STYLE_SONG_CHART + 110000_1020 + CITY_STYLE` 返回完整 100 项、无分页元数据，首项 `netease:3399839173`《甲乙丙丁 (你我怎么两清)》当前/上期均第 1、可播放，上游 `code=200`） |
| `check_music` | `/check/music` | `verified` | `GET /v1/tracks/{ref}/availability`（统一为 `TrackAvailability`，引用决定平台，`account` 选择登录态；完整支持统一 `bitrate`、参考 `br` 及缺省 999000 bit/s，固定 WeAPI `/api/song/enhance/player/url`，严格按参考实现以单项 `code=200` 判定可播，不可播是 HTTP 200 的正常布尔结果；返回请求/实际码率、平台码和兼容消息，保留费用、音质等诊断但清除临时播放 URL，避免绕过统一流解析；2026-07-16 provider 与匿名 HTTP 真实联网验证：`netease:1969519579` 默认请求可播、实际 320000，`br=128000` 实际 128000；`netease:1` 返回 `playable=false/platform_code=404`，三次上游顶层均为 200） |
| `cloud` | `/cloud` | `implemented` | `POST /v1/account/cloud/uploads`（以 `platform/account` 选择隔离登录态，统一接收必填安全文件名、可选 bit/s 码率与曲名/歌手/专辑查询参数，并以最大 500 MiB 的原始音频请求体替代平台 multipart 形态；完整执行 MD5 计算、音频标签解析、EAPI 上传检查、WeAPI NOS 凭据分配、受限 LBS 目标上传、EAPI 云盘信息登记及发布，文件已存在时跳过字节上传；显式元数据优先于标签，主标签缺少单个字段时按字段回退备用标签，再按参考逻辑回退到安全化文件主名和“未知艺术家/未知专辑”，兼容 `song/songName`；NOS token 不进入最终结果、扩展或 Debug，上传目标复用严格的 `*.127.net` 白名单与固定查询参数；有效 WAV 标签、双标签逐字段回退、MD5、元数据优先级/回退、文件和码率边界、认证前置、能力声明、二进制 HTTP 输入、500 MiB 参考上限及统一错误包络均有离线测试，服务器消费请求缓冲而不无条件复制整份音频；2026-07-17 持久化真实账户以唯一生成 MP3 实测需上传分支返回 200 并完成 NOS 写入、登记发布及精确删除回滚，待文件已存在、纯标签回退和上游失败分支补验） |
| `cloud_import` | `/cloud/import` | `implemented` | `POST /v1/account/cloud/imports`（以 `platform/account` 选择隔离登录态，统一接收 `md5/source_track_id/bitrate/file_size/file_type/song_name/artist/album`，并兼容参考字段 `id/fileSize/fileType/song` 及字符串化数字；TuneWeave 对外码率保持 bit/s，网易 provider 严格按参考文档执行 `floor(bit/s / 1000)` 后传入上游 kbps，缺省源曲目 ID 为 `-2`、歌手/专辑为“未知”；完整实现 EAPI `/api/cloud/upload/check/v2` 与 `/api/cloud/user/song/import` 两段事务，保留检查状态 0/1/2、已存在判定及两段原始响应，空/零首选结果 ID 会回退后续有效字段；协议 JSON 字符串、单位换算、默认值、文件/MD5/码率/来源 ID 边界、成功映射、冲突字段别名和错误包络均有离线测试；2026-07-17 持久化真实账户对刚发布音频的同 MD5 导入返回 200，并随测试资产完整回滚，待独立可导入与不可导入分支补验） |
| `cloud_lyric_get` | `/cloud/lyric/get` | `implemented` | `GET /v1/account/cloud/lyrics`（统一查询参数 `platform/account/user_id/track_id`，兼容参考 `uid/sid`；固定 EAPI `/api/cloud/lyric/get` 并完整提交 `lv=-1/kv=-1`，云盘歌曲 ID 按平台不透明字符串处理；结果复用统一 `Lyrics`，映射普通、翻译、罗马音、逐字歌词和贡献者，并在扩展保留用户 ID 与完整云盘响应；不透明 ID、参考载荷、统一歌词映射、字段别名、缺字段错误与认证前置均有离线测试；2026-07-16 匿名真实 HTTP 验证在歌词请求前稳定返回 401 `authentication_required`，待真实账户及含 `LYRICS` 标签的云盘文件验证内容成功态） |
| `cloud_match` | `/cloud/match` | `implemented` | `POST /v1/account/cloud/matches`（统一接收 `user_id/cloud_track_id/target_track_id`，兼容参考 `uid/sid/asid` 和字符串或数字 ID；固定 WeAPI `/api/cloud/user/song/match`，目标为 `0` 或省略时明确映射为取消匹配，非零目标映射为网易歌曲引用；统一返回云盘引用、目标引用与 `matched` 状态并保留完整响应；协议载荷、不透明 ID、匹配/取消两分支、参考/统一字段、非标量拒绝、账户隔离和 HTTP 包络均有离线测试；2026-07-16 以 `asid=0` 匿名真实 HTTP 验证在写请求前稳定返回 401 `authentication_required`，待真实账户验证匹配写入与取消回滚） |
| `cloud_upload_complete` | `/cloud/upload/complete` | `implemented` | `POST /v1/account/cloud/uploads/complete`（以 `platform/account` 选择隔离登录态，统一接收 `provisional_track_id/resource_id/md5/filename/song_name/artist/album/bitrate`，并兼容参考字段 `songId/resourceId/song`；完整实现 EAPI `/api/upload/cloud/info/v2` 登记与 `/api/cloud/pub/v2` 发布两段事务，曲名缺省时取文件主名，歌手/专辑缺省时分别使用“未知艺术家/未知专辑”，统一返回最终曲目引用并保留两段原始响应；请求边界、元数据默认值、成功映射、账户前置和 HTTP 别名均有离线测试；2026-07-16 匿名真实 HTTP 验证在发起上游登记前稳定返回 401 `authentication_required`，待真实账户验证成功发布） |
| `cloud_upload_token` | `/cloud/upload/token` | `implemented` | `POST /v1/account/cloud/uploads/ticket`（统一接收 `md5/file_size/filename/bitrate/content_type` 并兼容 `fileSize/contentType`；依次执行 EAPI `/api/cloud/upload/check`、WeAPI `/api/nos/token/alloc` 与真实 LBS 服务发现，完整返回 `needUpload/songId/resourceId` 对应字段、受限 NOS 上传 URL、方法及所需请求头；对象键按路径段编码，上传目标严格限制为无凭据、无自定义端口的 `http(s)://*.127.net` 和精确 `offset=0&complete=true&version=1.0` 参数，拒绝外域、重复参数与目标注入；NOS token 只存在于直传所需响应头映射，Debug 和扩展原文均不泄漏；协议构造、文件/MD5/码率边界、域名白名单、token 脱敏、统一 HTTP 字段及错误包络均有离线测试；2026-07-16 匿名真实 HTTP 验证在申请 token 前稳定返回 401 `authentication_required`，待真实账户验证票据与原始音频直传成功态） |
| `cloudsearch` | `/cloudsearch` | `verified` | `GET /v1/search?variant=cloud`（统一接受 `q/keywords`、平台、账户与分页；完整支持参考项目全部 11 种搜索类型及数字值：歌曲 1、专辑 10、歌手 100、歌单 1000、用户 1002、MV 1004、歌词 1006、播客 1009、视频 1014、综合 1018、声音 2000，缺省为歌曲；固定 EAPI `/api/cloudsearch/pc` 并传递 `s/type/limit/offset/total=true`；网易云 `djRadios` 是按需播客而非直播广播，统一结果因此映射为 `Podcast`，直播 `RadioStation` 搜索能力不被错误声明；其余已知结构映射为 `Track/Album/Artist/Playlist/User/Video`，歌词命中仍为歌曲并保留歌词原文，综合/声音及不可稳定映射项用不丢失原文的 `opaque` 表达；声音响应优先专用 `voices/voiceCount`，空数组或空 `result` 不会遮住后续非空兼容结构；视频包装同时含多层资源时按可用 ID、标题、完整创作者及媒体元数据选择最丰富层，空/零视频或创作者 ID 与零时长会继续回退有效字段，完整响应与分页应用状态保留在扩展；类型映射、全部分支、混合已知/未知条目、冲突字段、分页、能力声明、参数别名和 HTTP 错误均有离线测试；2026-07-16 手动运行 11 个显式忽略的 provider 联网测试全部通过，随后匿名真实 HTTP 以 `keywords=周杰伦&limit=2` 验证 11 个类型均返回 200：1/10/100/1000/1002/1004/1006 分别返回 2 项，1009 按上游真实行为返回 10 项并标记 `limit_applied=false`，1014/1018/2000 返回合法空结果） |
| `comment` | `/comment` | `implemented` | 2026-07-18 已按新上游把评论写入从 WeAPI 迁移为 EAPI + v2 checkToken；统一语义、模型和端点保持不变，待真实账户回归成功写入及删除后升级 verified。`POST /v1/resources/{type}/{ref}/comments`、`POST /v1/resources/{type}/{ref}/comments/{comment_id}/replies`、`DELETE /v1/resources/{type}/{ref}/comments/{comment_id}`（统一以资源引用决定内容平台、`account` 选择隔离登录态，并以 `CommentMutationResult` 表达目标、`create/reply/delete` 动作及不透明评论 ID；完整支持参考 `t=1/2/0` 三分支和 `type=0..7` 全部资源类型，固定映射歌曲 `R_SO_4_`、MV `R_MV_5_`、歌单 `A_PL_0_`、专辑 `R_AL_3_`、电台节目 `A_DJ_1_`、视频 `R_VI_62_`、动态完整 `A_EV_2_...` thread ID、电台 `A_DR_14_`，分别以 EAPI + v2 checkToken 调用 `/api/resource/comments/add|reply|delete`；内容仅以 trim 判空而不改写合法空格，事件 thread、视频和评论 ID 均保持不透明，评论/回复/用户映射会跳过空值及零 ID 后继续采用有效别名，完整响应保留在扩展；核心序列化、8 种资源×3 动作协议、结果 ID、冲突别名、字段/平台边界、能力声明、统一名称与数字别名、JSON 拒绝及 HTTP 包络均有离线测试；2026-07-16 无 Cookie 真实二进制 HTTP 分别验证创建、回复、删除三条路径均在上游写请求前返回 401 `authentication_required`，待真实账户验证成功创建、回复和删除回滚） |
| `comment_album` | `/comment/album` | `verified` | `GET /v1/resources/album/{ref}/comments`（统一 `Comment[]` 目录；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:32311` 返回上游 `code=200`、普通评论及 `mode=legacy`，请求 `limit=1` 被实际应用） |
| `comment_dj` | `/comment/dj` | `verified` | `GET /v1/resources/radio_episode/{ref}/comments`（电台节目 ID 保持不透明；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:794062371` 返回上游 `code=200`、普通评论及 `mode=legacy`） |
| `comment_event` | `/comment/event` | `verified` | `GET /v1/resources/event/{ref}/comments`（要求完整 `A_EV_2_...` thread ID，不重复拼接前缀；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:A_EV_2_6559519868_32953014` 返回上游 `code=200`、普通及热门评论） |
| `comment_floor` | `/comment/floor` | `verified` | `GET /v1/resources/{type}/{ref}/comments?view=replies&parent_comment_id=...`（完整支持 `limit/before_time_ms`，也兼容 `parentCommentId/time`；父评论 ID 保持不透明，当前父评论与楼层回复分开映射；2026-07-16 provider 与真实二进制 HTTP 实测歌曲评论楼层返回上游 `code=200`、`mode=floor`，空楼层被如实表达为成功空目录） |
| `comment_hot` | `/comment/hot` | `verified` | `GET /v1/resources/{type}/{ref}/comments?view=hot`（热门评论独立返回在 `hot_comments`；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:185809`、`limit=2` 返回上游 `code=200`、2 条热门评论、`mode=hot` 且页大小已应用） |
| `comment_hug_list` | `/comment/hug/list` | `implemented` | `GET /v1/resources/{type}/{ref}/comments/{comment_id}/reactions/hug`（抽象为可扩展统一评论反应目录，`target_user_ref` 指向评论作者，`account` 选择同平台隔离登录态；完整兼容参考 `uid/cid/sid/type/page/cursor/idCursor/pageSize` 语义，其中目标、评论 ID 与资源由统一路径表达，也接受 `target_user_id/targetUserId/uid`、`pageNo/pageSize/idCursor`；固定 EAPI `/api/v2/resource/comments/hug/list`，完整提交 `targetUserId/commentId/threadId/pageNo/pageSize/cursor/idCursor`，8 种资源 thread 前缀及动态完整 thread ID 全部覆盖；统一映射 `hugComments[{user,hugContent}]`、`currentComment/hasMore/hugTotalCounts` 和双游标，嵌套成功响应及未来字段不丢失；核心契约、协议构造、成功态、畸形响应、能力发现、统一/参考查询、跨平台冲突与分页边界均有离线测试；2026-07-16 匿名真实二进制 HTTP 以统一和参考两套输入均在网络请求前返回 401 `authentication_required`，待真实账户验证成功目录及续页） |
| `comment_info_list` | `/comment/info/list` | `verified` | `GET /v1/resources/{type}/comments/stats`（统一为同平台、同类型的 `CommentThreadStatsBatch`，`platform/account` 分离内容平台与可选登录态，完整支持 `type=0..7` 名称/数字映射和参考 `ids` 逗号列表、单 `id` 回退、空批次及重复项；固定 WeAPI `/api/resource/commentInfo/list`，精确映射内部 `resourceType=4/5/0/3/1/62/2/14` 与 JSON 字符串 `resourceIds`；每项统一返回请求引用、canonical 评论目标、点赞态及计数、评论计数/文案、分享数、升级态、音乐人评论数、最新点赞用户和评论快照，单项原文与完整响应不丢失；请求引用与 canonical 目标明确分离，覆盖视频哈希转内部 ID、动态数值 ID 转完整 thread；协议、8 类型、空/异常响应、用户/评论、能力发现及 HTTP 包络均有测试；2026-07-16 手动运行 provider 八类型联网测试通过，真实二进制统一 HTTP 再验证歌曲双 ID、MV、歌单、专辑、电台节目、视频、动态、电台及空批次全部上游 `code=200`，评论数分别含歌曲 68334、MV 681、歌单 729、专辑 1989、节目 8、视频 1123，视频 `netease:89ADDE33C0AAE8EC14B99F6750DB954D` canonical 为 `netease:2335163`，动态 `netease:6559519868` canonical 为 `netease:A_EV_2_6559519868_0`） |
| `comment_like` | `/comment/like` | `implemented` | `PUT/DELETE /v1/resources/{type}/{ref}/comments/{comment_id}/reactions/like`（统一以方法表达点赞 `active=true/false`，`ref` 决定内容平台、`account` 选择同平台隔离登录态，结果稳定返回目标、评论 ID、`kind=like` 和最终状态；完整支持参考 `t=1/0` 两分支及 `type=0..7` 全部资源类型，固定映射歌曲 `R_SO_4_`、MV `R_MV_5_`、歌单 `A_PL_0_`、专辑 `R_AL_3_`、电台节目 `A_DJ_1_`、视频 `R_VI_62_`、动态完整 `A_EV_2_...` thread ID、电台 `A_DR_14_`，分别调用 WeAPI `/api/v1/comment/like` 与 `/api/v1/comment/unlike`，精确提交 `threadId/commentId`；非 `like` 反应、跨平台目标和未知查询字段在上游请求前以统一 400 拒绝，未登录态在网络前以 401 拒绝；核心契约、8 种资源×点赞/取消协议、事件 thread、能力发现、统一 HTTP 包络及错误分支均有离线测试；2026-07-16 无 Cookie 真实二进制 HTTP 验证歌曲 PUT/DELETE 与动态 DELETE 均返回 `authentication_required`，待真实账户验证成功点赞及取消回滚） |
| `comment_music` | `/comment/music` | `verified` | `GET /v1/resources/track/{ref}/comments`（统一普通/热门/置顶评论、作者、时间、点赞、回复关系及 IP 地区；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:185809`、`limit=1` 返回上游 `code=200`、普通评论、15 条平台热门评论及 `mode=legacy`） |
| `comment_mv` | `/comment/mv` | `verified` | `GET /v1/resources/mv/{ref}/comments`（2026-07-16 provider 与真实二进制 HTTP 实测 `netease:5436712` 返回上游 `code=200`、普通及热门评论、`mode=legacy`） |
| `comment_new` | `/comment/new` | `verified` | `GET /v1/resources/{type}/{ref}/comments?sort=recommended|hot|time`（完整支持现代目录三种排序、`page/cursor/include_replies` 及参考 `sortType/pageNo/pageSize/showInner`；2026-07-16 provider 与真实二进制 HTTP 对 `netease:185809` 三种排序均实测上游 `code=200`、`mode=modern`；热门及时间排序请求 2 条均返回 2 条，推荐排序上游固定返回 10 条并正确标记 `limit_applied=false`） |
| `comment_playlist` | `/comment/playlist` | `verified` | `GET /v1/resources/playlist/{ref}/comments`（2026-07-16 provider 与真实二进制 HTTP 实测 `netease:705123491` 返回上游 `code=200`、普通及热门评论、`mode=legacy`） |
| `comment_report` | `/comment/report` | `implemented` | `POST /v1/resources/track/{ref}/comments/{comment_id}/reports`（统一由资源引用决定内容平台、`account` 选择同平台隔离登录态，JSON `{reason}` 表达参考必填举报理由，稳定返回目标、评论 ID、原样理由和 `submitted=true`；严格保持参考模块的歌曲专用边界，只接受 `type=track`，固定构造 `threadId=R_SO_4_{id}`，以默认 EAPI 调用 `/api/report/reportcomment` 并精确提交 `threadId/commentId/reason`，不虚构其他七类资源支持；空白理由、非歌曲目标、跨平台引用、未知 JSON/query 字段均在上游请求前以统一 400 拒绝，未登录态在网络前以 401 拒绝，完整成功响应保留在扩展；核心序列化、协议字段、歌曲限定、输入拒绝、能力发现、认证前置与统一 HTTP 包络均有离线测试；2026-07-16 无 Cookie 真实二进制 HTTP 验证合法歌曲举报返回 `authentication_required`，歌单目标与空白理由分别返回预期 400，待真实账户验证成功举报） |
| `comment_video` | `/comment/video` | `verified` | `GET /v1/resources/video/{ref}/comments`（视频 ID 保持不透明字符串；2026-07-16 provider 与真实二进制 HTTP 实测 `netease:89ADDE33C0AAE8EC14B99F6750DB954D` 返回上游 `code=200`、普通及热门评论、`mode=legacy`） |
| `countries_code_list` | `/countries/code/list` | `verified` | `GET /v1/auth/country-codes`（统一以 `platform/account` 选择平台与隔离会话，省略时使用默认平台和 `default` 账户；固定 EAPI `/api/lbs/countries/v1` 空负载，映射 `data[{label,countryList[{code,locale,zh,en}]}]` 为有序 `CountryCallingCodeGroup[]`，稳定字段分别表达电话区号、地区代码、中英文名称，组/条目原文和目录状态不丢失；缺失分组数组、`countryList` 或任一必需字段均返回稳定 `upstream_error`，未知平台/query 在发网前拒绝；核心序列化、成功映射、畸形响应、能力发现、平台/账户/default 选择和 HTTP 错误包络均有测试；2026-07-17 显式 provider 联网测试及真实二进制统一 HTTP 均返回上游 `code=200`，完整获得 22 组、189 个地区且地区代码无重复，首项为 `86/CN/中国/China`） |
| `creator_authinfo_get` | `/creator/authinfo/get` | `pending` | — |
| `daily_signin` | `/daily_signin` | `pending` | — |
| `decrypt` | `/decrypt` | `pending` | — |
| `digitalAlbum_detail` | `/digitalAlbum/detail` | `verified` | `GET /v1/digital-albums/{ref}`（`/album/detail` 的公开别名，共用实现与验证证据） |
| `digitalAlbum_ordering` | `/digitalAlbum/ordering` | `pending` | — |
| `digitalAlbum_purchased` | `/digitalAlbum/purchased` | `pending` | — |
| `digitalAlbum_sales` | `/digitalAlbum/sales` | `pending` | — |
| `dj_banner` | `/dj/banner` | `implemented` | `GET /v1/banners?catalog=podcast`（与音乐推广横幅共用统一 `Banner` 模型，但固定使用 WeAPI `/api/djradio/banner/get`，不伪造该接口不存在的客户端分支；目标类型 `60001` 稳定映射为 `podcast_episode`，节目引用、标题、封面、Orpheus 跳转、独家标志及完整平台原文均保留；省略 `catalog` 仍严格保持既有音乐横幅行为，播客目录显式拒绝非 PC 的客户端选择；2026-07-18 底层原始 API 匿名实测 `code=200`、返回 3 项，首项目标 `netease:3723949603`、标题“脱口秀”，协议选择、映射、非法组合和统一 HTTP 离线测试已完成，provider 与真实二进制统一端点按 Basic 收口集中验收） |
| `dj_category_excludehot` | `/dj/category/excludehot` | `verified` | `GET /v1/podcasts/categories?kind=non_hot`（也接受 `exclude_hot` 及连字符别名；固定空负载 WeAPI `/api/djradio/category/excludehot`，从顶层 `data` 映射稳定 `PodcastTaxonomy`，数字/字符串 ID、名称、网页/尺寸/客户端图标及完整单项和响应原文均不丢失，且以 `extensions.kind=non_hot` 区分完整分类目录；协议、映射、畸形结构、查询别名和统一 HTTP 均有测试；2026-07-22 provider 显式联网与真实二进制统一 HTTP 均返回上游 `code=200`、13 个有效分类，首项为 `11`“知识”） |
| `dj_category_recommend` | `/dj/category/recommend` | `verified` | `GET /v1/podcasts/category-recommendations`（独立 `PodcastCategoryRecommendations` 能力；固定空负载 WeAPI `/api/djradio/home/category/recommend`，不把上游分组错误压平为普通目录：每项稳定分离 `PodcastCategory` 与完整 `Podcast[]`，保留算法、推荐文案、原始分组和顶层响应；缺失分组数组、分类 ID/名称或播客数组均稳定拒绝；协议、映射、异常边界、账户选择及统一 HTTP 均有测试；2026-07-22 provider 显式联网与真实二进制统一 HTTP 均返回上游 `code=200`、12 个分组，首组分类 `3`“情感”含 3 个播客，首项 `netease:526564706`“伴听FM”） |
| `dj_catelist` | `/dj/catelist` | `verified` | `GET /v1/podcasts/categories`（固定 WeAPI `/api/djradio/category/get` 空负载，`platform/account` 分别选择平台与持久账户别名；统一 `PodcastTaxonomy` 将数字或字符串分类 ID 归一为不透明字符串，映射名称并从网页、尺寸及客户端图标字段稳定回退，单项与顶层完整原文均保存在扩展；缺失分类数组、ID、名称、未知平台和查询字段均稳定拒绝；2026-07-17 provider 显式联网及真实二进制统一 HTTP 均返回上游 `code=200`、19 个分类，全部 ID/名称/图标有效，能力发现和未知参数 400 分支同时验收） |
| `dj_detail` | `/dj/detail` | `verified` | `GET /v1/podcasts/{ref}`（统一为与直播 `RadioStation` 分离的 `Podcast`，资源引用决定平台、`account` 选择该平台可选持久登录态；固定 WeAPI `/api/djradio/v2/get` 和数字 `id`，稳定映射名称、介绍、封面、主播、主/次分类、节目/订阅/播放数、订阅态、付费/购买态及创建时间，空主播 ID/昵称会回退后续有效兼容字段，单项原文与完整响应不丢失；缺失对象、非法 ID 和上游错误均稳定拒绝；2026-07-17 provider 显式联网测试及真实二进制统一 HTTP 验证 `netease:336355127` 返回“代码时间”、36 期节目和上游 `code=200`） |
| `dj_difm_all_style_channel` | `/dj/difm/all/style/channel` | `pending` | — |
| `dj_difm_channel_subscribe` | `/dj/difm/channel/subscribe` | `pending` | — |
| `dj_difm_channel_unsubscribe` | `/dj/difm/channel/unsubscribe` | `pending` | — |
| `dj_difm_playing_tracks_list` | `/dj/difm/playing/tracks/list` | `pending` | — |
| `dj_difm_subscribe_channels_get` | `/dj/difm/subscribe/channels/get` | `pending` | — |
| `dj_hot` | `/dj/hot` | `verified` | `GET /v1/podcasts?catalog=hot`（统一 `PodcastCatalog` 保留后续平台目录扩展能力，网易云固定 WeAPI `/api/djradio/hot/v1` 并精确提交 `limit=1..100/offset`，不接受该上游不支持的 `category_id`；统一 `Podcast` 映射封面、主播、分类、节目/订阅/播放统计、付费态及创建时间，缺失 ID/名称或目录数组稳定拒绝，每项原文、真实 `count/hasMore` 和完整响应均保存在扩展；2026-07-18 provider 显式联网及真实二进制统一 HTTP 均返回上游 `code=200`，请求 2 项得到“`四只烤翅`”“`利胜`”、`total=null`、`next_offset=2`、`has_more=true`，能力发现同步声明 `podcast_list`） |
| `dj_paygift` | `/dj/paygift` | `implemented` | `GET /v1/podcasts?catalog=paid&limit=...&offset=...`（也接受 `paygift`；固定 WeAPI `/api/djradio/home/paygift/list` 并精确提交参考 `limit/offset/_nmclfl=1`，拒绝分类和独立 page；从 `data.list` 映射完整 `Podcast`，以 `data.hasMore` 生成真实 `next_offset`，上游无可靠总数所以 `total=null`；统一模型新增可空 `price: Money`，优先使用非空 `discountPrice`、否则使用 `originalPrice`，把网易云分值转换为 CNY，同时保留 `radioFeeType/feeScope` 付费态及完整原文；协议、异常边界、价格优先级、分页和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 以 `limit=3/offset=0` 实测 `code=200`、3 项、`hasMore=true`，首项“广播剧《青梅屿》”价格 1290 分，按集中验收安排留待 Basic 代码收口后复跑 provider 与真实统一 HTTP） |
| `dj_personalize_recommend` | `/dj/personalize/recommend` | `implemented` | `GET /v1/podcasts?catalog=personalized`（固定 WeAPI `/api/djradio/personalize/rcmd` 并精确提交 `limit=1..100`；该头部推荐不支持偏移或分类筛选，统一端点要求 `offset=0` 并拒绝 `category_id`，从顶层 `data` 数组映射完整 `Podcast`，保留推荐算法与全部条目原文；上游没有总数和续页游标，稳定表达为 `total=null/next_offset=null/has_more=false/limit_applied=true`；协议、异常边界、映射和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 以 `limit=3` 实测 `code=200`、3 项且算法为 `hot_server`，按集中验收安排留待 Basic 代码收口后复跑 provider 与真实统一 HTTP） |
| `dj_program` | `/dj/program` | `verified` | `GET /v1/podcasts/{ref}/episodes`（固定 WeAPI `/api/dj/program/byradio`，完整支持 `limit=1..100/offset/ascending` 并兼容参考 `asc`；统一 `PodcastEpisode` 明确分离节目 `ref`、所属 `podcast_ref` 与 `mainTrackId/mainSong` 对应的可播放 `audio.ref`，映射封面、主播、时长、发布时间、序号、收听/点赞/评论/分享数、歌词、订阅和付费态，零节目摘要时长/创建时间不会遮住完整音频时长或有效计划发布时间，分页保留 `count/more` 与完整响应；`mainTrackId` 和 `mainSong.id` 冲突时拒绝而不猜测；2026-07-17 provider 联网及真实二进制 HTTP 对 `netease:336355127` 请求 2 项成功，总数 36，首项节目 `netease:1367665101` 与音频 `netease:530692704` 保持独立，上游 `code=200`） |
| `dj_program_detail` | `/dj/program/detail` | `verified` | `GET /v1/episodes/{ref}`、`GET /v1/episodes/{ref}/stream` 及 `/stream/redirect`（固定 WeAPI `/api/dj/program/detail` 和节目 `id`，复用节目目录的完整稳定映射及节目/音频身份一致性校验；播放先取得独立 `audio.ref`，再复用统一歌曲的全部音质、VIP、账户、严格跨平台回退及 302 链，详情原文与整份响应保存在扩展；2026-07-17 provider 显式联网测试及真实二进制 HTTP 验证节目 `netease:1367665101` 成功，所属播客为 `netease:336355127`、独立音频为 `netease:530692704`、上游 `code=200`；JSON 流命中网易云且尝试状态为 `success`，重定向返回 302） |
| `dj_program_toplist` | `/dj/program/toplist` | `implemented` | `GET /v1/episodes?catalog=popular&limit=...&offset=...`（固定 WeAPI `/api/program/toplist/v1`，统一 `PodcastEpisodeChartEntry` 显式分离 `rank/previous_rank/score` 和完整 `PodcastEpisode`，节目与可播放 `audio.ref` 身份继续分离，榜单包装层明确付费态优先于节目内层默认值，完整条目及响应保存在扩展；参考模块提交 `limit/offset`，但 2026-07-18 底层原始 API 对 `limit=3` 的 offset 0 与 3 实测返回完全相同的 3 个节目 ID，故兼容接收 offset 但稳定返回 `offset=0/requested_offset/offset_submitted=true/offset_applied=false`，不生成虚假续页；协议、映射、排名负一新上榜语义、异常边界和统一 HTTP 离线测试已完成，provider 与真实二进制联网按 Basic 收口集中验收） |
| `dj_program_toplist_hours` | `/dj/program/toplist/hours` | `implemented` | `GET /v1/episodes?catalog=trending24_hours&limit=...`（也接受 `hours/24h`；固定 WeAPI `/api/djprogram/toplist/hours`，该参考模块没有 offset，统一端点明确拒绝非零 offset；从 `data.list` 映射排名包装和完整节目，保留 `data.total/updateTime` 与原始响应，分页不伪造续页并标记 `continuation_supported=false`；2026-07-18 底层原始 API 以 `limit=3` 实测 `code=200`、`total=3`、排名 1–3 且全部含独立节目与音频 ID，协议、映射、异常边界和统一 HTTP 离线测试已完成，provider 与真实二进制联网按 Basic 收口集中验收） |
| `dj_radio_hot` | `/dj/radio/hot` | `implemented` | `GET /v1/podcasts?catalog=category_hot&category_id=...`（要求数字分类 ID，固定 WeAPI `/api/djradio/hot` 并精确提交参考 `cateId/limit/offset`；统一 `Podcast` 保留完整分类播客与原文，`count/hasMore` 映射为真实分页；上游首屏会在请求窗口外插入推荐项，TuneWeave 不截断，返回数超过请求量时标记 `limit_applied=false`，下一偏移仍严格使用 `offset+limit` 而非返回项数，避免跳过正常窗口；缺失/非数字分类、非法 limit 及错误响应稳定拒绝；协议、异常边界、映射和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 对分类 2 实测 `limit=3/offset=0` 返回 8 项、`count=1000/hasMore=true`，offset 3 与 8 的续页结果证明必须按请求窗口推进，按集中验收安排留待 Basic 代码收口后复跑 provider 与真实统一 HTTP） |
| `dj_recommend` | `/dj/recommend` | `implemented` | `GET /v1/podcasts?catalog=featured`（固定无参数 WeAPI `/api/djradio/recommend/v1`；该上游返回不可续页的完整精选快照，统一端点要求 `offset=0` 且拒绝 `category_id`，保留请求 `limit` 但以 `limit_applied=false` 明示平台未应用，`total` 为实际返回项数、`next_offset=null/has_more=false`；复用完整 `Podcast` 映射并保留栏目名称、单项原文与顶层响应；协议、异常边界、映射和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 实测 `code=200`、10 项及栏目“精选电台 - 谈情说爱”，按集中验收安排留待 Basic 代码收口后复跑 provider 与真实统一 HTTP） |
| `dj_recommend_type` | `/dj/recommend/type` | `implemented` | `GET /v1/podcasts?catalog=category_featured&category_id=...`（要求数字分类 ID，固定无分页参数的 WeAPI `/api/djradio/recommend` 并提交参考 `cateId`；分类精选快照复用完整 `Podcast` 映射，要求 `offset=0`，保留请求 `limit` 但标记 `limit_applied=false`；上游可能返回 `hasMore=true` 却不接受任何续页参数，统一响应如实保留 `has_more=true/next_offset=null` 并以 `continuation_supported=false` 明确不可续页，不伪造 offset；缺失/非数字分类、非零 offset 和错误结构稳定拒绝；协议、异常边界、映射和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 对分类 2 实测 `code=200`、10 项、`hasMore=true`，按集中验收安排留待 Basic 代码收口后复跑 provider 与真实统一 HTTP） |
| `dj_sub` | `/dj/sub` | `implemented` | `PUT/DELETE /v1/account/library/podcasts/{ref}`（PUT 固定 WeAPI `/api/djradio/sub`，DELETE 固定 `/api/djradio/unsub`，均提交数字 `id`；资源引用决定平台，`account` 精确选择该平台持久登录态，统一返回 `SubscriptionResult` 并保留完整响应；请求构造、非法 ID、缺失账户别名、成功映射及统一 PUT/DELETE 离线 HTTP 均已测试，登录成功写入与回滚按 Basic 代码收口集中验收） |
| `dj_sublist` | `/dj/sublist` | `implemented` | `GET /v1/account/library/podcasts?limit=...&offset=...`（固定 WeAPI `/api/djradio/get/subed` 并提交 `limit/offset/total=true`；统一 `Podcast[]` 完整映射，`count` 与优先非空的 `hasMore/more` 生成真实分页，空页不会因陈旧 more 伪造下一页；资料库容器明确表示已订阅，因此覆盖单项可能陈旧的 `subed=false` 为 `subscribed=true`，同时保留原始条目和完整响应；映射、兼容分页、空页、异常结构、账户隔离及统一 HTTP 离线测试已完成，登录成功目录按 Basic 收口集中验收） |
| `dj_subscriber` | `/dj/subscriber` | `pending` | — |
| `dj_today_perfered` | `/dj/today/perfered` | `implemented` | `GET /v1/podcasts?catalog=today_preferred&page=...`（固定 WeAPI `/api/djradio/home/today/perfered` 并保留参考模块零基 `page`，省略时为 0；统一 `PodcastListRequest` 新增独立可选页码而不与 offset 混淆，该目录要求 `offset=0` 且拒绝分类筛选，从顶层 `data` 数组映射完整播客并保留原文；上游不应用 limit，也不返回总数、hasMore 或可靠下一页，稳定表达为 `total=null/next_offset=null/has_more=false/limit_applied=false`，页码位于扩展；协议、页码/异常边界、空与非空映射和统一 HTTP 离线测试已完成，2026-07-18 底层原始 API 对匿名 page 0/1 均实测 `code=200` 合法空数组，登录态成功内容按集中验收安排留待 Basic 代码收口后验证） |
| `dj_toplist` | `/dj/toplist` | `implemented` | `GET /v1/charts/podcasts?kind=new|hot&limit=...&offset=...`（固定 WeAPI `/api/djradio/toplist`；`new` 精确提交参考实现因 JavaScript 默认值语义产生的字符串 `type="0"`，`hot` 提交数字 `type=1`，两者均提交 `limit/offset`；统一 `PodcastChartEntry` 将正排名、允许 `-1` 的上期排名、分值与完整 `Podcast` 分离，主播、付费态、统计及完整榜单条目/响应不丢失；2026-07-18 底层原始 API 分别实测新晋榜与热门榜 `code=200`，首项为“一条小团团OvO的翻唱合集”和“清音悦耳”，但 `limit=3` 的 offset 0 与 3 返回相同 ID，故兼容接收 offset 而稳定表达 `offset=0/requested_offset/offset_submitted=true/offset_applied=false/continuation_supported=false`，绝不伪造续页；协议、稀疏字段、排名边界和统一 HTTP 离线测试已完成，provider 与真实二进制联网按 Basic 收口集中验收） |
| `dj_toplist_hours` | `/dj/toplist/hours` | `implemented` | `GET /v1/charts/podcast-creators?kind=trending24_hours&limit=...`（也接受 `hours/24h`；固定 WeAPI `/api/dj/toplist/hours`，只提交参考支持的 `limit` 并在发网前拒绝非零 offset；统一 `PodcastCreatorChartEntry` 将排名、上期排名、分值、粉丝数与完整 `User` 身份分离，用户认证、直播和未来字段完整保留在扩展；返回 `data.total/updateTime`，但无续页控制，稳定表达 `offset=0/next_offset=null/has_more=false/offset_submitted=false/continuation_supported=false`；2026-07-18 底层原始 API 以 `limit=3` 实测 `code=200/total=3`，首位“开心锤锤”排名 1、上期 7、粉丝 76488；协议、映射、异常边界和统一 HTTP 离线测试已完成，provider 与真实二进制统一端点按 Basic 收口集中验收） |
| `dj_toplist_newcomer` | `/dj/toplist/newcomer` | `implemented` | `GET /v1/charts/podcast-creators?kind=newcomer&limit=...&offset=...`（也接受 `new`；固定 WeAPI `/api/dj/toplist/newcomer` 并按参考提交 `limit/offset`，映射完整主播身份、排名、上期排名、分值及粉丝数；2026-07-18 底层原始 API 对 `limit=3` 的 offset 0 与 3 实测返回完全相同的三个用户 ID，故保留输入兼容但稳定返回 `offset=0/requested_offset/offset_submitted=true/offset_applied=false/continuation_supported=false`，不伪造续页；首位“煎包比比”排名 1、上期 1、分值 862097；协议、映射、异常边界和统一 HTTP 离线测试已完成，provider 与真实二进制统一端点按 Basic 收口集中验收） |
| `dj_toplist_pay` | `/dj/toplist/pay` | `implemented` | `GET /v1/charts/podcasts?kind=paid&limit=...`（固定 WeAPI `/api/djradio/toplist/pay` 并只提交参考支持的 `limit`，非零 offset 在发网前拒绝；从 `data.list` 的稀疏付费榜条目映射 `PodcastChartEntry`，榜单容器明确覆盖 `podcast.paid=true`，缺少 `dj` 对象时以 `creatorName` 保留无平台 ID 的主播摘要，同时保留 `rank/lastRank/score` 和完整原文；真实 `data.total/updateTime` 进入分页及扩展，但上游没有续页参数，稳定返回 `next_offset=null/has_more=false/continuation_supported=false`；2026-07-18 底层原始 API 以 `limit=3` 实测 `code=200/total=3`，首项“猫平安逆袭传奇”；协议、稀疏映射、错误边界和统一 HTTP 离线测试已完成，provider 与真实二进制联网按 Basic 收口集中验收） |
| `dj_toplist_popular` | `/dj/toplist/popular` | `implemented` | `GET /v1/charts/podcast-creators?kind=popular&limit=...`（也接受 `hot`；固定 WeAPI `/api/dj/toplist/popular`，只提交 `limit` 并拒绝非零 offset；与新人及 24 小时榜共用不丢字段的 `PodcastCreatorChartEntry`，真实 `data.total/updateTime` 进入分页与扩展，榜单快照不伪装为可续页目录；2026-07-18 底层原始 API 以 `limit=3` 实测 `code=200/total=3`，首位“应萤”排名 1、上期 1、粉丝 843；协议、映射、异常边界和统一 HTTP 离线测试已完成，provider 与真实二进制统一端点按 Basic 收口集中验收） |
| `djRadio_top` | `/djRadio/top` | `pending` | — |
| `eapi_decrypt` | `/eapi/decrypt` | `pending` | — |
| `event` | `/event` | `pending` | — |
| `event_del` | `/event/del` | `pending` | — |
| `event_forward` | `/event/forward` | `pending` | — |
| `fanscenter_basicinfo_age_get` | `/fanscenter/basicinfo/age/get` | `pending` | — |
| `fanscenter_basicinfo_gender_get` | `/fanscenter/basicinfo/gender/get` | `pending` | — |
| `fanscenter_basicinfo_province_get` | `/fanscenter/basicinfo/province/get` | `pending` | — |
| `fanscenter_overview_get` | `/fanscenter/overview/get` | `pending` | — |
| `fanscenter_trend_list` | `/fanscenter/trend/list` | `pending` | — |
| `fm_trash` | `/fm_trash` | `pending` | — |
| `follow` | `/follow` | `pending` | — |
| `get_userids` | `/get/userids` | `pending` | — |
| `history_recommend_songs` | `/history/recommend/songs` | `pending` | — |
| `history_recommend_songs_detail` | `/history/recommend/songs/detail` | `pending` | — |
| `homepage_block_page` | `/homepage/block/page` | `pending` | — |
| `homepage_dragon_ball` | `/homepage/dragon/ball` | `pending` | — |
| `hot_topic` | `/hot/topic` | `pending` | — |
| `hug_comment` | `/hug/comment` | `pending` | — |
| `inner_version` | `/inner/version` | `pending` | — |
| `lbs_city_code` | `/lbs/city/code` | `pending` | — |
| `like` | `/like` | `pending` | — |
| `likelist` | `/likelist` | `verified` | `GET /v1/account/favorites/tracks`、`GET /v1/users/{ref}/favorites/tracks`（2026-07-17 持久化真实账户 HTTP 实测返回 5 项，喜欢 ID、歌曲详情和统一分页链路成功） |
| `listen_data_realtime_report` | `/listen/data/realtime/report` | `pending` | — |
| `listen_data_report` | `/listen/data/report` | `pending` | — |
| `listen_data_song_play_rank` | `/listen/data/song/play/rank` | `pending` | — |
| `listen_data_today_song` | `/listen/data/today/song` | `pending` | — |
| `listen_data_total` | `/listen/data/total` | `pending` | — |
| `listen_data_year_report` | `/listen/data/year/report` | `pending` | — |
| `listentogether_accept` | `/listentogether/accept` | `pending` | — |
| `listentogether_end` | `/listentogether/end` | `pending` | — |
| `listentogether_heatbeat` | `/listentogether/heatbeat` | `pending` | — |
| `listentogether_play_command` | `/listentogether/play/command` | `pending` | — |
| `listentogether_room_check` | `/listentogether/room/check` | `pending` | — |
| `listentogether_room_create` | `/listentogether/room/create` | `pending` | — |
| `listentogether_status` | `/listentogether/status` | `pending` | — |
| `listentogether_sync_list_command` | `/listentogether/sync/list/command` | `pending` | — |
| `listentogether_sync_playlist_get` | `/listentogether/sync/playlist/get` | `pending` | — |
| `login` | `/login` | `implemented` | `POST /v1/auth/password`（邮箱，待真实账户验证） |
| `login_cellphone` | `/login/cellphone` | `implemented` | `POST /v1/auth/password` / challenge verify（待真实账户验证） |
| `login_qr_check` | `/login/qr/check` | `partial` | `GET /v1/auth/qr/{transaction_id}`（waiting 已验证，确认态待账户实测） |
| `login_qr_create` | `/login/qr/create` | `verified` | `POST /v1/auth/qr`（二维码 key 与业务码会跳过空或不可解析的顶层别名并读取嵌套有效值；2026-07-17 真实 HTTP 创建返回可扫码 URL 及自包含 SVG data URL；二维码图片不依赖外部渲染服务） |
| `login_qr_key` | `/login/qr/key` | `verified` | `POST /v1/auth/qr`（2026-07-17 真实 HTTP 验证上游 key 创建成功；响应只暴露 TuneWeave 随机事务 ID，不泄露上游 key） |
| `login_refresh` | `/login/refresh` | `verified` | `POST /v1/auth/session/refresh`（2026-07-17 持久化真实账户 HTTP 实测返回已认证；新 Cookie 原子替换为单一代际，服务重启后会话及 EAPI 云盘下载继续成功） |
| `login_status` | `/login/status` | `verified` | `GET /v1/auth/session`（空白或零账户 ID/昵称不会误判为已认证；匿名态已验证；2026-07-17 真实二维码确认后返回已认证，并在服务重启后从 `platform/account` 持久化存储恢复） |
| `logout` | `/logout` | `implemented` | `DELETE /v1/auth/session`（待真实账户验证） |
| `lyric` | `/lyric` | `partial` | `GET /v1/tracks/{ref}/lyrics`（由新版歌词覆盖；逐字格式优先且全部文本并存保留，无效旧贡献者 ID 不遮住有效 `userId`） |
| `lyric_new` | `/lyric/new` | `verified` | `GET /v1/tracks/{ref}/lyrics`（普通、翻译、罗马音、逐字及逐字翻译/罗马音均保留；YRC 与 LRC 并存时稳定标记 `format=yrc`，2026-07-18 以公开曲目 `185809` 真实验证两者同时存在且逐字能力不会再被逐行格式覆盖） |
| `mlog_music_rcmd` | `/mlog/music/rcmd` | `pending` | — |
| `mlog_to_video` | `/mlog/to/video` | `pending` | — |
| `mlog_url` | `/mlog/url` | `pending` | — |
| `msg_comments` | `/msg/comments` | `pending` | — |
| `msg_forwards` | `/msg/forwards` | `pending` | — |
| `msg_notices` | `/msg/notices` | `pending` | — |
| `msg_private` | `/msg/private` | `pending` | — |
| `msg_private_history` | `/msg/private/history` | `pending` | — |
| `msg_recentcontact` | `/msg/recentcontact` | `pending` | — |
| `music_first_listen_info` | `/music/first/listen/info` | `pending` | — |
| `musician_cloudbean` | `/musician/cloudbean` | `pending` | — |
| `musician_cloudbean_obtain` | `/musician/cloudbean/obtain` | `pending` | — |
| `musician_data_overview` | `/musician/data/overview` | `pending` | — |
| `musician_play_trend` | `/musician/play/trend` | `pending` | — |
| `musician_sign` | `/musician/sign` | `pending` | — |
| `musician_tasks` | `/musician/tasks` | `pending` | — |
| `musician_tasks_new` | `/musician/tasks/new` | `pending` | — |
| `musician_vip_tasks` | `/musician/vip/tasks` | `pending` | — |
| `mv_all` | `/mv/all` | `verified` | `GET /v1/videos?catalog=all`（独立 `VideoCatalog` 能力；完整支持地区、类型、排序及偏移分页，统一英文值与参考中文值均映射为 `tags` JSON 字符串，精确提交字符串 `total="true"`；`count/hasMore` 驱动真实总数和续页，完整条目/响应不丢失；请求、筛选、分页、异常结构、查询边界和统一 HTTP 均有测试；2026-07-22 真实筛选目录返回 200、3 项非空统一 MV、总数 397 和下一偏移 3） |
| `mv_detail` | `/mv/detail` | `verified` | `GET /v1/videos/netease:22695250`：数值引用推断 MV，真实返回标题、创作者及 240/480/720/1080 四档资源信息 |
| `mv_detail_info` | `/mv/detail/info` | `verified` | `GET /v1/videos/netease:22695250/stats`：真实返回点赞态及点赞、评论、分享计数 |
| `mv_exclusive_rcmd` | `/mv/exclusive/rcmd` | `verified` | `GET /v1/videos?catalog=exclusive`（精确提交参考 `offset/limit`，不接受不存在的地区/类型/排序筛选；按真实 `more` 返回下一偏移并保留完整响应，空白描述/封面和零时长不遮蔽有效数据；2026-07-22 真实统一 HTTP 返回 200、3 项非空 MV、`has_more=true/next_offset=3`） |
| `mv_first` | `/mv/first` | `verified` | `GET /v1/videos?catalog=latest`（只支持参考真实存在的地区和 limit，全部地区精确提交空字符串，`total` 保持布尔值；明确拒绝非零 offset、类型及排序，不伪造分页，稳定返回 `next_offset=null/has_more=false/continuation_supported=false`；2026-07-22 真实统一 HTTP 返回 200 和 3 项非空最新 MV） |
| `mv_sub` | `/mv/sub` | `verified` | `PUT/DELETE /v1/account/library/videos/{ref}?kind=mv`（统一方法表达收藏与取消收藏，登录态严格由引用平台和 `account` 别名选择；精确调用 WeAPI `/api/mv/sub|unsub`，同时提交数值 `mvId` 和字符串化 `mvIds=["..."]`，完整响应保留在 `SubscriptionResult.extensions`；普通视频类型、非法 ID、缺失账户、查询字段和能力发现均有离线回归；2026-07-22 持久化真实账户以公开 MV 完成原始未收藏→PUT 收藏→DELETE 取消收藏闭环，最终详情确认恢复未收藏） |
| `mv_sublist` | `/mv/sublist` | `verified` | `GET /v1/account/library/videos`（固定 WeAPI `/api/cloudvideo/allvideo/sublist` 及参考 `limit/offset/total=true`，映射混合 MV/普通视频的字符串 `vid`、创作者、封面、正时长、播放量和收藏态，来源 `type` 与完整条目保留在扩展；`count/hasMore/more` 生成真实分页，空页不伪造续页，顶层扩展移除已映射大数组；协议、字段、分页、空响应、账户隔离与统一 HTTP 均有测试；2026-07-22 持久化真实账户 HTTP 返回 200、1 项普通视频且 `has_more=false`） |
| `mv_url` | `/mv/url` | `verified` | `GET /v1/videos/netease:22695250/stream` 与 `/redirect`：四档真实 URL、大小及 302 均已验收；零首选清晰度/有效期会继续读取有效兼容字段 |
| `nickname_check` | `/nickname/check` | `pending` | — |
| `personal_fm` | `/personal_fm` | `verified` | `GET /v1/recommendations/personal-fm`（缺省 `backend=classic`，也接受 `default/personal_fm`；固定 WeAPI `/api/v1/radio/get` 和空载荷，不伪造参考实现不存在的分页参数；统一返回当前 `Track[]` 队列快照，`total` 为本次数量，`has_more=false/next_offset=null/continuation_supported=false`，完整响应保留在分页扩展；请求协议、能力发现、歌曲映射、截取语义、账户隔离、冲突和未知字段均有测试；2026-07-18 匿名真实 provider 联网返回非空统一曲目队列） |
| `personal_fm_mode` | `/personal/fm/mode` | `verified` | `GET /v1/recommendations/personal-fm?backend=mode`（也接受 `personal_fm_mode`；固定 EAPI `/api/v1/radio/get`，按参考实现保留可选 `mode/subMode/limit`，统一字段兼容 `sub_mode/submode/subMode`；模式值采用可扩展字符串校验，不把未来平台模式锁死在本地枚举；响应沿用不可续页队列快照并在扩展保留实际后端、协议、参数和完整原文；精确协议、可选参数、校验、统一 HTTP 与匿名真实联网均已覆盖，2026-07-18 返回非空统一曲目队列） |
| `personalized` | `/personalized` | `verified` | `GET /v1/recommendations/playlists?source=personalized`（固定 WeAPI `/api/personalized/playlist`，精确提交 `limit/total=true/n=1000`；首页歌单快照不支持 offset，统一分页固定无续页并保留算法、文案、可反馈态、`hasTaste/category` 及完整响应；播放量兼容当前上游浮点 JSON 并无损保留；协议、映射、输入边界、统一 HTTP 和 2026-07-18 匿名真实非空目录均已验证） |
| `personalized_djprogram` | `/personalized/djprogram` | `verified` | `GET /v1/recommendations/podcast-episodes`（固定无参数 WeAPI `/api/personalized/djprogram`；外层 `program` 完整映射为 `PodcastEpisode`，节目、播客和承载音频引用严格分离，推荐包装保留在扩展；接口无分页控制，非零 offset 被拒绝且 `continuation_supported=false/limit_applied=false`；离线包装/播放身份和 2026-07-18 匿名真实非空节目及音频均已验证） |
| `personalized_mv` | `/personalized/mv` | `verified` | `GET /v1/recommendations/videos?kind=mv&view=featured`（固定无参数 WeAPI `/api/personalized/mv`，不可续页快照不伪造目录；统一 `Video` 保留真实 MV ID、艺人、封面、正时长、播放量、收藏态、算法和完整条目；平台不存在对应分页目录，`mv/catalog` 明确拒绝；2026-07-18 匿名真实返回非空类型化 MV） |
| `personalized_newsong` | `/personalized/newsong` | `verified` | `GET /v1/recommendations/tracks?source=personalized`（固定 WeAPI `/api/personalized/newsong`，精确提交 `type=recommend/limit/areaId`，`area_id` 缺省 0 并只用于此分支；外层 `song` 映射完整 `Track`，算法、文案、可反馈态和原包装不丢失；不可续页快照拒绝 offset/refresh；协议、映射、边界、统一 HTTP 和 2026-07-18 匿名真实非空新歌均已验证） |
| `personalized_privatecontent` | `/personalized/privatecontent` | `verified` | `GET /v1/recommendations/videos?kind=exclusive&view=featured`（固定无参数 WeAPI `/api/personalized/privatecontent`，独家放送入口以 `Video` 表达并保留大小封面、文案、类型、时间和完整包装；不可续页快照拒绝非零 offset；2026-07-18 匿名真实返回非空入口） |
| `personalized_privatecontent_list` | `/personalized/privatecontent/list` | `verified` | `GET /v1/recommendations/videos?kind=exclusive&view=catalog`（固定 WeAPI `/api/v2/privatecontent/list`，精确提交 `offset/limit/total="true"`；统一分页按真实 `more` 生成 `next_offset`，返回项保留 ID、标题、大小封面、文案、时间、类型及完整原文；协议、分页、输入边界、统一 HTTP 和 2026-07-18 匿名真实分页目录均已验证） |
| `pl_count` | `/pl/count` | `pending` | — |
| `playlist_category_list` | `/playlist/category/list` | `pending` | — |
| `playlist_catlist` | `/playlist/catlist` | `pending` | — |
| `playlist_cover_update` | `/playlist/cover/update` | `implemented` | `PUT /v1/playlists/{ref}/cover`（统一接收最大 20 MiB 的 `image/*` 原始字节及 `filename/image_size/crop_x/crop_y/account`，兼容 `imgSize/imgX/imgY`；完整执行 NOS 凭据分配、对象上传和 WeAPI `/api/playlist/cover/update`，不泄露 NOS token；协议载荷、图片边界、匿名认证前置和统一 HTTP 包络已离线验证，待真实账户验证封面写入） |
| `playlist_create` | `/playlist/create` | `implemented` | `POST /v1/playlists`（以 `platform/account` 选择登录态，完整支持 `public/private` 与参考 `privacy=0/10`，以及 `NORMAL/VIDEO/SHARED` 三种歌单类型；创建响应会跳过零首选 ID 并读取后续有效字段；统一/参考字段、响应冲突、认证前置和 HTTP 包络已测试，待真实账户创建并回滚） |
| `playlist_delete` | `/playlist/delete` | `implemented` | `DELETE /v1/playlists/{ref}`、`DELETE /v1/playlists`（单删及同平台批量删除，`refs/ids/id` 接受数组、单值和逗号列表，严格保序并保留重复项；跨平台批次与冲突字段在请求前拒绝，待真实账户删除回滚） |
| `playlist_desc_update` | `/playlist/desc/update` | `implemented` | `PATCH /v1/playlists/{ref}`（`description/desc`，支持空字符串清除；`variant=individual` 固定调用 plain API `/api/playlist/desc/update`，认证前置及冲突字段已测试，待真实账户写入） |
| `playlist_detail` | `/playlist/detail` | `verified` | `GET /v1/playlists/{ref}` |
| `playlist_detail_dynamic` | `/playlist/detail/dynamic` | `pending` | — |
| `playlist_detail_rcmd_get` | `/playlist/detail/rcmd/get` | `pending` | — |
| `playlist_highquality_tags` | `/playlist/highquality/tags` | `pending` | — |
| `playlist_hot` | `/playlist/hot` | `pending` | — |
| `playlist_import_name_task_create` | `/playlist/import/name/task/create` | `pending` | — |
| `playlist_import_task_status` | `/playlist/import/task/status` | `pending` | — |
| `playlist_mylike` | `/playlist/mylike` | `pending` | — |
| `playlist_name_update` | `/playlist/name/update` | `implemented` | `PATCH /v1/playlists/{ref}`（`name` 与 `variant=individual` 调用 plain API `/api/playlist/update/name`，名称边界、认证前置与统一响应已测试，待真实账户写入） |
| `playlist_order_update` | `/playlist/order/update` | `implemented` | `PUT /v1/account/playlists/order`（`refs/ids/id` 接受数组、单值或逗号列表，完整保留账户歌单顺序和重复输入，固定 WeAPI `/api/playlist/order/update`；平台/账户隔离和认证前置已测试，待真实账户写入） |
| `playlist_privacy` | `/playlist/privacy` | `pending` | — |
| `playlist_subscribe` | `/playlist/subscribe` | `pending` | — |
| `playlist_subscribers` | `/playlist/subscribers` | `pending` | — |
| `playlist_tags_update` | `/playlist/tags/update` | `implemented` | `PATCH /v1/playlists/{ref}`（`tags` 接受字符串数组或参考分号字符串，空数组/空字符串清除标签；`variant=individual` 调用 plain API `/api/playlist/tags/update`，标签边界及认证前置已测试，待真实账户写入） |
| `playlist_track_add` | `/playlist/track/add` | `implemented` | `POST /v1/playlists/{ref}/videos` 或 `/items` 的 `kind=video/type=3`（按参考语义操作 VIDEO 歌单而非普通歌曲，ID 保持不透明，固定 WeAPI `/api/playlist/track/add` 并提交 `{type:3,id}`；歌曲/视频分流及认证前置已测试，待真实账户写入） |
| `playlist_track_all` | `/playlist/track/all` | `verified` | `GET /v1/playlists/{ref}/tracks` |
| `playlist_track_delete` | `/playlist/track/delete` | `implemented` | `DELETE /v1/playlists/{ref}/videos` 或 `/items` 的 `kind=video/type=3`（固定 WeAPI `/api/playlist/track/delete`，保持 VIDEO 歌单不透明 ID、顺序与重复项；认证前置和统一响应已测试，待真实账户写入） |
| `playlist_tracks` | `/playlist/tracks` | `implemented` | `POST/DELETE /v1/playlists/{ref}/tracks` 或 `/items` 的 `kind=track`（普通歌曲固定 plain API `/api/playlist/manipulate/tracks` 的 `op=add/del`，精确提交 JSON 字符串 `trackIds` 与 `imme=true`；仅在业务码 512 时按参考实现复制 ID 列表重试，结果记录初次响应；空首选快照 ID 会回退后续有效字段；分支、冲突快照、保序/重复、认证前置和 HTTP 已测试，待真实账户写入/回滚） |
| `playlist_update` | `/playlist/update` | `implemented` | `PATCH /v1/playlists/{ref}`（`variant=batch` 要求并同时提交 `name/description/tags`，固定 plain API `/api/batch` 且三个子请求值为 JSON 字符串；`default` 在三字段齐全时选择批量，否则选择独立模块；完整分支、清空、冲突和认证前置已测试，待真实账户写入） |
| `playlist_update_playcount` | `/playlist/update/playcount` | `pending` | — |
| `playlist_video_recent` | `/playlist/video/recent` | `pending` | — |
| `playmode_intelligence_list` | `/playmode/intelligence/list` | `pending` | — |
| `playmode_song_vector` | `/playmode/song/vector` | `pending` | — |
| `program_recommend` | `/program/recommend` | `pending` | — |
| `radio_sport_get` | `/radio/sport/get` | `pending` | — |
| `rebind` | `/rebind` | `pending` | — |
| `recent_listen_list` | `/recent/listen/list` | `pending` | — |
| `recommend_resource` | `/recommend/resource` | `verified` | `GET /v1/recommendations/playlists`（2026-07-17 持久化真实账户 HTTP 实测返回 5 项） |
| `recommend_songs` | `/recommend/songs` | `verified` | `GET /v1/recommendations/tracks`（含 `afresh`→`refresh`；2026-07-16 匿名 HTTP 实测返回 30 首并保留推荐理由） |
| `recommend_songs_dislike` | `/recommend/songs/dislike` | `implemented` | `POST /v1/recommendations/tracks/{ref}/dislike`（完整曲目引用决定平台，`account` 选择隔离持久账户；网易云固定 WeAPI `/api/v2/discovery/recommend/dislike`，精确提交 `resId/resType=4/sceneType=1`，统一返回 `RecommendationDislikeResult` 并保留完整响应；协议、能力、引用/平台冲突、账户隔离、未知字段及 HTTP 包络均有测试；2026-07-18 匿名真实联网确认上游登录边界稳定映射 401 `authentication_required`，成功写入待 Basic 末尾使用持久化账户验收） |
| `record_recent_album` | `/record/recent/album` | `pending` | — |
| `record_recent_dj` | `/record/recent/dj` | `pending` | — |
| `record_recent_playlist` | `/record/recent/playlist` | `pending` | — |
| `record_recent_song` | `/record/recent/song` | `pending` | — |
| `record_recent_video` | `/record/recent/video` | `pending` | — |
| `record_recent_voice` | `/record/recent/voice` | `pending` | — |
| `register_anonimous` | `/register/anonimous` | `implemented` | `GET/POST /v1/extensions/netease/anonymous-session`，兼容正确拼写 `/register/anonymous` 与参考拼写 `/register/anonimous`（独立 `AnonymousSession` 能力完整复刻 52 位大写十六进制设备 ID、循环 XOR、MD5、双层 Base64 用户名及 XEAPI `/api/register/anonimous`；GET 缺省读取进程/持久化身份，`refresh=true` 与 POST 强刷；设备 ID 和 `MUSIC_A` 作为单一版本化凭据原子落盘，重启恢复后自动供默认公开请求使用，不进入登录账户映射，也不覆盖显式账户；参考兼容响应保留 Cookie，但 Debug/错误不泄漏且客户端不能注入；核心契约、固定编码向量、随机设备格式、Cookie/设备校验、凭据恢复、账户隔离、能力发现、三个路由别名、GET/POST/强刷及非法查询均有测试；2026-07-18 TuneWeave 真实请求与同机当前参考实现均返回上游 `code=400` 且无 Cookie，稳定拒绝伪造身份，待上游恢复后补成功注册验收） |
| `register_cellphone` | `/register/cellphone` | `pending` | — |
| `register_checktoken_v2` | `/register/checktoken/v2` | `verified` | `GET/POST /v1/extensions/netease/register/checktoken/v2`，也可在通用 `/v1/extensions/netease/check-token` 以 `version=v2` 选择（固定请求易盾 `/v2/config/js?pn=YD00000558929251`，严格要求成功 JSON 的非空 `result.conf` 并校验安全 HTTP 头值；v2/v3 使用独立 URL 和共享于账户 client 的独立缓存，返回体以 `version` 明示版本，固定版本路由拒绝冲突参数，旧端点缺省仍为 v3；EAPI 可由 provider 受控取得并注入 v2 token，客户端不能提交 token；核心版本契约、双解析器、畸形响应、缓存隔离、通用/固定版本 GET/POST 和冲突输入均有测试；2026-07-18 真实易盾联网验证 v2/v3 首次注册、缓存命中与强制刷新全部成功） |
| `register_checktoken_v3` | `/register/checktoken/v3` | `verified` | `GET/POST /v1/extensions/netease/check-token`，并兼容 `/v1/extensions/netease/register/checktoken`（当前明确对应 v3；GET 缺省复用进程内缓存，`refresh=1|true` 及 POST 强制刷新；固定请求官方易盾 `/v3/b?pn=YD00000558929251`，严格解析成功 JSONP 并校验安全 HTTP 头值；账户 client 共享缓存，要求 v3 checkToken 的 XEAPI 请求由 provider 自动取得并注入 `X-antiCheatToken`，不接受客户端传 token；稳定结果返回 `token/registered/refreshed`，序列化仍满足参考响应，Debug 和普通日志强制脱敏；核心模型、能力名、有效/字符串业务码、畸形/失败响应、共享缓存、查询布尔值、双路径 GET/POST、未知字段和 HTTP 包络均有测试；2026-07-18 真实易盾联网验证首次注册、缓存命中及强制刷新全部成功） |
| `register_xeapikey` | `/register/xeapikey` | `pending` | — |
| `related_allvideo` | `/related/allvideo` | `pending` | — |
| `related_playlist` | `/related/playlist` | `pending` | — |
| `relay_play_state_submit` | `/relay/play/state/submit` | `pending` | — |
| `rep_ugc_activity_collect` | `/rep/ugc/activity/collect` | `pending` | 2026-07-17 上游新增；云小编领取活动积分，缺省 `activityId=5001` |
| `rep_ugc_activity_get` | `/rep/ugc/activity/get` | `pending` | 2026-07-17 上游新增；云小编活动信息 |
| `rep_ugc_user_collect-vip` | `/rep/ugc/user/collect-vip` | `pending` | 2026-07-17 上游新增；云小编领取一日会员，缺省 `activityId=5001` |
| `rep_ugc_user_get` | `/rep/ugc/user/get` | `pending` | 2026-07-17 上游新增；云小编账户详情 |
| `rep_ugc_user_sign` | `/rep/ugc/user/sign` | `pending` | 2026-07-17 上游新增；云小编每日签到 |
| `rep_ugc_user_vip` | `/rep/ugc/user/vip` | `pending` | 2026-07-17 上游新增；云小编会员任务状态 |
| `resource_like` | `/resource/like` | `pending` | — |
| `sati_resource_list` | `/sati/resource/list` | `pending` | — |
| `sati_resource_list_more` | `/sati/resource/list/more` | `pending` | — |
| `sati_resource_sub` | `/sati/resource/sub` | `pending` | — |
| `sati_resource_sub_list` | `/sati/resource/sub/list` | `pending` | — |
| `sati_tag_list` | `/sati/tag/list` | `pending` | — |
| `sati_timescene_resources_get` | `/sati/timescene/resources/get` | `pending` | — |
| `scrobble` | `/scrobble` | `pending` | — |
| `scrobble_v1` | `/scrobble/v1` | `pending` | — |
| `search` | `/search` | `verified` | `GET /v1/search?variant=legacy`（与 `/cloudsearch` 共用统一端点和 `SearchItem` 判别联合，通过 `variant` 选择参考后端；完整支持 `keywords/q`、`limit/offset` 和全部参考类型 `1/10/100/1000/1002/1004/1006/1009/1014/1018/2000`，普通 10 类固定 EAPI `/api/search/get` 并提交 `s/type/limit/offset`，声音类型精确切换 EAPI `/api/search/voice/get` 并提交 `keyword/scene=normal/limit/offset`，不混入新版 `total=true`；1009 的 `djRadios` 按播客 `Podcast` 映射而不伪装成直播 `RadioStation`；旧声音的 `data.resources/totalCount/hasMore` 与普通 `result` 都进入统一分页，已知实体规范化、综合/声音及异常结构不丢失原文；核心 `default/legacy/cloud` 契约、两套协议负载、11 类型、旧声音形状、服务端别名/错误和分页后端标记均有测试；2026-07-17 显式 provider 联网测试与真实二进制统一 HTTP 逐类验证 11 种类型全部上游 `code=200`，1/10/100/1000/1002/1004/1006 各返回请求的 2 项，1009 按上游行为返回 30 项并标记 `limit_applied=false`，1014 合法空结果、1018 返回完整 opaque 综合块、2000 走声音路径并保留 `total=569`） |
| `search_default` | `/search/default` | `verified` | `GET /v1/search/default`（统一支持 `platform/account`，缺省使用服务默认平台和 `default` 会话；固定 EAPI `/api/search/defaultkeyword/get` 空负载，将 `realkeyword/showKeyword/searchType/imageUrl` 映射为实际 `keyword`、展示 `display_text`、可选 `SearchKind` 和图片，展示词缺失或空白时依次回退 `styleKeyword.keyWord` 与真实词，未知搜索类型保持 `null` 而不猜测；算法、样式、业务意图和完整响应保留在扩展，缺失 `data` 或实际关键词返回稳定 `upstream_error`；核心序列化、协议负载、冲突主/回退字段、未知类型、畸形响应、能力发现、平台/账户/default 选择及 HTTP 错误均有测试；2026-07-17 显式 provider 联网测试和真实二进制统一 HTTP 均返回上游 `code=200`，当前实际词“周旋”、展示文案“🔥周旋 最近很火哦”、`kind=track`） |
| `search_hot` | `/search/hot` | `verified` | `GET /v1/search/trending?detail=brief`（统一以 `platform/account` 选择平台和隔离会话，固定 EAPI `/api/search/hot` 并精确提交 `type=1111`；映射 `result.hots[{first,second,third,iconType}]` 为从 1 开始的 `SearchTrendingEntry[]`，`first` 是稳定关键词、可用 `third` 为说明、`iconType` 保留图标类型，不把语义不明的 `second` 伪装成分数，列表和条目原文完整保留；协议、字段、顺序、缺失数组/关键词、能力发现、统一端点别名和错误均有测试；2026-07-17 显式 provider 联网测试及真实二进制统一 HTTP 均返回上游 `code=200` 和 10 项，首项 rank 1“薛之谦”、`icon_type=1`） |
| `search_hot_detail` | `/search/hot/detail` | `verified` | `GET /v1/search/trending?detail=full`（与简略榜共用统一模型，缺省即详细模式；固定 WeAPI `/api/hotsearchlist/get` 空负载，将 `data[{searchWord,score,content,iconType,iconUrl,url}]` 映射为排名、关键词、分数、说明和图标/目标地址，空字符串保持可空字段，`alg/source` 等完整原文不丢失；完整覆盖 `detail=full/detail/detailed`、`mode` 别名、默认平台/账户和输入拒绝；2026-07-17 显式 provider 联网测试及真实二进制统一 HTTP 均返回上游 `code=200` 和 20 项，首项 rank 1“薛之谦”、`score=107509`、`icon_type=4`） |
| `search_match` | `/search/match` | `verified` | `GET/POST /v1/search/match`（统一 POST 以 `duration_ms` 表达时长，同时兼容参考 GET 查询及 `duration/duration_seconds` 秒数；完整接收 `title/album/artist/duration/md5/platform/account`，标签和时长缺省时保持参考实现的空字符串/0 分支，`md5` 必填、去除外围空白、转小写并校验 32 位十六进制，同时给出毫秒和秒数时必须一致；严格复刻参考默认协议，以未加密直连 API 调用 `/api/search/match/new`，将单项 `{title,album,artist,duration,persistId}` JSON 序列化到 `songs`；上游 `result.ids/songs` 映射为命中 ID 和完整统一 `Track[]`，无命中以成功空数组表达，单项及完整响应原文不丢失；核心契约、协议负载、大小写 MD5、旧歌曲结构、成功/无命中/畸形响应、能力发现、GET/POST 双形态、单位换算/冲突及输入拒绝均有测试；2026-07-17 显式 provider 联网测试和真实二进制 HTTP 均验证成功命中与无命中分支：参考示例命中 `netease:65766`《富士山下》，不存在曲目返回上游 `code=200`、空 `matches` 和空 `matched_ids`） |
| `search_multimatch` | `/search/multimatch` | `verified` | `GET /v1/search/multimatch`（统一接收 `q/keywords/keyword`、`kind/type`、`platform/account`，搜索类型完整复用 `track/album/artist/playlist/user/mv/lyric/podcast/video/mixed/voice` 及网易数字 `1/10/100/1000/1002/1004/1006/1009/1014/1018/2000`；固定 WeAPI `/api/search/suggest/multimatch` 并精确提交 `s/type`，严格按非空 `result.orders` 输出 `SearchMultiMatchSection[]`，`orders=null` 时回退 `order`，未列入顺序但实际存在的数组仍追加保留；歌手、歌单、播客、普通 MV/视频以及 `new_mlog` 均尽可能映射为统一 `SearchItem`，其中 `djRadios` 明确映射为 `Podcast`；未知分区和映射失败项以保留有效 ID、标题及完整原文的 `opaque` 表达，单项原文和完整响应分别保留在条目及结果扩展；核心契约、协议、冲突分区顺序、已知/未知资源、畸形响应、能力发现、统一/参考参数和 HTTP 错误均有测试；2026-07-17 显式 provider 联网测试逐一验证 11 个 `type` 分支均返回上游 `code=200`，真实二进制 HTTP 以参考 `keywords/type=1` 再验证 3 个有序分区：歌手 `Beyond`、`new_mlog` 视频和歌单均为统一类型） |
| `search_suggest` | `/search/suggest` | `verified` | `GET /v1/search/suggestions?client=web|mobile`（统一接收 `q/keywords/keyword`、平台和账户，兼容参考 `type=mobile`；web 固定 WeAPI `/api/search/suggest/web`、mobile 固定 WeAPI `/api/search/suggest/keyword`，两者都精确提交 `s`；web 按上游 `order` 保持歌曲/专辑/歌手/歌单/用户/MV/播客/视频分组顺序，将每项映射为带统一 `SearchItem resource` 的建议，`djRadios` 明确使用 `Podcast` 而不是直播 `RadioStation`，未列入 order 但实际存在的已知数组也不会遗漏；mobile 将 `result.allMatch[{keyword,type,resourceType,...}]` 映射为纯关键词及可识别类型，零/未知 `type` 会继续读取有效 `resourceType`，1009 映射播客且不伪造资源；完整列表/条目和未知分组原文不丢失，缺失容器、错误数组或无关键词返回稳定错误；协议双分支、资源/关键词冲突映射、能力发现、统一/参考参数和输入拒绝均有测试；2026-07-17 显式 provider 联网测试与真实二进制统一 HTTP 均返回 `code=200`，web 6 条且全部带统一资源，mobile 6 条纯关键词，首项均为“海阔天空”） |
| `search_suggest_pc` | `/search/suggest/pc` | `verified` | `GET /v1/search/suggestions?client=pc`（固定 EAPI `/api/search/pc/suggest/keyword/get` 并精确提交参考 `keyword`；完整映射 `data.suggests` 为普通建议、`data.recs` 为独立 recommendations，保留 `showText/iconUrl/resourceType/relatedResource/highLightInfo/recTitle` 等完整原文，任一数组缺省可为空但错误类型或无关键词会稳定失败；统一端点同时接受 `keyword` 原参数名；2026-07-17 显式 provider 联网测试及真实二进制统一 HTTP 返回上游 `code=200`、10 条建议，当前 recommendations 合法为空，首项“海阔天空”） |
| `send_album` | `/send/album` | `pending` | — |
| `send_playlist` | `/send/playlist` | `pending` | — |
| `send_song` | `/send/song` | `pending` | — |
| `send_text` | `/send/text` | `pending` | — |
| `setting` | `/setting` | `pending` | — |
| `share_resource` | `/share/resource` | `pending` | — |
| `sheet_list` | `/sheet/list` | `pending` | — |
| `sheet_preview` | `/sheet/preview` | `pending` | — |
| `sign_happy_info` | `/sign/happy/info` | `pending` | — |
| `signin_progress` | `/signin/progress` | `pending` | — |
| `simi_artist` | `/simi/artist` | `pending` | — |
| `simi_mv` | `/simi/mv` | `pending` | — |
| `simi_playlist` | `/simi/playlist` | `pending` | — |
| `simi_song` | `/simi/song` | `pending` | — |
| `simi_user` | `/simi/user` | `pending` | — |
| `song_chorus` | `/song/chorus` | `pending` | — |
| `song_cloud_download` | `/song/cloud/download` | `verified` | `GET /v1/account/cloud/tracks/{ref}/download`、`GET /v1/account/cloud/tracks/{ref}/download/redirect`（以完整网易云云盘引用和隔离账户取源文件；严格复刻上游 EAPI 路径中的既有拼写 `/api/cloud/dowonload` 及 `songId` 载荷；兼容历史 `data` 对象/数组与当前顶层 `code/name/size/url` 成功结构，专用 `downloadUrl` 与通用 `url` 并存时优先源文件下载地址，统一映射 URL、大小及格式；2026-07-17 持久化真实账户在刷新和服务重启后实测源文件 200、重定向 302，云盘引用直接播放亦返回可用 URL） |
| `song_copyright_rcmd` | `/song/copyright/rcmd` | `pending` | — |
| `song_creators` | `/song/creators` | `pending` | — |
| `song_detail` | `/song/detail` | `verified` | `GET /v1/tracks/{ref}` |
| `song_downlist` | `/song/downlist` | `pending` | — |
| `song_download_url` | `/song/download/url` | `verified` | `GET /v1/tracks/{ref}/download?variant=legacy`（统一 `bitrate` 与参考 `br` 接受任意无符号 bit/s 并原样提交 EAPI `/api/song/enhance/download/url`，省略时按统一音质映射默认码率；统一 `MediaDownload` 保留可用态、URL、格式、实际码率、大小、时长、业务码、费用及完整响应，空白编码/零响应时长不会遮住有效容器格式/歌曲时长；2026-07-17 provider 忽略测试与真实二进制 HTTP 均验证 `netease:2709812973&br=192123` 成功，上游顶层/条目 `code=200`、实际 192000 bit/s、`available=true`） |
| `song_download_url_v1` | `/song/download/url/v1` | `verified` | `GET /v1/tracks/{ref}/download`（缺省或 `variant/backend=modern|v1` 固定 EAPI `/api/song/enhance/download/url/v1`，精确提交字符串 ID、`immerseType=c51` 和九档 `level`；对象与数组两种 `data` 形状均支持，顶层成功但条目无 URL 以 `available=false` 正常返回，不伪造错误或 URL；2026-07-17 可重复 provider 联网测试和真实二进制 HTTP 对 `standard/higher/exhigh/lossless/hires/jyeffect/sky/dolby/jymaster` 九档全部验证上游 `code=200`，八档取得 URL，当前 `sky` 如实返回条目 `code=-110/url=null`） |
| `song_dynamic_cover` | `/song/dynamic/cover` | `pending` | — |
| `song_like` | `/song/like` | `pending` | — |
| `song_like_check` | `/song/like/check` | `pending` | — |
| `song_lyrics_mark` | `/song/lyrics/mark` | `pending` | — |
| `song_lyrics_mark_add` | `/song/lyrics/mark/add` | `pending` | — |
| `song_lyrics_mark_del` | `/song/lyrics/mark/del` | `pending` | — |
| `song_lyrics_mark_user_page` | `/song/lyrics/mark/user/page` | `pending` | — |
| `song_monthdownlist` | `/song/monthdownlist` | `pending` | — |
| `song_music_detail` | `/song/music/detail` | `pending` | — |
| `song_order_update` | `/song/order/update` | `implemented` | `PUT /v1/playlists/{ref}/tracks/order`（`refs/ids/trackIds` 接受单值、数组或逗号列表，完整保序及保留重复项，固定 plain API `/api/playlist/manipulate/tracks` 的 `op=update`；空首选快照会回退后续有效字段；平台/账户、冲突响应和认证前置已测试，待真实账户写入） |
| `song_purchased` | `/song/purchased` | `pending` | — |
| `song_red_count` | `/song/red/count` | `pending` | — |
| `song_singledownlist` | `/song/singledownlist` | `pending` | — |
| `song_url` | `/song/url` | `verified` | `GET /v1/tracks/{ref}/stream`、`GET/POST /v1/tracks/streams`（`variant/backend=legacy`；完整接受统一 `bitrate` 和参考 `br`，任意无符号 bit/s 原样进入 `/api/song/enhance/player/url`，省略时按音质映射默认码率；批量 `id/ids` 保留顺序与重复项并以逐项结果表达失败，空白编码/零响应时长回退有效容器格式/歌曲时长，完整上游响应只保存一次；2026-07-17 真实二进制 HTTP 以 `br=192123` 请求两首歌曲，上游 `code=200`、两项均成功并按平台档位返回 192000 bit/s，路径与变体分别确认为旧版 API/legacy） |
| `song_url_match` | `/song/url/match` | `implemented` | `GET /v1/tracks/{ref}/stream?unblock=true&source=...`、批量端点同参数（复用统一严格匹配解析器而不引入第二套 URL 匹配；支持选择任意平台注册来源，省略时按 QQ/酷狗/酷我/咪咕顺序后回原平台，账号绑定首个目标，返回完整尝试轨迹；离线已验证来源选择、冲突拒绝、账户归属和批量逐项结果；2026-07-17 真实二进制 HTTP 验证当前未注册 QQ 时明确记录 `qq:unavailable` 后 `netease:success`，待 QQ Basic 接入后验证真实跨平台成功 URL） |
| `song_url_ncmget` | `/song/url/ncmget` | `pending` | — |
| `song_url_v1` | `/song/url/v1` | `implemented` | `GET /v1/tracks/{ref}/stream`、`GET/POST /v1/tracks/streams`（缺省或 `variant/backend=modern|v1` 固定 XEAPI `/api/song/enhance/player/url/v1`，精确提交数字 ID 列表、`level`、`encodeType=flac`，`sky` 额外提交 `immerseType=c51`；完整支持 `standard/higher/exhigh/lossless/hires/jyeffect/sky/dolby/jymaster` 九档及统一别名，批量保序、保重复、逐项失败且完整响应不重复；`unblock/source` 分支复用统一回退；2026-07-17 真实二进制 HTTP 对九档逐项验证均为上游 `code=200` 和成功流，三项含重复 ID 的 GET/POST 批量均按原顺序成功；跨平台成功态待对应 provider Basic 接入） |
| `song_url_v1_302` | `/song/url/v1/302` | `verified` | `GET /v1/tracks/{ref}/download/redirect`（先请求对应旧/新版专用下载 URL，非空即发 302；无 URL 时以同一 `quality/variant/bitrate/account` 请求播放 URL，成功后发 302，两个阶段都失败则返回统一错误并保留下载结果和流错误详情；不向客户端暴露上游 Cookie；2026-07-17 真实二进制 HTTP 禁止自动跟随后验证 `exhigh` 直接下载分支与 `sky` 下载 `code=-110` 后播放兜底分支均返回 302 且存在 `Location`） |
| `song_wiki_summary` | `/song/wiki/summary` | `pending` | — |
| `starpick_comments_summary` | `/starpick/comments/summary` | `pending` | — |
| `style_album` | `/style/album` | `pending` | — |
| `style_artist` | `/style/artist` | `pending` | — |
| `style_detail` | `/style/detail` | `pending` | — |
| `style_list` | `/style/list` | `pending` | — |
| `style_playlist` | `/style/playlist` | `pending` | — |
| `style_preference` | `/style/preference` | `pending` | — |
| `style_song` | `/style/song` | `pending` | — |
| `summary_annual` | `/summary/annual` | `pending` | — |
| `threshold_detail_get` | `/threshold/detail/get` | `pending` | — |
| `thinktank_audit_resource_detail` | `/thinktank/audit/resource/detail` | `pending` | 2026-07-17 上游新增；云小编按类型领取曲风、语种、原唱或情绪标签审核任务 |
| `thinktank_audit_resource_update` | `/thinktank/audit/resource/update` | `pending` | 2026-07-17 上游新增；云小编提交同意、否决或跳过审核结果，要求 `taskId/judgement` |
| `top_album` | `/top/album` | `pending` | — |
| `top_artists` | `/top/artists` | `pending` | — |
| `top_list` | `/top/list` | `pending` | — |
| `top_mv` | `/top/mv` | `pending` | — |
| `top_playlist` | `/top/playlist` | `pending` | — |
| `top_playlist_highquality` | `/top/playlist/highquality` | `pending` | — |
| `top_song` | `/top/song` | `pending` | — |
| `topic_detail` | `/topic/detail` | `pending` | — |
| `topic_detail_event_hot` | `/topic/detail/event/hot` | `pending` | — |
| `topic_sublist` | `/topic/sublist` | `pending` | — |
| `toplist` | `/toplist` | `verified` | `GET /v1/charts?view=overview`（统一 `ChartCatalog`，完整保留榜单介绍与特殊榜单原文；2026-07-17 匿名真实 HTTP 返回 1 组、62 个普通音乐榜单，上游 `code=200`） |
| `toplist_artist` | `/toplist/artist` | `verified` | `GET /v1/charts/artists`（统一 `ArtistChart`，支持 `area=chinese/western/korean/japanese` 及参考 `type=1/2/3/4`，固定 100 位快照并保留分数和上期名次；2026-07-17 provider 持久化联网测试及真实 HTTP 四分支全部通过，榜首依次为林俊杰、Justin Bieber、BIGBANG、初音ミク） |
| `toplist_detail` | `/toplist/detail` | `verified` | `GET /v1/charts?view=summary`（默认经典内容摘要，榜单及无 ID 的曲名/歌手预览进入稳定字段，歌手榜和奖励榜原文不丢失；2026-07-17 匿名真实 HTTP 返回 62 个榜单、12 条预览，上游 `code=200`） |
| `toplist_detail_v2` | `/toplist/detail/v2` | `verified` | `GET /v1/charts?view=modern`（完整映射分组、可播放/H5 目标、真实歌曲引用、当前/上期名次与封面；新版 `newFirstCoverUrl` 优先于旧 `firstCoverUrl`，同时仍以显式榜单封面为首选；2026-07-17 匿名真实 HTTP 返回 7 组、49 个目录项、45 条排名预览，上游 `code=200`；另以 `/v1/charts/netease:19723756/tracks?limit=2` 验证飙升榜共 99 首并返回前 2 首） |
| `ugc_album_get` | `/ugc/album/get` | `pending` | — |
| `ugc_artist_get` | `/ugc/artist/get` | `pending` | — |
| `ugc_artist_search` | `/ugc/artist/search` | `pending` | — |
| `ugc_detail` | `/ugc/detail` | `pending` | — |
| `ugc_mv_get` | `/ugc/mv/get` | `pending` | — |
| `ugc_song_get` | `/ugc/song/get` | `pending` | — |
| `ugc_user_devote` | `/ugc/user/devote` | `pending` | — |
| `user_account` | `/user/account` | `verified` | `GET /v1/account` 与 `GET /v1/account/profile`（空白或零 `profile.userId` 不会遮蔽有效账户 ID；2026-07-17 持久化真实账户及服务重启后均返回已认证账户摘要，2026-07-22 同一隔离账户成功解析用户 ID 并取得完整统一资料） |
| `user_audio` | `/user/audio` | `pending` | — |
| `user_binding` | `/user/binding` | `pending` | — |
| `user_bindingcellphone` | `/user/bindingcellphone` | `pending` | — |
| `user_cloud` | `/user/cloud` | `verified` | `GET /v1/account/cloud/tracks`（以 `platform/account/limit/offset` 选择隔离会话，WeAPI `/api/v1/cloud/get`；统一返回 `CloudTrack` 分页，稳定保留云盘引用、内嵌歌曲、文件名/大小/类型、码率、MD5、加入时间、匹配歌曲引用和存储统计，完整原始条目及响应保留在扩展；2026-07-17 持久化真实账户在进程重启前后实测完整读取 209 项三页数据，并兼容历史条目中显式为空的别名、歌手和专辑字段） |
| `user_cloud_del` | `/user/cloud/del` | `verified` | `DELETE /v1/account/cloud/tracks`（JSON `refs` 或 `ids`，支持 `platform/account`、完整引用推断、保序与重复项；WeAPI `/api/cloud/del` 严格保留参考实现的单元素数组载荷 `songIds: [ids.join(",")]`，拒绝跨平台混合和引用冲突；协议、选择器、认证前置及结果映射均有离线测试；2026-07-17 持久化真实账户仅对唯一生成的测试音频执行删除，统一 HTTP 返回 200，随后完整列表恢复 209 项且无测试标记残留） |
| `user_cloud_detail` | `/user/cloud/detail` | `verified` | `GET/POST /v1/account/cloud/tracks/details`（`refs` 或 `ids`，完整引用可推断平台，原始 ID 使用显式或默认平台；WeAPI `/api/v1/cloud/get/byids` 按 `songIds` 数组请求，统一结果保持输入顺序和重复项，并复用完整 `CloudTrack` 映射；2026-07-17 持久化真实账户以列表返回的统一引用实测详情成功） |
| `user_comment_history` | `/user/comment/history` | `pending` | — |
| `user_detail` | `/user/detail` | `verified` | `GET /v1/users/{ref}?backend=legacy` 及 `GET /v1/account/profile?backend=legacy`（独立 `UserProfileLegacy` 能力，精确以空载荷 WeAPI 调用 `/api/v1/user/detail/{uid}`；统一 `UserProfile` 映射身份、等级、听歌数、社交/歌单计数、生日、注册时间、背景与公开态，完整平台资料和原始响应不丢失；请求构造、映射、空包装/文本/零时间戳回退、ID 一致性、无效等级、账户选择、查询边界及统一 HTTP 均有测试；2026-07-22 公开用户真实 HTTP 返回 200 和完整非空资料） |
| `user_detail_new` | `/user/detail/new` | `verified` | `GET /v1/users/{ref}`（缺省 `backend=modern`，也接受 `new/eapi/v2`）及 `GET /v1/account/profile`（独立 `UserProfileModern` 能力，精确以 EAPI 调用 `/api/w/v1/user/detail/{uid}` 并提交字符串 `all=true/userId`；与 legacy 共用稳定模型但以 `extensions.backend/response` 明确保留后端和完整未来字段；2026-07-22 公开用户及持久化 `manual-sms` 账户的统一 HTTP 均真实返回 200，账户路径正确解析所选账户自己的用户 ID） |
| `user_dj` | `/user/dj` | `pending` | — |
| `user_event` | `/user/event` | `pending` | — |
| `user_follow_mixed` | `/user/follow/mixed` | `pending` | — |
| `user_followeds` | `/user/followeds` | `pending` | — |
| `user_follows` | `/user/follows` | `pending` | — |
| `user_level` | `/user/level` | `pending` | — |
| `user_medal` | `/user/medal` | `pending` | — |
| `user_mutualfollow_get` | `/user/mutualfollow/get` | `pending` | — |
| `user_playlist` | `/user/playlist` | `implemented` | `GET /v1/account/playlists`（2026-07-17 持久化真实账户内容成功返回，但请求 `limit=5` 时上游仍返回完整列表；待收口并验证真实分页契约） |
| `user_playlist_collect` | `/user/playlist/collect` | `pending` | — |
| `user_playlist_create` | `/user/playlist/create` | `pending` | — |
| `user_record` | `/user/record` | `verified` | `GET /v1/account/history`、`GET /v1/users/{ref}/history`（`all_time/week`；2026-07-17 持久化真实账户实测全部历史返回 5 项，周历史成功返回空列表） |
| `user_replacephone` | `/user/replacephone` | `pending` | — |
| `user_social_status` | `/user/social/status` | `pending` | — |
| `user_social_status_edit` | `/user/social/status/edit` | `pending` | — |
| `user_social_status_rcmd` | `/user/social/status/rcmd` | `pending` | — |
| `user_social_status_support` | `/user/social/status/support` | `pending` | — |
| `user_subcount` | `/user/subcount` | `pending` | — |
| `user_update` | `/user/update` | `pending` | — |
| `verify_getQr` | `/verify/getQr` | `pending` | — |
| `verify_qrcodestatus` | `/verify/qrcodestatus` | `pending` | — |
| `video_category_list` | `/video/category/list` | `verified` | `GET /v1/videos/taxonomy?kind=categories`（独立 `VideoTaxonomy` 能力；固定 WeAPI `/api/cloudvideo/category/list`，精确提交参考 `offset/total="true"/limit`，统一映射字符串 ID、名称、跳转 URL、选中态、关联视频类型及完整条目；上游返回数超过请求 limit 时明确 `limit_applied=false`，不截断伪装；协议、分页、畸形条目、能力发现与统一 HTTP 均有测试；2026-07-22 匿名真实 HTTP 返回 9 项分类） |
| `video_detail` | `/video/detail` | `verified` | `GET /v1/videos/{ref}?kind=video`（不透明视频 ID、标题、创作者、描述、封面、时长、发布时间、播放量、收藏态与资源档位完整映射；2026-07-22 从真实账户收藏列表取得当前有效视频并验证详情返回 200、4 档分辨率及正时长） |
| `video_detail_info` | `/video/detail/info` | `verified` | `GET /v1/videos/{ref}/stats?kind=video`：旧样本仍真实返回点赞、评论和分享统计，映射已验收 |
| `video_group` | `/video/group` | `verified` | `GET /v1/videos?catalog=group&group_id=...`（固定 WeAPI `/api/videotimeline/videogroup/otherclient/get`，精确提交数值 `groupId`、参考字符串 `need_preview_url="true"`、布尔 `total=true` 与 offset，不提交虚构 limit；复用完整站内 `Video` 映射并保留时间线包装；`datas=null` 合法规范化为空页，缺失/错误容器仍拒绝；非空/空/畸形、筛选冲突和统一 HTTP 均有测试；2026-07-22 从真实时间线及分类/标签累计发起 63 次 group 请求，均返回 `code=200,datas=null,hasmore=false`） |
| `video_group_list` | `/video/group/list` | `verified` | `GET /v1/videos/taxonomy?kind=groups`（固定 WeAPI `/api/cloudvideo/group/list` 与空载荷；返回完整标签目录且不接受不存在的非零 offset，统一 `total` 为实际项数、`continuation_supported=false/limit_applied=false`；条目、协议、分页边界和 HTTP 均有测试；2026-07-22 匿名真实 HTTP 返回 107 项标签） |
| `video_sub` | `/video/sub` | `verified` | `PUT/DELETE /v1/account/library/videos/{ref}?kind=video`（与 MV 分支共用统一 `SubscriptionResult`，但严格分派至 WeAPI `/api/cloudvideo/video/sub|unsub` 并只提交不透明字符串 `id`；账户别名、资源类型、完整响应与查询边界均保留/验证；2026-07-22 对真实账户已有普通视频完成原始已收藏→DELETE 取消→PUT 恢复闭环，最终列表确认原条目仍存在） |
| `video_timeline_all` | `/video/timeline/all` | `verified` | `GET /v1/videos?catalog=timeline_all`（固定 WeAPI `/api/videotimeline/otherclient/get`，精确提交 `groupId=0/offset/need_preview_url="true"/total=true`；不提交参考模块不存在的 limit，按 `hasmore` 与真实返回数生成下一 offset，外层算法包装、内层视频、创作者、收藏态和完整响应均保留；2026-07-22 持久化真实账户统一 HTTP 返回 8 项、`has_more=true/next_offset=8`） |
| `video_timeline_recommend` | `/video/timeline/recommend` | `verified` | `GET /v1/videos?catalog=timeline_recommended`（固定 WeAPI `/api/videotimeline/get`，精确提交参考 `offset/filterLives="[]"/withProgramInfo="true"/needUrl="1"/resolution="480"`；不伪造 limit，完整映射 `datas[].data` 并保留外层算法；2026-07-22 持久化真实账户统一 HTTP 返回 8 项、`has_more=true/next_offset=8`） |
| `video_url` | `/video/url` | `verified` | `GET /v1/videos/{ref}/stream?kind=video&resolution=...` 与 `/redirect`（离线覆盖成功、空 URL 与多兼容字段；2026-07-22 使用账户收藏中的当前有效普通视频真实返回 480p 非空播放 URL、`available=true/actual_resolution=480`，统一重定向返回 302） |
| `vip_growthpoint` | `/vip/growthpoint` | `pending` | — |
| `vip_growthpoint_details` | `/vip/growthpoint/details` | `pending` | — |
| `vip_growthpoint_get` | `/vip/growthpoint/get` | `pending` | — |
| `vip_growthpoint_getall` | `/vip/growthpoint/getall` | `pending` | — |
| `vip_info` | `/vip/info` | `verified` | `GET /v1/users/{ref}/membership`、`GET /v1/account/membership`（统一 `MembershipSummary` 稳定表达用户引用、等级、激活态、年费次数、到期时间和图标，平台未明确提供的 `active/expires_at` 保持 `null` 而不按等级猜测；公开用户引用决定平台，当前账户由 `platform/account` 选择，完整覆盖参考 `uid` 指定用户和缺省空字符串两分支；固定 WeAPI `/api/music-vip-membership/front/vip/info` 并精确提交字符串 `userId`，映射 `redVipLevel/redVipAnnualCount/redVipLevelIcon`，动态图标和完整响应保留在扩展；核心可空契约、双协议负载、成功/畸形/数值越界、能力发现、公开/账户端点、默认选择和输入拒绝均有测试；2026-07-17 显式 provider 联网测试及真实二进制 HTTP 验证公开用户 `netease:32953014` 返回上游 `code=200`、等级 7、年费次数 -1，省略 `uid` 的当前账户匿名分支稳定返回 401 `authentication_required` 与上游码 301） |
| `vip_info_v2` | `/vip/info/v2` | `implemented` | `GET /v1/users/{ref}/membership?backend=client`、`GET /v1/account/membership?backend=client`（以显式后端和独立 `UserMembershipClientInfo` 能力与公开 `vip_info` 分开，不静默回退；兼容 `variant/source` 字段及 `detail/v2` 值；固定 WeAPI `/api/music-vip-membership/client/vip/info`，精确提交指定用户字符串 ID 或当前账户空字符串，并在请求前要求所选 `account` 已登录；统一 `MembershipSummary` 映射 `userId/redVipLevel/redVipAnnualCount`，从 `redplus/musicPackage/associator/voiceBookVip/albumVip` 取最长有效期并结合上游 `now` 计算激活态，空图标不会遮蔽后续非空动态/静态图标，完整五类权益和未来字段保留在原始响应扩展；协议、独立能力、登录前置、当前/公开用户、有效/过期、等级回退、秒/毫秒时间、空图标优先级、畸形响应、数值越界、统一端点及未知后端/字段拒绝均有测试，待 Basic 末尾用持久化真实账户验证成功态） |
| `vip_sign` | `/vip/sign` | `pending` | — |
| `vip_sign_detail` | `/vip/sign/detail` | `pending` | — |
| `vip_sign_history` | `/vip/sign/history` | `pending` | — |
| `vip_sign_info` | `/vip/sign/info` | `pending` | — |
| `vip_tasks` | `/vip/tasks` | `pending` | — |
| `vip_tasks_v1` | `/vip/tasks/v1` | `pending` | — |
| `vip_timemachine` | `/vip/timemachine` | `pending` | — |
| `voice_delete` | `/voice/delete` | `implemented` | `DELETE /v1/account/podcast-episodes/{ref}`、`DELETE /v1/account/podcast-episodes`（独立 `PodcastEpisodeDeleteWrite` 能力与稳定批量请求/结果；单条引用由路径决定平台，批量 `refs` 接受完整引用数组或逗号串，`ids` 接受所选平台的裸 ID 数组或逗号串，并兼容 `episode_refs/episodeRefs/episodes/programs/id/programIds/voiceIds`；严格拒绝空输入、`refs+ids` 冲突、完整引用同时指定 platform、空逗号项、畸形值、混合平台和未知字段，顺序及重复项不擅自改变；网易云侧要求全部引用属于本平台且为数字 ID，固定 EAPI `/api/content/voice/delete`，精确把有序 ID 拼成参考 `ids` 逗号字符串，要求 `account` 选择已登录隔离会话，成功结果保留删除标记、全部引用和完整响应；参考文档把 ids 误称为 voiceListId，但实际路径与同协议方法均明确删除声音，统一层未错误删除声音歌单；核心序列化、能力发现、协议、多 ID 映射、上游错误、账户前置、单条/批量 HTTP、别名/冲突/平台边界均有测试；2026-07-22 空凭据真实服务器验证缺失账户别名在发网前稳定返回 401，破坏性成功态待可丢弃的自有声音验证） |
| `voice_detail` | `/voice/detail` | `implemented` | `GET /v1/episodes/{ref}?backend=workbench`（与缺省 `/dj/program/detail` 共用稳定 `PodcastEpisode` 输出，但以显式后端和独立 `PodcastEpisodeWorkbenchDetail` 能力保留创作者工作台语义，不静默合并两套上游功能；固定 EAPI `/api/voice/workbench/voice/detail` 并只提交数字 `id`，要求由 `account` 选择已登录隔离会话；按非空优先级解包 `data.voice/data/voice`，兼容 `voiceId/programId`、`songName`、`radioId/voiceListId`、`songId/trackId`、`durationMs`、`publishTime`、`orderNo`、`voiceFeeType` 和 `creator`，节目、所属播客及承载音频引用仍严格分离，完整工作台状态和未来字段保留在扩展；协议、能力、认证前置、包装优先级、全部别名、畸形响应、统一端点后端别名及未知字段拒绝均有测试；2026-07-18 匿名真实原始 API 以 `2058695201/1367665101` 验证上游均返回 301，稳定映射 401 `authentication_required`，成功详情留待 Basic 末尾使用持久化账户集中验收） |
| `voice_lyric` | `/voice/lyric` | `verified` | `GET /v1/episodes/{ref}/lyrics`（EAPI `/api/voice/lyric/get`；节目与音频引用分离，受限下载网易媒体域名的完整 JSON 转写并生成句段 LRC，逐词/说话人原文保留于 `word_synced`；2026-07-17 provider 与真实 HTTP 实测 `2058695201` 返回 675 段、约 1.6 MB 转写，`1367665101` 正确保留 `data=null` 无歌词成功态） |
| `voice_upload` | `/voice/upload` | `pending` | — |
| `voicelist_detail` | `/voicelist/detail` | `implemented` | `GET /v1/podcasts/{ref}?backend=workbench`（与缺省 `/dj/detail` 共用稳定 `Podcast`，但用显式后端和独立 `PodcastWorkbenchDetail` 能力保留创作者声音歌单工作台语义；固定 EAPI `/api/voice/workbench/voicelist/detail` 并只提交数字 `id`，要求 `account` 选择已登录隔离会话；按可成功映射的优先级解包 `data.voiceList/data.voicelist/data/voiceList/voicelist`，无效高优先包装不会遮蔽有效兼容对象，兼容 `voiceListId/radioId`、`coverImgUrl`、`creator`、`categoryName`、`voiceCount` 和 `voiceFeeType`，完整发布状态与未来字段保留在扩展；完整主播对象优先于 `creatorName` 摘要，不会丢失用户引用和头像；协议、能力、认证前置、全部包装/别名、冲突优先级、畸形响应、统一端点后端别名和未知字段拒绝均有测试；2026-07-18 匿名真实原始 API 以 `336355127` 验证上游返回 301 并稳定映射 401 `authentication_required`，成功详情留待 Basic 末尾使用持久化账户集中验收） |
| `voicelist_list` | `/voicelist/list` | `implemented` | `GET /v1/podcasts/{ref}/episodes?backend=workbench`（与缺省公开 `/dj/program` 目录共用稳定 `PodcastEpisode` 列表，但以显式后端和独立 `PodcastEpisodeWorkbenchList` 能力保留创作者工作台语义，不静默回退；固定 EAPI `/api/voice/workbench/voices/by/voicelist`，精确提交 `voiceListId/limit/offset`，要求 `account` 选择已登录隔离会话；工作台缺省和最大 limit 均为参考实现的 200，明确拒绝上游不支持的升序控制；按可成功映射的顺序兼容 `data.list/data.voices/data.records/data/list/voices`，空或畸形高优先数组不会遮蔽非空有效兼容数组，嵌套 `voice` 与外层审核字段无损合并且声音对象自身字段优先；`voiceId/programId`、`voiceListId/radioId`、`songId/trackId` 分别保持节目、声音歌单和承载音频身份，`total/count` 与 `hasMore/more` 驱动真实分页；协议、能力、认证前置、200 条边界、升序拒绝、全部包装/别名、冲突优先级、畸形响应、统一端点后端别名及未知字段拒绝均有测试；2026-07-18 匿名真实原始 API 以声音歌单 `336355127` 验证上游返回 301 并稳定映射 401 `authentication_required`，成功列表留待 Basic 末尾使用持久化账户集中验收） |
| `voicelist_list_search` | `/voicelist/list/search` | `implemented` | `GET /v1/account/podcast-episodes`（作为登录账户创作者工作台查询与公开声音搜索严格分离，以独立 `PodcastEpisodeWorkbenchSearch` 能力返回稳定 `PodcastEpisode[]`；固定 EAPI `/api/voice/workbench/voice/list`，完整保留参考 `name/displayStatus/type/voiceFeeType/radioId/limit/offset`，统一参数及原参数别名均可用，未指定筛选精确提交 null；审核状态覆盖 `AUDITING/ONLY_SELF_SEE/ONLINE/SCHEDULE_PUBLISH/TRANSCODE_FAILED/PUBLISHING/FAILED` 全部分支，可见性覆盖 `PUBLIC/PRIVATE`，付费筛选覆盖 `-1/0/1`，可限定裸播客 ID 或同平台完整引用；要求 `platform/account` 选择已登录隔离会话，最大 200 条，跨平台播客引用和未知字段在请求前拒绝；复用工作台声音映射，节目/播客/承载音频三类身份及审核包装不丢失；核心类型、能力、精确协议/全状态/空筛选、认证与 ID 前置、统一和参考参数、全部过滤边界、跨平台冲突、未知字段及 HTTP 包络均有测试；2026-07-18 匿名真实原始 API 验证上游返回 301 并稳定映射 401 `authentication_required`，成功查询留待 Basic 末尾使用持久化账户集中验收） |
| `voicelist_my_created` | `/voicelist/my/created` | `implemented` | `GET /v1/account/podcasts/created`（以独立 `AccountCreatedPodcasts` 能力和稳定 `Podcast[]` 表达账户创作目录，与订阅库严格分开；固定 WeAPI `/api/social/my/created/voicelist/v1` 并只提交参考 `limit`，缺省 20，要求 `platform/account` 选择已登录隔离会话；上游不存在 offset，因此统一端点拒绝 offset，分页固定 `offset=0/next_offset=null/has_more=false` 并显式记录 `continuation_supported=false`，不伪造可续页；兼容 `data.list/data.voiceLists/data.voicelists/data.records/data/voiceLists/voicelists/list` 及嵌套 `voiceList/voicelist/baseInfo`，空旧列表不遮蔽后续非空内容，内层播客字段优先且外层审核/未来字段无损合并；协议、能力、认证与 offset 前置、包装优先级、空快照、畸形响应、统一端点及未知字段拒绝均有测试；2026-07-18 匿名真实原始 API 验证上游返回 301 并稳定映射 401 `authentication_required`，成功快照留待 Basic 末尾使用持久化账户集中验收） |
| `voicelist_search` | `/voicelist/search` | `verified` | `GET /v1/search?type=podcast`（统一新增 `Podcast` 搜索类型和 `SearchPodcasts` 能力；缺省 `variant=default` 精确使用 EAPI `/api/search/voicelist/get` 并提交 `keyword/scene=normal/limit/offset/e_r=true`，同时保留 `legacy/cloud` 的 1009 兼容后端；解包 `data.resources[].baseInfo` 为完整 `Podcast`，排名算法与命中理由保存在 `extensions.search_item`，以 `totalCount/hasMore` 驱动统一分页；Web 搜索建议、多重匹配和数字类型 1009 同步改正为播客语义，直播广播搜索会明确返回能力不支持；核心模型、全部协议分支、包装映射、错误边界、别名和 HTTP 参数均有测试；2026-07-18 匿名真实 provider 联网以“故事”查询成功返回类型化播客、上游 `code=200`，并确认实际路径为 `/api/search/voicelist/get`） |
| `voicelist_trans` | `/voicelist/trans` | `implemented` | `PUT /v1/account/podcasts/{ref}/episodes/order`（独立 `PodcastEpisodeOrderWrite` 能力与稳定 `PodcastEpisodeOrderRequest/Result`；路径播客引用、声音引用与目标位置严格分离，声音兼容裸 ID、完整引用及 `episode_ref/episodeRef/program_id/programId/id`，跨平台或畸形引用在发网前拒绝；`position` 缺省 1 且最小 1，`limit/offset` 缺省 200/0 并保留参考接口的最大 200 条工作台分页语义；固定 EAPI `/api/voice/workbench/radio/program/trans`，精确提交 `limit/offset/radioId/programId/position`，要求 `account` 选择已登录隔离会话；成功结果保留声音歌单、声音、位置及完整响应；核心契约、能力、协议、映射、认证、同平台约束、全部别名、数值边界、未知字段和统一 HTTP 均有测试；2026-07-22 真实匿名协议请求到达上游并返回 `code=400`“只允许操作自己的播客”，统一端点缺失账户别名返回 401，拥有者成功重排待创作者账户验证） |
| `weblog` | `/weblog` | `pending` | — |
| `yunbei` | `/yunbei` | `pending` | — |
| `yunbei_expense` | `/yunbei/expense` | `pending` | — |
| `yunbei_info` | `/yunbei/info` | `pending` | — |
| `yunbei_rcmd_song` | `/yunbei/rcmd/song` | `pending` | — |
| `yunbei_rcmd_song_history` | `/yunbei/rcmd/song/history` | `pending` | — |
| `yunbei_receipt` | `/yunbei/receipt` | `pending` | — |
| `yunbei_sign` | `/yunbei/sign` | `pending` | — |
| `yunbei_task_finish` | `/yunbei/task/finish` | `pending` | — |
| `yunbei_tasks` | `/yunbei/tasks` | `pending` | — |
| `yunbei_tasks_todo` | `/yunbei/tasks/todo` | `pending` | — |
| `yunbei_today` | `/yunbei/today` | `pending` | — |
