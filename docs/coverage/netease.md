# 网易云 API 全量覆盖账本

上游快照：`NeteaseCloudMusicApiEnhanced/api-enhanced@6946dc8e14b6fb125191bc43525d4faa8123d8ae`

本表由该快照的 `module/*.js` 文件生成，共 404 项。它是完成度验收清单，不是功能推荐列表。状态含义：

- `pending`：尚未完成统一映射或平台扩展端点。
- `partial`：已有一部分统一能力，但仍缺输入、输出、分支或真实验证。
- `implemented`：代码和离线测试已完成，仍需要带真实前置条件的联网验证。
- `verified`：统一端点、测试和对应真实网络路径均已验证。

当前统计：`pending=341`、`partial=7`、`implemented=21`、`verified=35`。只有所有条目都达到 `verified`，或以证据明确标为上游已失效，网易云阶段才算完成。

| 上游模块 | 参考路由 | 状态 | TuneWeave 映射/缺口 |
| --- | --- | --- | --- |
| `activate_init_profile` | `/activate/init/profile` | `pending` | — |
| `aidj_content_rcmd` | `/aidj/content/rcmd` | `pending` | — |
| `album` | `/album` | `verified` | `GET /v1/albums/{ref}`、`GET /v1/albums/{ref}/tracks`（2026-07-16 HTTP 实测 `netease:18915` 返回《范特西》及 10 首曲目） |
| `album_detail` | `/album/detail` | `verified` | `GET /v1/digital-albums/{ref}`（与 `/digitalAlbum/detail` 共用上游协议；2026-07-16 HTTP 实测 `netease:120605500` 返回《冀西南林路行》及 22 CNY 商品信息） |
| `album_detail_dynamic` | `/album/detail/dynamic` | `verified` | `GET /v1/albums/{ref}/stats`（2026-07-16 HTTP 实测 `netease:32311` 返回收藏状态、71671 收藏、1989 评论及 9306 分享） |
| `album_list` | `/album/list` | `verified` | `GET /v1/digital-albums`（`area/type` 筛选；2026-07-16 HTTP 实测返回 2 项、首项 `netease:387169747`《小海子村儿》，上游未给总数故 `total=null`） |
| `album_list_style` | `/album/list/style` | `verified` | `GET /v1/digital-albums?catalog=style`（`ZH/EA` 统一区域值映射到上游 `Z_H/E_A`；2026-07-16 HTTP 实测返回 2 项并保留销量与购买状态） |
| `album_new` | `/album/new` | `verified` | `GET /v1/albums?catalog=new`（`area` 筛选；2026-07-16 匿名 HTTP 实测返回 2 项、总数 500） |
| `album_newest` | `/album/newest` | `verified` | `GET /v1/albums?catalog=newest`（2026-07-16 匿名 HTTP 实测首页共 12 项，统一分页返回前 2 项） |
| `album_privilege` | `/album/privilege` | `verified` | `GET /v1/albums/{ref}/track-entitlements`（2026-07-16 匿名 HTTP 实测 `netease:168223858` 共 10 项，首项 `netease:2058263030` 可播 320 kbps、最高 999 kbps，并保留无损及 Hi-Res 权益） |
| `album_songsaleboard` | `/album/songsaleboard` | `verified` | `GET /v1/charts/digital-albums`（完整支持 `daily/week/year/total` 与数字专辑/数字单曲；2026-07-16 HTTP 实测 2025 年数字单曲榜共 10 项，首项 `netease:83848829`《好想爱这个世界啊》，销量 316218） |
| `album_sub` | `/album/sub` | `implemented` | `PUT/DELETE /v1/account/library/albums/{ref}`（收藏与取消收藏路径均已实现；2026-07-16 匿名 HTTP 实测正确映射为 401 `authentication_required`，待真实账户验证成功写入） |
| `album_sublist` | `/album/sublist` | `implemented` | `GET /v1/account/library/albums`（分页、收藏时间与 `paidCount` 等元数据已完整映射；2026-07-16 匿名 HTTP 实测正确映射为 401 `authentication_required`，待真实账户验证内容成功态） |
| `api` | `/api` | `verified` | `POST /v1/extensions/netease/api`（仅允许固定网易云域名与 `/api/...` 路径，登录态由 `account` 别名选择；完整支持默认 EAPI 及 `weapi/api/linuxapi/xeapi`，拒绝原始 Cookie、域名、代理和请求头覆盖；2026-07-16 五种协议均以搜索请求联网实测成功，另实测 XEAPI 公钥注册/加密响应及 `e_r=true` EAPI 响应解密） |
| `artist_album` | `/artist/album` | `verified` | `GET /v1/artists/{ref}/albums`（统一分页并保留歌手级元数据与单项原始字段；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 5 张周杰伦专辑，首项 `netease:274336916`《即兴曲》，`has_more=true`、`next_offset=5`） |
| `artist_desc` | `/artist/desc` | `verified` | `GET /v1/artists/{ref}`（与 `/artist/detail` 合并为统一 `Artist`，映射简介及分段传记，并在扩展字段保留专题原始响应；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回周杰伦简介、6 段传记及 3 项专题数据） |
| `artist_detail` | `/artist/detail` | `verified` | `GET /v1/artists/{ref}`（映射名称、别名、身份、头像、封面及作品计数，并保留完整原始响应；2026-07-16 匿名 HTTP 实测返回 44 张专辑、568 首曲目、9 个 MV 与 8 个视频） |
| `artist_detail_dynamic` | `/artist/detail/dynamic` | `verified` | `GET /v1/artists/{ref}/stats`（统一关注态、视频分类计数与在线演出计数，未知平台类别保留原始标识，演出及推荐对象保留完整响应；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 `followed=false`、分类 `0:9/1:9`、在线演出数 0） |
| `artist_fans` | `/artist/fans` | `verified` | `GET /v1/artists/{ref}/fans`（统一为分页 `User[]`，昵称、头像、签名和关系状态进入稳定字段，地区、认证、VIP 等完整资料保留在单项扩展；2026-07-16 匿名 HTTP 实测 `netease:2116` 返回 2 位粉丝、`next_offset=2`、`has_more=true`，上游无总数故 `total=null`） |
| `artist_follow_count` | `/artist/follow/count` | `verified` | `GET /v1/artists/{ref}/stats`（统一粉丝总数和账户关注态，日增量等附加字段保留完整响应；2026-07-16 匿名 HTTP 实测 `netease:2116` 返回 `follower_count=13704933`、`followed=false`，统一值与上游 `fansCnt/isFollow` 一致） |
| `artist_list` | `/artist/list` | `verified` | `GET /v1/artists`（统一 `type=all/male/female/group`、六类 `area` 与 `initial=a..z/hot/other`，条目映射为 `Artist` 并保留完整目录字段；2026-07-16 匿名 HTTP 实测 `type=male&area=western&initial=b&limit=2` 返回 Bruno Mars 与 bbno$，首项 50 张专辑/959 首歌曲，`next_offset=2`、`has_more=true`） |
| `artist_mv` | `/artist/mv` | `verified` | `GET /v1/artists/{ref}/videos?type=mv`（统一为分页 `Video[]`，映射创作者、16:9 封面、时长、发布日期、播放数和收藏态，并保留完整 MV 与响应时间；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 2 项，首项 `netease:22695250`《任性 (5525 Live版)》、266000 ms、100726 播放，`next_offset=2`、`has_more=true`） |
| `artist_new_mv` | `/artist/new/mv` | `implemented` | `GET /v1/account/following/artists/new-videos`（以 `platform/account` 选择登录态，`before` 毫秒时间戳翻页，统一映射为 `Video[]` 并保留完整响应；离线成功态映射、端点和登录别名测试已完成；2026-07-16 匿名 HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功内容） |
| `artist_new_song` | `/artist/new/song` | `implemented` | `GET /v1/account/following/artists/new-tracks`（以 `platform/account` 选择登录态，`before` 毫秒时间戳翻页，统一为 `Track[]`，保留新曲总数及完整歌曲原文；离线成功态映射、端点和登录别名测试已完成；2026-07-16 匿名 HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功内容） |
| `artist_new_song_mv_list_v2` | `/artist/new/song/mv/list/v2` | `implemented` | `GET /v1/account/following/artists/new-works`（完整支持 `before/source_type/first_request`，以 `ArtistWorkUpdate` 区分歌曲、MV 和未知来源，未知结构完整保留；离线已验证歌曲分支、未知来源兼容和端点参数；2026-07-16 匿名 EAPI HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功内容及更多 `sourceType` 样本） |
| `artist_new_song_playall` | `/artist/new/song/playall` | `implemented` | `GET /v1/account/following/artists/new-tracks/play-all`（固定返回最近至多 50 首 `Track[]` 和上游 `count`，完整歌曲字段保留在扩展；离线成功态映射、账户端点和能力发现测试已完成；2026-07-16 匿名 EAPI HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功内容） |
| `artist_songs` | `/artist/songs` | `verified` | `GET /v1/artists/{ref}/tracks`（完整支持 `order=hot/time`、分页及 `account` 登录态选择，统一为 `Track[]` 并在 `extensions.artist_track` 保留完整歌曲原文；2026-07-16 真实上游与统一 HTTP 均实测成功，`/v1/artists/netease:6452/tracks?order=time&limit=2` 首项为 `netease:2712553851`《即兴曲》，总数 566、`next_offset=2`、`has_more=true`） |
| `artist_sub` | `/artist/sub` | `implemented` | `PUT/DELETE /v1/account/following/artists/{ref}`（关注与取消关注共用统一 `SubscriptionResult`，登录态由引用平台和 `account` 别名选择，完整上游响应保留在扩展；请求构造、成功态映射、非法 ID、账户别名及 HTTP 端点测试已完成；2026-07-16 匿名 WeAPI HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功写入及回滚） |
| `artist_sublist` | `/artist/sublist` | `implemented` | `GET /v1/account/following/artists`（统一为分页 `Artist[]`，支持 `platform/account/limit/offset`，名称、别名、封面及作品计数进入稳定字段，关注时间和完整歌手原文保留在 `extensions.following_item`；离线成功态映射、账户别名和 HTTP 端点测试已完成；2026-07-16 匿名 WeAPI HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，待真实账户验证成功内容） |
| `artist_top_song` | `/artist/top/song` | `verified` | `GET /v1/artists/{ref}/top-tracks`（固定热门 50 首快照，不接收伪分页参数；歌曲与独立权益按 ID 合并为统一 `Track[]`，`has_more=false`，单项原文和完整响应均保留；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回 50 首，首项 `netease:210049`《布拉格广场》（蔡依林 / 周杰伦），`total=50`、`next_offset=null`） |
| `artist_video` | `/artist/video` | `verified` | `GET /v1/artists/{ref}/videos?type=all`（统一为游标分页 `Video[]`，映射标题、创作者、封面、时长、发布时间和播放数，原始 Mlog 资源完整保留；2026-07-16 匿名 HTTP 实测 `netease:2116` 连续两页各返回 2 项，游标由 `2` 前进至 `4` 且资源无重复，首项 `netease:34702399`《K歌之王 AIR (Day Version / Lyric Video / China Version)》） |
| `artists` | `/artists` | `verified` | `GET /v1/artists/{ref}/overview`（统一为 `ArtistOverview`，明确分离歌手摘要、50 首精选 `Track[]` 和 `has_more_tracks`，不与 `/artist/list` 或完整曲目目录误合并；歌手、曲目及完整响应原文分别保留；2026-07-16 匿名 HTTP 实测 `netease:6452` 返回周杰伦、568 首总曲目计数、50 首精选，首项 `netease:210049`《布拉格广场》，`has_more_tracks=true`） |
| `audio_match` | `/audio/match` | `implemented` | `POST /v1/audio/recognize`（统一 `platform/account/fingerprint/duration_seconds` 输入，兼容参考项目 `audioFP/duration` 字段；多候选曲目、命中起点、查询 ID、无匹配原因和完整上游响应均已映射；离线成功命中样本、输入边界及 HTTP 端点测试已完成；2026-07-16 匿名 HTTP 实测无匹配路径返回 `code=200`、空 `matches`、`no_match_reason=10` 与真实查询 ID，待有效音频指纹验证真实成功命中） |
| `avatar_upload` | `/avatar/upload` | `implemented` | `PUT /v1/account/avatar`（统一以 `platform/account/filename` 查询参数、`Content-Type: image/*` 和原始图片请求体写入，最大 20 MiB；完整实现 WeAPI 申请 `yyimgs` NOS 凭据、原始字节上传及 EAPI 提交 `imgId` 三段流程，统一返回 URL/图片 ID，NOS token 不进入响应或日志；兼容 `imgSize/imgX/imgY` 参数并明确记录参考实现未实际应用裁剪；离线映射、认证前置、参数别名、大小边界、标准错误包络及 token 防泄漏测试已完成；2026-07-16 匿名 HTTP 实测在 NOS 分配前稳定返回 401 `authentication_required`，待真实账户验证最终写入） |
| `banner` | `/banner` | `verified` | `GET /v1/banners`（完整支持 `client=pc/android/iphone/ipad`，并兼容参考项目 `type=0/1/2/3`；图片、标题、横幅 ID、跳转 URL、独家标志及歌曲/专辑/歌手/歌单/MV/网页/未知目标进入稳定字段，监测和广告等完整原文保留在 `extensions.banner`；2026-07-16 适配器与统一 HTTP 均逐分支联网实测成功，PC 7 项、Android 8 项、iPhone 8 项、iPad 6 项，首项目标分别正确映射为网页或 `netease:384808686` 专辑） |
| `batch` | `/batch` | `verified` | `GET/POST /v1/extensions/netease/batch`（完整保留任意 `/api/...` 子请求及逐项原始响应，支持参考 GET 查询键、POST 顶层动态键和 `requests` 结构化容器；对象值自动序列化为上游真实要求的 JSON 文本，预序列化字符串原样保留；完整支持 `eapi/weapi/api/linuxapi/xeapi`、`crypto/protocol`、`e_r/encrypted_response` 与 `account`，逐路径限制固定网易云域名并拒绝 Cookie、域名、代理、请求头和 IP 等传输注入；2026-07-16 适配器及统一 HTTP 对五种协议均联网实测顶层/子请求 `code=200`，每种取得 7 条横幅，参考 GET 形态加 `e_r=true` 亦成功解密并返回 7 条，不存在的账户别名实测为 401） |
| `broadcast_category_region_get` | `/broadcast/category/region/get` | `verified` | `GET /v1/radio/taxonomy`（统一为 `RadioTaxonomy`，分类与地区 ID 均保持平台不透明字符串，单项及完整响应原文保留在扩展中，可直接供后续广播电台列表筛选；支持 `platform/account` 选择且公开响应无需登录；2026-07-16 适配器与统一 HTTP 均联网实测成功，返回 12 个分类和 32 个地区，首项分别为 `1`“音乐台”与 `407`“网络台”，原始上游 `code=200`） |
| `broadcast_channel_collect_list` | `/broadcast/channel/collect/list` | `implemented` | `GET /v1/account/library/radio-stations`（以 `platform/account` 选择登录态，完整提交参考实现的 `contentType/timeReverseOrder/startDate/limit`，并补齐参考接口声明的 `offset` 分页；统一为 `RadioStation[]`，兼容对象及 JSON 字符串嵌套条目，收藏项、频道原文和完整分页响应分别保留在扩展中；离线成功态映射、缺失列表错误、账户别名隔离、端点与分页契约测试已完成；2026-07-16 匿名 provider 及统一 HTTP 实测稳定返回 401 `authentication_required` 与上游码 301，匿名注册接口另实测业务码 400、未取得可用 Cookie，待真实账户验证收藏内容成功态） |
| `broadcast_channel_currentinfo` | `/broadcast/channel/currentinfo` | `verified` | `GET /v1/radio/stations/{ref}`（以资源引用选择平台、`account` 选择可选登录态，统一为 `RadioStation`；名称、封面、地区、当前节目与直播音频地址进入稳定字段，第三方频道/节目 ID、时间窗口及完整响应保留在扩展中，公开响应未给收藏态时严格保持 `null`；无符号整数 ID 在网络请求前校验；2026-07-16 provider 与统一 HTTP 均联网实测 `netease:362` 成功，返回“金山区广播电视台综合广播”、地区“上海”、可用的 `https://lhttp.qtfm.cn/live/4022/64k.mp3...` 音频地址及上游 `code=200`） |
| `broadcast_channel_list` | `/broadcast/channel/list` | `verified` | `GET /v1/radio/stations`（完整支持 `categoryId/regionId/limit/lastId/score` 及 snake_case 别名；分类、地区和电台 ID 保持字符串，`lastId+score` 统一为成对游标并在分页扩展返回 `next_cursor`，两字段独立出现时分别补参考默认 `0/-1`；参考类型公开但实现忽略的 `offset` 仍被接收，并明确返回 `requested_offset` 与 `offset_applied=false`；首屏推荐插入导致返回数大于 `limit` 时不截断，完整频道和响应原文保留；2026-07-16 provider 与统一 HTTP 均联网实测：音乐分类 `categoryId=1` 首/二页各 20 项、总数 105、两页零重复，首屏下一游标 `{id:965,score:1139}`；网络台 `regionId=407` 返回 4/4 项且全部地区为“网络台”、`has_more=false`；`offset=100` 实测不改变上游首屏并正确标记未应用，上游均为 `code=200`） |
| `broadcast_sub` | `/broadcast/sub` | `implemented` | `PUT/DELETE /v1/account/library/radio-stations/{ref}`（参考 `t=1` 的收藏分支完整映射为 `contentType=BROADCAST`、`cancelCollect=false`，其余 `t` 值的取消分支映射为 `cancelCollect=true`；统一端点以 HTTP 方法明确表达两种语义，电台 ID 在网络前校验，`account` 选择隔离登录态，统一返回 `SubscriptionResult` 并保留完整上游响应；离线请求构造、成功响应映射、非法 ID、缺失账户别名、PUT/DELETE 路由与响应契约均已测试；2026-07-16 provider 和统一 HTTP 对收藏/取消两条匿名路径均联网实测为 401 `authentication_required` 与上游码 301，待真实账户验证成功写入） |
| `calendar` | `/calendar` | `pending` | — |
| `captcha_sent` | `/captcha/sent` | `implemented` | `POST /v1/auth/challenges`（为避免误发短信不做自动联网测试） |
| `captcha_verify` | `/captcha/verify` | `partial` | 适配器已实现；统一挑战验证直接完成验证码登录 |
| `cellphone_existence_check` | `/cellphone/existence/check` | `pending` | — |
| `chart_detail` | `/chart/detail` | `pending` | — |
| `chart_song_detail` | `/chart/song/detail` | `pending` | — |
| `check_music` | `/check/music` | `pending` | — |
| `cloud` | `/cloud` | `pending` | — |
| `cloud_import` | `/cloud/import` | `pending` | — |
| `cloud_lyric_get` | `/cloud/lyric/get` | `pending` | — |
| `cloud_match` | `/cloud/match` | `pending` | — |
| `cloud_upload_complete` | `/cloud/upload/complete` | `pending` | — |
| `cloud_upload_token` | `/cloud/upload/token` | `pending` | — |
| `cloudsearch` | `/cloudsearch` | `pending` | — |
| `comment` | `/comment` | `pending` | — |
| `comment_album` | `/comment/album` | `pending` | — |
| `comment_dj` | `/comment/dj` | `pending` | — |
| `comment_event` | `/comment/event` | `pending` | — |
| `comment_floor` | `/comment/floor` | `pending` | — |
| `comment_hot` | `/comment/hot` | `pending` | — |
| `comment_hug_list` | `/comment/hug/list` | `pending` | — |
| `comment_info_list` | `/comment/info/list` | `pending` | — |
| `comment_like` | `/comment/like` | `pending` | — |
| `comment_music` | `/comment/music` | `pending` | — |
| `comment_mv` | `/comment/mv` | `pending` | — |
| `comment_new` | `/comment/new` | `pending` | — |
| `comment_playlist` | `/comment/playlist` | `pending` | — |
| `comment_report` | `/comment/report` | `pending` | — |
| `comment_video` | `/comment/video` | `pending` | — |
| `countries_code_list` | `/countries/code/list` | `pending` | — |
| `creator_authinfo_get` | `/creator/authinfo/get` | `pending` | — |
| `daily_signin` | `/daily_signin` | `pending` | — |
| `decrypt` | `/decrypt` | `pending` | — |
| `digitalAlbum_detail` | `/digitalAlbum/detail` | `verified` | `GET /v1/digital-albums/{ref}`（`/album/detail` 的公开别名，共用实现与验证证据） |
| `digitalAlbum_ordering` | `/digitalAlbum/ordering` | `pending` | — |
| `digitalAlbum_purchased` | `/digitalAlbum/purchased` | `pending` | — |
| `digitalAlbum_sales` | `/digitalAlbum/sales` | `pending` | — |
| `dj_banner` | `/dj/banner` | `pending` | — |
| `dj_category_excludehot` | `/dj/category/excludehot` | `pending` | — |
| `dj_category_recommend` | `/dj/category/recommend` | `pending` | — |
| `dj_catelist` | `/dj/catelist` | `pending` | — |
| `dj_detail` | `/dj/detail` | `pending` | — |
| `dj_difm_all_style_channel` | `/dj/difm/all/style/channel` | `pending` | — |
| `dj_difm_channel_subscribe` | `/dj/difm/channel/subscribe` | `pending` | — |
| `dj_difm_channel_unsubscribe` | `/dj/difm/channel/unsubscribe` | `pending` | — |
| `dj_difm_playing_tracks_list` | `/dj/difm/playing/tracks/list` | `pending` | — |
| `dj_difm_subscribe_channels_get` | `/dj/difm/subscribe/channels/get` | `pending` | — |
| `dj_hot` | `/dj/hot` | `pending` | — |
| `dj_paygift` | `/dj/paygift` | `pending` | — |
| `dj_personalize_recommend` | `/dj/personalize/recommend` | `pending` | — |
| `dj_program` | `/dj/program` | `pending` | — |
| `dj_program_detail` | `/dj/program/detail` | `pending` | — |
| `dj_program_toplist` | `/dj/program/toplist` | `pending` | — |
| `dj_program_toplist_hours` | `/dj/program/toplist/hours` | `pending` | — |
| `dj_radio_hot` | `/dj/radio/hot` | `pending` | — |
| `dj_recommend` | `/dj/recommend` | `pending` | — |
| `dj_recommend_type` | `/dj/recommend/type` | `pending` | — |
| `dj_sub` | `/dj/sub` | `pending` | — |
| `dj_sublist` | `/dj/sublist` | `pending` | — |
| `dj_subscriber` | `/dj/subscriber` | `pending` | — |
| `dj_today_perfered` | `/dj/today/perfered` | `pending` | — |
| `dj_toplist` | `/dj/toplist` | `pending` | — |
| `dj_toplist_hours` | `/dj/toplist/hours` | `pending` | — |
| `dj_toplist_newcomer` | `/dj/toplist/newcomer` | `pending` | — |
| `dj_toplist_pay` | `/dj/toplist/pay` | `pending` | — |
| `dj_toplist_popular` | `/dj/toplist/popular` | `pending` | — |
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
| `likelist` | `/likelist` | `implemented` | `GET /v1/account/favorites/tracks`、`GET /v1/users/{ref}/favorites/tracks`（已验证匿名请求返回登录要求；待真实账户验证） |
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
| `login_qr_create` | `/login/qr/create` | `partial` | `POST /v1/auth/qr`（返回 URL，暂不生成图片） |
| `login_qr_key` | `/login/qr/key` | `partial` | `POST /v1/auth/qr`（创建已验证） |
| `login_refresh` | `/login/refresh` | `implemented` | `POST /v1/auth/session/refresh`（待真实账户验证） |
| `login_status` | `/login/status` | `verified` | `GET /v1/auth/session`（匿名态已验证） |
| `logout` | `/logout` | `implemented` | `DELETE /v1/auth/session`（待真实账户验证） |
| `lyric` | `/lyric` | `partial` | `GET /v1/tracks/{ref}/lyrics`（由新版歌词覆盖） |
| `lyric_new` | `/lyric/new` | `verified` | `GET /v1/tracks/{ref}/lyrics` |
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
| `mv_all` | `/mv/all` | `pending` | — |
| `mv_detail` | `/mv/detail` | `pending` | — |
| `mv_detail_info` | `/mv/detail/info` | `pending` | — |
| `mv_exclusive_rcmd` | `/mv/exclusive/rcmd` | `pending` | — |
| `mv_first` | `/mv/first` | `pending` | — |
| `mv_sub` | `/mv/sub` | `pending` | — |
| `mv_sublist` | `/mv/sublist` | `pending` | — |
| `mv_url` | `/mv/url` | `pending` | — |
| `nickname_check` | `/nickname/check` | `pending` | — |
| `personal_fm` | `/personal_fm` | `pending` | — |
| `personal_fm_mode` | `/personal/fm/mode` | `pending` | — |
| `personalized` | `/personalized` | `pending` | — |
| `personalized_djprogram` | `/personalized/djprogram` | `pending` | — |
| `personalized_mv` | `/personalized/mv` | `pending` | — |
| `personalized_newsong` | `/personalized/newsong` | `pending` | — |
| `personalized_privatecontent` | `/personalized/privatecontent` | `pending` | — |
| `personalized_privatecontent_list` | `/personalized/privatecontent/list` | `pending` | — |
| `pl_count` | `/pl/count` | `pending` | — |
| `playlist_category_list` | `/playlist/category/list` | `pending` | — |
| `playlist_catlist` | `/playlist/catlist` | `pending` | — |
| `playlist_cover_update` | `/playlist/cover/update` | `pending` | — |
| `playlist_create` | `/playlist/create` | `pending` | — |
| `playlist_delete` | `/playlist/delete` | `pending` | — |
| `playlist_desc_update` | `/playlist/desc/update` | `pending` | — |
| `playlist_detail` | `/playlist/detail` | `verified` | `GET /v1/playlists/{ref}` |
| `playlist_detail_dynamic` | `/playlist/detail/dynamic` | `pending` | — |
| `playlist_detail_rcmd_get` | `/playlist/detail/rcmd/get` | `pending` | — |
| `playlist_highquality_tags` | `/playlist/highquality/tags` | `pending` | — |
| `playlist_hot` | `/playlist/hot` | `pending` | — |
| `playlist_import_name_task_create` | `/playlist/import/name/task/create` | `pending` | — |
| `playlist_import_task_status` | `/playlist/import/task/status` | `pending` | — |
| `playlist_mylike` | `/playlist/mylike` | `pending` | — |
| `playlist_name_update` | `/playlist/name/update` | `pending` | — |
| `playlist_order_update` | `/playlist/order/update` | `pending` | — |
| `playlist_privacy` | `/playlist/privacy` | `pending` | — |
| `playlist_subscribe` | `/playlist/subscribe` | `pending` | — |
| `playlist_subscribers` | `/playlist/subscribers` | `pending` | — |
| `playlist_tags_update` | `/playlist/tags/update` | `pending` | — |
| `playlist_track_add` | `/playlist/track/add` | `pending` | — |
| `playlist_track_all` | `/playlist/track/all` | `verified` | `GET /v1/playlists/{ref}/tracks` |
| `playlist_track_delete` | `/playlist/track/delete` | `pending` | — |
| `playlist_tracks` | `/playlist/tracks` | `pending` | — |
| `playlist_update` | `/playlist/update` | `pending` | — |
| `playlist_update_playcount` | `/playlist/update/playcount` | `pending` | — |
| `playlist_video_recent` | `/playlist/video/recent` | `pending` | — |
| `playmode_intelligence_list` | `/playmode/intelligence/list` | `pending` | — |
| `playmode_song_vector` | `/playmode/song/vector` | `pending` | — |
| `program_recommend` | `/program/recommend` | `pending` | — |
| `radio_sport_get` | `/radio/sport/get` | `pending` | — |
| `rebind` | `/rebind` | `pending` | — |
| `recent_listen_list` | `/recent/listen/list` | `pending` | — |
| `recommend_resource` | `/recommend/resource` | `implemented` | `GET /v1/recommendations/playlists`（2026-07-16 匿名 HTTP 实测为 401/上游 301；待真实账户验证内容路径） |
| `recommend_songs` | `/recommend/songs` | `verified` | `GET /v1/recommendations/tracks`（含 `afresh`→`refresh`；2026-07-16 匿名 HTTP 实测返回 30 首并保留推荐理由） |
| `recommend_songs_dislike` | `/recommend/songs/dislike` | `pending` | — |
| `record_recent_album` | `/record/recent/album` | `pending` | — |
| `record_recent_dj` | `/record/recent/dj` | `pending` | — |
| `record_recent_playlist` | `/record/recent/playlist` | `pending` | — |
| `record_recent_song` | `/record/recent/song` | `pending` | — |
| `record_recent_video` | `/record/recent/video` | `pending` | — |
| `record_recent_voice` | `/record/recent/voice` | `pending` | — |
| `register_anonimous` | `/register/anonimous` | `pending` | — |
| `register_cellphone` | `/register/cellphone` | `pending` | — |
| `register_xeapikey` | `/register/xeapikey` | `pending` | — |
| `related_allvideo` | `/related/allvideo` | `pending` | — |
| `related_playlist` | `/related/playlist` | `pending` | — |
| `relay_play_state_submit` | `/relay/play/state/submit` | `pending` | — |
| `resource_like` | `/resource/like` | `pending` | — |
| `sati_resource_list` | `/sati/resource/list` | `pending` | — |
| `sati_resource_list_more` | `/sati/resource/list/more` | `pending` | — |
| `sati_resource_sub` | `/sati/resource/sub` | `pending` | — |
| `sati_resource_sub_list` | `/sati/resource/sub/list` | `pending` | — |
| `sati_tag_list` | `/sati/tag/list` | `pending` | — |
| `sati_timescene_resources_get` | `/sati/timescene/resources/get` | `pending` | — |
| `scrobble` | `/scrobble` | `pending` | — |
| `scrobble_v1` | `/scrobble/v1` | `pending` | — |
| `search` | `/search` | `partial` | `GET /v1/search`（当前仅单曲类型） |
| `search_default` | `/search/default` | `pending` | — |
| `search_hot` | `/search/hot` | `pending` | — |
| `search_hot_detail` | `/search/hot/detail` | `pending` | — |
| `search_match` | `/search/match` | `pending` | — |
| `search_multimatch` | `/search/multimatch` | `pending` | — |
| `search_suggest` | `/search/suggest` | `pending` | — |
| `search_suggest_pc` | `/search/suggest/pc` | `pending` | — |
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
| `song_cloud_download` | `/song/cloud/download` | `pending` | — |
| `song_copyright_rcmd` | `/song/copyright/rcmd` | `pending` | — |
| `song_creators` | `/song/creators` | `pending` | — |
| `song_detail` | `/song/detail` | `verified` | `GET /v1/tracks/{ref}` |
| `song_downlist` | `/song/downlist` | `pending` | — |
| `song_download_url` | `/song/download/url` | `pending` | — |
| `song_download_url_v1` | `/song/download/url/v1` | `pending` | — |
| `song_dynamic_cover` | `/song/dynamic/cover` | `pending` | — |
| `song_like` | `/song/like` | `pending` | — |
| `song_like_check` | `/song/like/check` | `pending` | — |
| `song_lyrics_mark` | `/song/lyrics/mark` | `pending` | — |
| `song_lyrics_mark_add` | `/song/lyrics/mark/add` | `pending` | — |
| `song_lyrics_mark_del` | `/song/lyrics/mark/del` | `pending` | — |
| `song_lyrics_mark_user_page` | `/song/lyrics/mark/user/page` | `pending` | — |
| `song_monthdownlist` | `/song/monthdownlist` | `pending` | — |
| `song_music_detail` | `/song/music/detail` | `pending` | — |
| `song_order_update` | `/song/order/update` | `pending` | — |
| `song_purchased` | `/song/purchased` | `pending` | — |
| `song_red_count` | `/song/red/count` | `pending` | — |
| `song_singledownlist` | `/song/singledownlist` | `pending` | — |
| `song_url` | `/song/url` | `verified` | `GET /v1/tracks/{ref}/stream`（旧码率接口） |
| `song_url_match` | `/song/url/match` | `pending` | — |
| `song_url_ncmget` | `/song/url/ncmget` | `pending` | — |
| `song_url_v1` | `/song/url/v1` | `pending` | — |
| `song_url_v1_302` | `/song/url/v1/302` | `pending` | — |
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
| `toplist` | `/toplist` | `pending` | — |
| `toplist_artist` | `/toplist/artist` | `pending` | — |
| `toplist_detail` | `/toplist/detail` | `pending` | — |
| `toplist_detail_v2` | `/toplist/detail/v2` | `pending` | — |
| `ugc_album_get` | `/ugc/album/get` | `pending` | — |
| `ugc_artist_get` | `/ugc/artist/get` | `pending` | — |
| `ugc_artist_search` | `/ugc/artist/search` | `pending` | — |
| `ugc_detail` | `/ugc/detail` | `pending` | — |
| `ugc_mv_get` | `/ugc/mv/get` | `pending` | — |
| `ugc_song_get` | `/ugc/song/get` | `pending` | — |
| `ugc_user_devote` | `/ugc/user/devote` | `pending` | — |
| `user_account` | `/user/account` | `partial` | `GET /v1/account`（统一资料映射，待真实账户验证） |
| `user_audio` | `/user/audio` | `pending` | — |
| `user_binding` | `/user/binding` | `pending` | — |
| `user_bindingcellphone` | `/user/bindingcellphone` | `pending` | — |
| `user_cloud` | `/user/cloud` | `pending` | — |
| `user_cloud_del` | `/user/cloud/del` | `pending` | — |
| `user_cloud_detail` | `/user/cloud/detail` | `pending` | — |
| `user_comment_history` | `/user/comment/history` | `pending` | — |
| `user_detail` | `/user/detail` | `pending` | — |
| `user_detail_new` | `/user/detail/new` | `pending` | — |
| `user_dj` | `/user/dj` | `pending` | — |
| `user_event` | `/user/event` | `pending` | — |
| `user_follow_mixed` | `/user/follow/mixed` | `pending` | — |
| `user_followeds` | `/user/followeds` | `pending` | — |
| `user_follows` | `/user/follows` | `pending` | — |
| `user_level` | `/user/level` | `pending` | — |
| `user_medal` | `/user/medal` | `pending` | — |
| `user_mutualfollow_get` | `/user/mutualfollow/get` | `pending` | — |
| `user_playlist` | `/user/playlist` | `implemented` | `GET /v1/account/playlists`（待真实账户验证） |
| `user_playlist_collect` | `/user/playlist/collect` | `pending` | — |
| `user_playlist_create` | `/user/playlist/create` | `pending` | — |
| `user_record` | `/user/record` | `implemented` | `GET /v1/account/history`、`GET /v1/users/{ref}/history`（`all_time/week`；已验证匿名权限错误映射，待真实账户验证） |
| `user_replacephone` | `/user/replacephone` | `pending` | — |
| `user_social_status` | `/user/social/status` | `pending` | — |
| `user_social_status_edit` | `/user/social/status/edit` | `pending` | — |
| `user_social_status_rcmd` | `/user/social/status/rcmd` | `pending` | — |
| `user_social_status_support` | `/user/social/status/support` | `pending` | — |
| `user_subcount` | `/user/subcount` | `pending` | — |
| `user_update` | `/user/update` | `pending` | — |
| `verify_getQr` | `/verify/getQr` | `pending` | — |
| `verify_qrcodestatus` | `/verify/qrcodestatus` | `pending` | — |
| `video_category_list` | `/video/category/list` | `pending` | — |
| `video_detail` | `/video/detail` | `pending` | — |
| `video_detail_info` | `/video/detail/info` | `pending` | — |
| `video_group` | `/video/group` | `pending` | — |
| `video_group_list` | `/video/group/list` | `pending` | — |
| `video_sub` | `/video/sub` | `pending` | — |
| `video_timeline_all` | `/video/timeline/all` | `pending` | — |
| `video_timeline_recommend` | `/video/timeline/recommend` | `pending` | — |
| `video_url` | `/video/url` | `pending` | — |
| `vip_growthpoint` | `/vip/growthpoint` | `pending` | — |
| `vip_growthpoint_details` | `/vip/growthpoint/details` | `pending` | — |
| `vip_growthpoint_get` | `/vip/growthpoint/get` | `pending` | — |
| `vip_growthpoint_getall` | `/vip/growthpoint/getall` | `pending` | — |
| `vip_info` | `/vip/info` | `pending` | — |
| `vip_info_v2` | `/vip/info/v2` | `pending` | — |
| `vip_sign` | `/vip/sign` | `pending` | — |
| `vip_sign_detail` | `/vip/sign/detail` | `pending` | — |
| `vip_sign_history` | `/vip/sign/history` | `pending` | — |
| `vip_sign_info` | `/vip/sign/info` | `pending` | — |
| `vip_tasks` | `/vip/tasks` | `pending` | — |
| `vip_tasks_v1` | `/vip/tasks/v1` | `pending` | — |
| `vip_timemachine` | `/vip/timemachine` | `pending` | — |
| `voice_delete` | `/voice/delete` | `pending` | — |
| `voice_detail` | `/voice/detail` | `pending` | — |
| `voice_lyric` | `/voice/lyric` | `pending` | — |
| `voice_upload` | `/voice/upload` | `pending` | — |
| `voicelist_detail` | `/voicelist/detail` | `pending` | — |
| `voicelist_list` | `/voicelist/list` | `pending` | — |
| `voicelist_list_search` | `/voicelist/list/search` | `pending` | — |
| `voicelist_my_created` | `/voicelist/my/created` | `pending` | — |
| `voicelist_search` | `/voicelist/search` | `pending` | — |
| `voicelist_trans` | `/voicelist/trans` | `pending` | — |
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
