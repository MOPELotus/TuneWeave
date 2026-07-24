# QQ 音乐 API 全量覆盖账本

上游快照：`L-1124/QQMusicApi@261326eec051e7f444296b5c461e7412c4b25bb9`

本表逐项登记该快照 14 个公开 API 类的 104 个公开方法。QQMusicApi 是异步 Python SDK，不是 HTTP 服务；方法名用于固定验收分母，TuneWeave 将独立实现观察到的 QQ 音乐协议，不复制、翻译、链接或打包上游源码。内部辅助函数、会话封装、分页器和模型不重复计入业务方法分母，但 Basic 所需的平台协议单独列入 [`qq-basic.md`](qq-basic.md) 验收。

状态含义：

- `pending`：尚未完成统一映射或 QQ 扩展端点。
- `partial`：只完成部分参数、响应、协议分支或统一链路。
- `implemented`：代码与离线测试已完成，仍缺真实网络或账户前置验证。
- `verified`：统一端点、测试以及相应真实网络路径均已验证。

当前统计：`pending=83`、`partial=2`、`implemented=10`、`verified=9`。其中 QQ Basic 为 77 项，QQ 全量后续项为 27 项。2026-07-25 上游新增彩铃搜索/文件规格、搜索 selectors、助唱标注及 4 个歌词方法，并扩展批量歌曲查询；缺失的新分支已如实退回 `partial` 或登记为 `pending`，其中彩铃/selectors、逐项歌曲查询和助唱标注已完成修正与真实验证。实施顺序按普通音乐 App 的使用频率、播放依赖和底层必要性排列，不按类名或方法名字母排序。

| 编号 | 类别 | 上游公开方法 | Basic | 状态 | TuneWeave 映射/缺口 |
| --- | --- | --- | ---: | --- | --- |
| Q001 | 搜索与发现 | `SearchApi.get_hotkey` | 是 | `verified` | `GET /v1/search/trending?platform=qq&detail=...` 精确调用 Android `music.musicsearch.HotkeyService/GetHotkeyForQQMusicMobile` 并提交参考算法生成的 `search_id`。`vec_hotkey` 原始顺序映射为从 1 开始的稳定排名，实际搜索 `query` 不被活动展示 `title` 覆盖；`detail=full` 提供说明、字符串分值转无符号整数、趋势/序列类型、图标与跳转，`brief` 只收敛关键字和排名，但两种模式都在条目扩展保留标题、封面、热词/直达/歌曲 ID、置顶态、排序、趋势、来源及完整原项。`ret_code` 非零、缺失或目录缺失均拒绝为假成功；实验 ID、榜单时段、列表 ID 与完整响应保留在列表扩展。2026-07-22 provider 与 release 统一 HTTP 真实返回 30 项，首项排名 1“周杰伦”，full 分值存在、brief 富字段为空，上游码 0 |
| Q002 | 搜索与发现 | `SearchApi.complete` | 是 | `verified` | `GET /v1/search/suggestions?platform=qq&client=mobile&q=...` 精确调用 Android `music.smartboxCgi.SmartBoxCgi/GetSmartBoxResult`，参考固定的 `search_id/query/num_per_page=0/page_idx=0` 均保留。`items` 普通补全、`vec_related_items` 相关词和按 `insert_pos` 插入的 `vec_direct_items` 直达结果不会合并丢失；歌手直达结果提升为统一 `Artist`，其他已知类型保留 `kind`，无法安全提升的直达结构以含完整原文的 `opaque` 资源表达。搜索会话、展示高亮、图标、跳转、分值、关联 ID 和完整响应均保留，非数组桶拒绝为假空结果。2026-07-22 同一持久匿名设备的 provider 与 release 统一 HTTP 真实搜索“周杰伦”，返回 21 项，首项为 `artist/qq:0025NhlN2yWrP4`，上游码 0 |
| Q003 | 搜索与发现 | `SearchApi.quick_search` | 是 | `verified` | `GET /v1/search/suggestions?platform=qq&client=web&q=...` 精确调用固定 HTTPS `c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg`，查询参数经 URL 编码且不会开放任意域名、请求头或凭据注入。响应按各分区 `order` 动态排序，单曲、歌手、专辑、MV 分别提升为统一 `Track/Artist/Album/Video`，不会因 JSON 对象字段顺序变化而乱序；未来新增的未知分区仍逐项以携带完整原文的 `Opaque` 资源返回，不会静默丢弃。分区名称、顺序、类型、计数、原项和完整响应均保留；非零或缺失 `code/subcode`、缺失数据、已知分区缺失或畸形 `itemlist` 均拒绝为假成功。2026-07-22 provider 与 release 统一 HTTP 真实搜索“周杰伦”均通过，返回 10 项，依次覆盖 4 首单曲、2 位歌手、2 张专辑、2 个 MV，首项为 `track/qq:0039MnYb0qxYhV`“晴天”，上游 `code/subcode=0` |
| Q004 | 搜索与发现 | `SearchApi.search_by_type` | 是 | `verified` | `GET /v1/search?platform=qq&kind=...` 接入 Android `DoSearchForQQMusicMobile` 的歌曲、歌手、专辑、歌单、MV、歌词、用户、彩铃、节目专辑和节目 10 类，并保留 `searchid/highlight`、按类别安全页宽、逻辑槽位分页、稀疏歌单缺口、稳定身份和完整原项。统一 `ringtone|ring` 不复用跨平台数字 `10=album`，彩铃结果提升为带 `search_category=ringtone` 的可播放 `Track`。`selectors` 以 URL 编码的强类型 `[{id,name,type}]` 接受；同类型重复项在联网前拒绝，合法项同时生成字符串映射 `selectors` 与保序 `vec_selectors`，二维响应目录经强结构校验进入分页扩展，未知字段保存在 selector 扩展。2026-07-25 上游 Python、Rust provider 和统一 HTTP 均真实验证：彩铃“周杰伦”总数 553、返回 2 条统一曲目；`id=4558/name=默认/type=0` selector 返回 2 条且选择语义保留。随后 Rust provider 逐类真实回归全部 10 类均通过，并据当前用户响应补齐 `title/subtitle/iconurl` 字段优先级；早前把最后三类合并进单批探测得到的 `code=2001` 不再作为单类可用性结论 |
| Q005 | 搜索与发现 | `SearchApi.general_search` | 是 | `pending` | 综合搜索及多字段续页游标 |
| Q006 | 搜索与发现 | `RecommendApi.get_home_feed` | 是 | `pending` | 首页推荐卡片和防重复游标 |
| Q007 | 搜索与发现 | `RecommendApi.get_recommend_songlist` | 是 | `pending` | 推荐歌单 |
| Q008 | 搜索与发现 | `RecommendApi.get_recommend_newsong` | 是 | `pending` | 分地区/语种新歌 |
| Q009 | 搜索与发现 | `RecommendApi.get_guess_recommend` | 是 | `pending` | 猜你喜欢 |
| Q010 | 搜索与发现 | `RecommendApi.get_radar_recommend` | 是 | `pending` | 雷达推荐 |
| Q011 | 搜索与发现 | `TopApi.get_category` | 是 | `pending` | 榜单目录 |
| Q012 | 搜索与发现 | `TopApi.get_detail` | 是 | `pending` | 榜单歌曲及分页 |
| Q013 | 内容展示 | `SongApi.query_song` | 是 | `verified` | `GET/POST /v1/tracks` 保留批量、顺序、重复项、账户和数字 ID/MID 身份；GET 的 `song_type/type` 可为整批提供同一类型，POST 新增强类型 `items/query_info`，逐项接受且仅接受 `ref`、数字 `id` 或 `mid` 之一，并分别保存 `identifier_kind/song_type`。2026-07-25 差分验证确认两个数字 ID 使用 `types=[1,113]` 能真实返回普通与特殊歌曲；参考实现宣称的单子请求混合 `ids+mids` 无论顺序都返回 `103901`。TuneWeave 不机械复制该缺陷，而是在同一个 QQ HTTP 批包内生成独立 `ids` 和 `mids` 两个合法 CGI 子请求，各自重排 `types/modify_stamp`，随后按原输入位置恢复跨组顺序和重复项；返回扩展同时保留实际 `song_type` 与 `requested_song_type`。单元、统一 HTTP 和真实 Rust provider 均验证 MID→特殊数字 ID→普通数字 ID→重复 MID 的混合顺序，真实结果依次为 `qq:003w2xz20QlUZt/qq:003Hx1mg4SlZVM/qq:0017ahqa0NvuNU/重复首项`，类型 `1/113/1` 正确 |
| Q014 | 内容展示 | `SongApi.get_detail` | 是 | `verified` | `GET /v1/tracks/{qq-ref}` 精确调用固定 Web `music.pf_song_detail_svr/get_song_detail_yqq`，QQ 数字 ID 使用 `song_id`，MID 使用 `song_mid`，两种输入均不改写为另一分支。新增 Web JSON CGI 档案逐字段匹配参考：独立 Chrome 120 UA，`ct=24/cv=4747474/platform=yqq.json/chid=0/uin=0/g_tk=5381/g_tk_new_20200303=5381` 及字符集、通知、新码字段，不借用 Android UA 或设备身份。`track_info` 复用完整统一 `Track` 映射；`info.company/genre/intro/lan/pub_time.content`、`extras` 和含业务码的完整子响应分别保存在详情扩展，发行公司、流派、简介、语言、发布时间及未来平台字段不会丢失。缺失曲目明确返回资源不存在；请求/返回身份不一致，或已出现但类型、内容项结构畸形的富字段均拒绝为假成功。2026-07-22 provider 与 release 统一 HTTP 已真实验证数字 ID `100` 和 MID `003w2xz20QlUZt` 两条分支：数字 ID 返回 `qq:003a7WZv0CYKYn`，五类富内容各有 1 项，扩展含 `from/name/subtitle/transname/wikiurl`，MID 返回原请求引用，两者上游码均为 0 |
| Q015 | 内容展示 | `SongApi.get_similar_song` | 是 | `pending` | 相似歌曲 |
| Q016 | 内容展示 | `SongApi.get_labels` | 是 | `pending` | 歌曲标签 |
| Q017 | 内容展示 | `SongApi.get_related_songlist` | 是 | `pending` | 相关歌单 |
| Q018 | 内容展示 | `SongApi.get_related_mv` | 是 | `pending` | 相关 MV |
| Q019 | 内容展示 | `SongApi.get_other_version` | 是 | `pending` | 同曲其他版本 |
| Q020 | 内容展示 | `SongApi.get_producer` | 是 | `pending` | 制作人信息；排在高频链路之后 |
| Q021 | 内容展示 | `SongApi.get_sheet` | 是 | `pending` | 曲谱详情；排在高频链路之后 |
| Q022 | 内容展示 | `SongApi.has_sheet` | 是 | `pending` | 曲谱存在性；排在高频链路之后 |
| Q023 | 内容展示 | `SongApi.get_fav_num` | 是 | `pending` | 歌曲收藏人数 |
| Q024 | 内容展示 | `LyricApi.get_lyric` | 是 | `verified` | `GET /v1/tracks/{qq-ref}/lyrics` 完整覆盖数字 ID/MID、`song_type`、LRC/QRC、翻译、罗马音、助唱标注、严格解密和“逐字不被逐行覆盖”。Core 将助唱内容与时间戳建为独立强类型字段；QQ 请求对 `qrc/trans/roma` 保持整数，而 `needSingingAnnotations` 通过逐请求策略保留真实 JSON 布尔，避免全局改变其他 CGI。畸形助唱时间戳或密文拒绝为假成功。2026-07-25 上游 Python 差分确认数字 ID `97773` 存在助唱标注；Rust provider 与统一 HTTP 均真实返回并解密 8784 字符 QRC XML、时间戳 `1768529601`，同时原有 LRC/QRC/翻译/罗马音及 MID 分支回归通过 |
| Q101 | 内容展示 | `LyricApi.get_singing_annotations_info` | 是 | `pending` | 助唱标注歌词存在性；精确请求 `GetSingingAnnotationsInfo` 的 `songID/needNum=false` 布尔分支，并以强类型布尔结果表达 |
| Q102 | 内容展示 | `LyricApi.get_multi_style_trans_lyric` | 是 | `pending` | 多风格翻译歌词；完整保留 `style/styleName/lyric/timestamp`，每项独立解密，不能压入单一 `translated` 字符串 |
| Q103 | 内容展示 | `LyricApi.is_ai_dict_exists` | 是 | `pending` | AI 歌词词典存在性；与词典详情分离，不因空列表猜测存在性 |
| Q104 | 内容展示 | `LyricApi.get_ai_dict` | 是 | `pending` | AI 歌词词典详情；强类型建模短语、解释、原歌词、翻译和歌词时间戳，完整保留列表顺序 |
| Q025 | 内容展示 | `AlbumApi.get_detail` | 是 | `pending` | 专辑详情 |
| Q026 | 内容展示 | `AlbumApi.get_song` | 是 | `pending` | 专辑歌曲分页 |
| Q027 | 内容展示 | `AlbumApi.get_new_album` | 是 | `pending` | 新专辑目录 |
| Q028 | 内容展示 | `SingerApi.get_singer_list` | 是 | `pending` | 歌手分类目录 |
| Q029 | 内容展示 | `SingerApi.get_singer_list_index` | 是 | `pending` | 歌手索引分页 |
| Q030 | 内容展示 | `SingerApi.get_info` | 是 | `pending` | 歌手基本资料 |
| Q031 | 内容展示 | `SingerApi.get_tab_detail` | 是 | `pending` | 歌手主页标签内容 |
| Q032 | 内容展示 | `SingerApi.get_desc` | 是 | `pending` | 歌手简介 |
| Q033 | 内容展示 | `SingerApi.get_similar` | 是 | `pending` | 相似歌手 |
| Q034 | 内容展示 | `SingerApi.get_songs_list` | 是 | `pending` | 歌手歌曲分页 |
| Q035 | 内容展示 | `SingerApi.get_album_list` | 是 | `pending` | 歌手专辑分页 |
| Q036 | 内容展示 | `SingerApi.get_mv_list` | 是 | `pending` | 歌手 MV 分页 |
| Q037 | 内容展示 | `SonglistApi.get_detail` | 是 | `implemented` | `GET /v1/playlists/{qq-ref}` 与 `/tracks` 精确调用 Android `music.srfDissInfo.DissInfo/CgiGetDiss`。公开 `qq:<playlist-id>` 映射 `disstid`；`qq:dir:<dirid>` 映射 `disstid=0/dirid` 并以所选账户 `encryptUin` 提交 `enc_host_uin`。详情分支固定 `tag/userinfo=true`、`onlysonglist=false`，歌曲分页固定 `tag/userinfo=false`、`onlysonglist=true`，两者都保留 `orderlist=true` 和精确 `song_begin/song_num`；强类型解析 `dirinfo/creator/songlist_size/songlist/total_song_num/hasmore`，业务码、ID 冲突和分页矛盾均拒绝为假成功，歌曲复用完整 QQ Track 映射。2026-07-25 provider 与 release 统一 HTTP 真实验证公开歌单 `7039749142`：详情非空、首个 2 曲分页总数 99、首曲 `0039MnYb0qxYhV`；账户特殊目录代码和参数已离线验收，真实账户待联合验收，故保持 `implemented` |
| Q038 | 内容展示 | `MvApi.get_detail` | 是 | `pending` | 批量 MV 详情 |
| Q039 | 内容展示 | `MvApi.get_mv_list` | 是 | `pending` | 地区、版本、排序 MV 目录 |
| Q040 | 播放与权益 | `SongApi.get_cdn_dispatch` | 是 | `verified` | `GET /v1/media/cdn?platform=qq` 精确调用 Android `music.audioCdnDispatch.cdnDispatch/GetCdnDispatch`，每次生成独立 32 位小写十六进制 GUID，并完整提交参考参数 `uid="0"/use_new_domain=1/use_ipv6=1`。统一 `AudioCdnDispatch` 保留 CDN 根地址的上游顺序与重复项、QUIC 节点参数、相对探活文件及过期/刷新/缓存秒数；只接受无凭据的 HTTP(S) 根地址，畸形目录、绝对探活 URL、非零 `retcode`、空根目录和非正计时不会伪装为成功。节点原项、完整响应及本次 GUID 保存在扩展。2026-07-22 provider 与 release 统一 HTTP 真实返回 10 个根地址、9 个节点和 1 个重复根，HTTP/HTTPS 均存在，`expiration/cacheTime=86400`、`refreshTime=1800`、顶层及业务码均为 0 |
| Q041 | 播放与权益 | `SongApi.get_song_urls` | 是 | `implemented` | `GET /v1/tracks/{qq-ref}/files` 与 `POST /v1/media/files` 完整保留 1–100 项批量、顶层默认规格、逐项规格/MID/`song_type`/`media_mid`、顺序和重复项；参考实现未执行其声明的 100 项上限，TuneWeave 修正为明确边界。2026-07-25 同步上游后规格扩展为普通 17、加密 13、特殊 15、彩铃 3，共 48 种；整数 `0..47` 稳定映射，`44=trial_ogg_640`、`45..47=ring_128/ring_96/ring_48`。普通/加密模块选择、文件名双 MID/单媒体 MID、独立 GUID、匿名或 `(qq, account)` 凭据注入、MID/文件名/数量严格对齐、相对 PURL、VKey/EKey、过期秒数、权限业务码和单次匿名会话刷新均完整保留。统一 `AudioStream/AudioDownload` 只选择无需额外解密的可播放规格：`auto` 从常用 320k 向下回退，明确高阶音质不被自动误选，六档精确码率不猜测，试听窗口、实际音质、文件大小、最短有效期、HTTPS 首选 CDN 和保序备用地址均返回；下载不把试听冒充完整文件，`/download/redirect` 仅在真实 URL 存在时 302。QQ 已成为原始播放平台及跨平台 resolver 目标，2026-07-22 release HTTP 真实验证统一试听流、无损下载、302，并以网易云“青花瓷”严格匹配到 QQ 成功播放。已知文件/版本/试听元数据在解析前进入内部强类型结构，冲突或畸形字段拒绝。2026-07-22 全部旧 45 种规格真实覆盖；2026-07-25 新增 3 种彩铃离线差分通过，但全新匿名设备真实请求当前被 QQ 以 `code=1000` 拒绝。仍缺登录/VIP 账户和新增彩铃成功态真实验收，故保持 `implemented` |
| Q042 | 播放与权益 | `MvApi.get_mv_urls` | 是 | `pending` | MV 多清晰度播放地址 |
| Q043 | 登录与账户 | `LoginApi.check_expired` | 是 | `implemented` | `GET /v1/auth/session?platform=qq&account=...` 精确加载 `(qq, account)` 的 `qq_credential_v1`，以凭据同时注入 Android `comm` 和 Cookie 后调用 `music.UserInfo.userInfoServer/GetLoginUserInfo`；业务码 `0` 映射已认证，`1000/104400/104401` 映射凭据失效而不是 HTTP 失败，其余登录/限流码保持统一错误类。不存在的账户别名直接返回 `authenticated=false`，不会回退默认账户或访问网络。账户资料保留 music ID、登录类型、平台码，以及仅在两个时间字段都存在时计算的本地到期时间/状态；本地时钟只作扩展信息，不覆盖服务端有效性。缺失账户、时间语义、错误映射和请求形状已离线验收，真实已登录状态待账户验收后升为 `verified` |
| Q044 | 登录与账户 | `LoginApi.refresh_credential` | 是 | `implemented` | `POST /v1/auth/session` 精确加载指定 `(qq, account)` 后调用 Android `music.login.LoginServer/Login`，固定 `loginMode=2`，并按 `loginType=1`、`2`、其他值保留微信、QQ、移动端/验证码三套不同参数分支；原凭据同时注入 `comm` 和 Cookie，`comm.tmeLoginType` 不被推断成其他账户类型。只有业务码为 0、新凭据通过强类型规范化且账户仓库写入成功时，才以新一代文件原子替换同一账户；网络、业务码、响应畸形和持久化失败均不预删旧凭据。三分支参数、缺失别名和错误前置已离线验收，真实账户刷新待登录验收后升为 `verified` |
| Q045 | 登录与账户 | `LoginApi.logout` | 是 | `implemented` | `DELETE /v1/auth/session?platform=qq&account=...` 以指定账户凭据调用 Android `music.login.LoginServer/Logout`，凭据同时进入 `comm` 与 Cookie。业务码 0 或明确失效码 `1000/104400/104401` 后才删除本地精确 `(qq, account)`；缺失别名幂等返回 `removed=false`，限流、未知业务错误和网络失败保留旧凭据，同平台其他账户不受影响。若上游已成功而本地删除失败，错误明确携带脱敏上游状态。成功/失效码映射、幂等缺失别名、精确多账户删除和 `SessionManagement` 能力已离线验收，真实退出待登录账户联合验收后升为 `verified` |
| Q046 | 登录与账户 | `LoginApi.get_qrcode` | 是 | `verified` | `POST /v1/auth/qr` 完整接入 `login_type=qq/default`、`wx/wechat/weixin` 与 `mobile/app`：固定 QQ 互联/微信开放平台端点分别取得 `qrsig` 或 `uuid`，Android `music.login.LoginServer/CreateQRCode` 使用参考 `param.ct=11/cv=14090008` 和 `comm.ct=23/cv=0` 取得移动端二维码 ID；三类图片均在本地校验并返回 Base64 PNG/JPEG。上游 Cookie 和标识符只存于 10 分钟进程内私有事务，外部仅返回随机 `tw-auth-*`。2026-07-25 provider 与统一 HTTP 三类真实图片及未扫码等待态全部通过 |
| Q047 | 登录与账户 | `LoginApi.check_qrcode` | 是 | `implemented` | QQ `ptqrlogin` 的 `66/67/65/68/0` 与微信长轮询 `408/404/402/403/405` 已映射为统一等待、已扫码、过期、失败和确认；QQ 成功态继续完成 `check_sig`、OAuth code、`QQConnectLogin.LoginServer/QQLogin`，微信成功态调用 `music.login.LoginServer/Login`，凭据只在确认后按 `(qq, account)` 以 `qq_credential_v1` 原子持久化并返回脱敏账户资料。Cookie jar、并发重复轮询和终态缓存均隔离在短期事务内，网络错误不回显 qrsig、ptsigx、OAuth code 或 Cookie。2026-07-25 两种统一 HTTP 未扫码等待态真实通过，回调解析、终态隔离和凭据持久化已有离线单元覆盖，待真实扫码验收后升为 `verified` |
| Q048 | 登录与账户 | `LoginApi.checking_mobile_qrcode` | 是 | `implemented` | 移动端二维码创建后、响应图片前即通过固定 `wss://mu.y.qq.com:443/ws/handshake` 建立 MQTT 5 会话，WebSocket 握手复用 QQ 服务端代理/TLS 配置并校验 Upgrade、子协议和 accept key；CONNECT 保留 `AuthenticationMethod=pass` 与五项业务 User Property，完整处理 CONNACK 成功、`0x9C/0x9D` 节点重定向、上限和 server reference 校验。订阅 `management.qrcode_login/{qrcodeID}` 时提交 `authorization=tmelogin/pubsub=unicast`，持久后台监听并发送 PINGREQ，避免两次 HTTP 轮询间丢失事件；`scanned/canceled/timeout/loginFailed/cookies` 分别映射统一状态，cookies 仅提取强类型 music ID/token 后调用 `music.login.LoginServer/Login`、`tmeLoginType=6` 并沿用 Q047 原子账户持久化。10 分钟硬截止会主动终止任务，连接/解析错误不回显二维码 ID 或 token。2026-07-25 真实 `CreateQRCode → WebSocket → MQTT CONNECT/SUBACK → waiting` 在 provider 与统一 HTTP 均通过；扫码确认态待真实账户验收后升为 `verified` |
| Q049 | 登录与账户 | `LoginApi.send_authcode` | 是 | `implemented` | `POST /v1/auth/challenges` 以 Android `music.login.LoginServer/SendPhoneAuthCode` 单次发送，固定 `tmeAppid=qqmusic`、`areaCode` 和 `comm.tmeLoginMethod=3`；普通号码保持字符串 `phoneNo`，显式 `encrypted:` 前缀才映射 `encryptedPhoneNo`，避免参考实现把所有字符串误判为密文。成功才创建 10 分钟外层挑战事务，`20276` 作为安全验证错误返回平台 `security_url`，`100001/104604/2001` 稳定映射限流，其余业务码保留；不自动重试、不记录或回显手机号。参数、分支、错误类和统一 provider 已离线验收，真实发送按用户风控要求暂不触发 |
| Q050 | 登录与账户 | `LoginApi.phone_authorize` | 是 | `implemented` | 挑战验证复用服务端保存的同一 `principal/account`，调用 `music.login.LoginServer/Login`，提交 `code/loginMode=1`、普通或加密手机号分支以及 `comm.tmeLoginMethod=3/tmeLoginType=0`；空白、控制字符和超长验证码在网络前拒绝。成功凭据复用二维码链路的强类型解析、`qq_credential_v1` 原子写入和脱敏 `AccountProfile`，验证码错误、绑定异常、账户限制、设备上限及限流码分别映射统一错误，不会把一次性验证码或密钥写入日志/响应。完整参数与持久化分支已离线验收，真实成功态待用户主动提供验证码后升为 `verified` |
| Q051 | 个人音乐库 | `AlbumApi.fav_album` | 是 | `pending` | 收藏专辑 |
| Q052 | 个人音乐库 | `AlbumApi.del_fav_album` | 是 | `pending` | 取消收藏专辑 |
| Q053 | 个人音乐库 | `SonglistApi.create` | 是 | `pending` | 创建歌单 |
| Q054 | 个人音乐库 | `SonglistApi.delete` | 是 | `pending` | 删除歌单 |
| Q055 | 个人音乐库 | `SonglistApi.add_songs` | 是 | `pending` | 歌单添加歌曲，保留歌曲 ID 与类型元组 |
| Q056 | 个人音乐库 | `SonglistApi.del_songs` | 是 | `pending` | 歌单删除歌曲，保留歌曲 ID 与类型元组 |
| Q057 | 个人音乐库 | `SonglistApi.like_song` | 是 | `pending` | 喜欢歌曲 |
| Q058 | 个人音乐库 | `SonglistApi.unlike_song` | 是 | `pending` | 取消喜欢歌曲 |
| Q059 | 个人音乐库 | `UserApi.get_homepage` | 是 | `pending` | 用户/账户主页资料 |
| Q060 | 个人音乐库 | `UserApi.get_vip_info` | 是 | `pending` | VIP 等级、有效期和权益 |
| Q061 | 个人音乐库 | `UserApi.get_follow_singers` | 是 | `pending` | 关注歌手目录 |
| Q062 | 个人音乐库 | `UserApi.get_created_songlist` | 是 | `partial` | `GET /v1/account/playlists?platform=qq&account=...` 已用精确账户 music ID 调用 Android `music.musicasset.PlaylistBaseRead/GetPlaylistByUin`，强类型解析 `v_playlist/v_delTid/bFinish/total`，创建歌单固定排在统一账户目录前部；`id=0` 的特殊目录以 `qq:dir:<dirid>` 保持稳定身份，普通目录保留 playlist ID 与 dir ID。创建与收藏目录跨边界分页无重复/漏项，缺失账户或加密 UIN 在联网前失败。仍缺上游允许的任意用户 UIN 查询、Uni 集合导入和真实账户验收，故未升为 `implemented` |
| Q063 | 个人音乐库 | `UserApi.get_fav_song` | 是 | `implemented` | `GET /v1/account/favorites/tracks?platform=qq&account=...` 与 `GET /v1/users/qq:<encrypted-uin>/favorites/tracks` 均精确调用 Android `music.srfDissInfo.DissInfo/CgiGetDiss` 的 `disstid=0/dirid=201/enc_host_uin` 分支；前者从精确账户凭据取加密 UIN，后者保留上游任意用户 `euin` 能力并允许可选查看者账户。两端复用 `orderlist=true/onlysonglist=true/tag=false/userinfo=false`、完整 offset/limit、强类型分页和 Track 映射；输入与账户错误均在联网前拒绝，不使用占位凭据。代码与离线分支已验收，真实账户及已知公开加密 UIN 成功态待联合验收后升为 `verified` |
| Q064 | 个人音乐库 | `UserApi.get_fav_songlist` | 是 | `partial` | 同一账户歌单端点已用所选凭据的 `encryptUin` 调用 Android `music.musicasset.PlaylistFavRead/CgiGetPlaylistFavInfo`，精确保留 `offset/size`、`number/total/hasmore/hide`、删除/失败 ID 和完整响应，并与创建目录组成连续全局分页；收藏项标记 `subscribed=true`，不会使用参考项目的占位凭据。仍缺任意用户加密 UIN 查询、Uni 集合导入和真实账户验收，故保持 `partial` |
| Q065 | 个人音乐库 | `UserApi.fav_songlist` | 是 | `pending` | 收藏歌单 |
| Q066 | 个人音乐库 | `UserApi.unfav_songlist` | 是 | `pending` | 取消收藏歌单 |
| Q067 | 个人音乐库 | `UserApi.get_fav_album` | 是 | `pending` | 收藏专辑列表 |
| Q068 | 个人音乐库 | `UserApi.get_fav_mv` | 是 | `pending` | 收藏 MV 列表 |
| Q069 | 个人音乐库 | `UserApi.get_music_gene` | 是 | `pending` | 音乐基因/个性资料 |
| Q070 | 个人音乐库 | `UserApi.get_dislike_list` | 是 | `pending` | 不喜欢列表 |
| Q071 | 个人音乐库 | `UserApi.add_dislike` | 是 | `pending` | 添加不喜欢内容 |
| Q072 | 个人音乐库 | `UserApi.cancel_dislike` | 是 | `pending` | 取消单项不喜欢 |
| Q073 | 个人音乐库 | `UserApi.cancel_all_dislike_song` | 是 | `pending` | 清空歌曲不喜欢列表 |
| Q074 | 评论（全量） | `CommentApi.get_comment_count` | 否 | `pending` | QQ 全量阶段接入，不从最终范围删除 |
| Q075 | 评论（全量） | `CommentApi.get_hot_comments` | 否 | `pending` | QQ 全量阶段接入 |
| Q076 | 评论（全量） | `CommentApi.get_new_comments` | 否 | `pending` | QQ 全量阶段接入 |
| Q077 | 评论（全量） | `CommentApi.get_recommend_comments` | 否 | `pending` | QQ 全量阶段接入 |
| Q078 | 评论（全量） | `CommentApi.get_moment_comments` | 否 | `pending` | QQ 全量阶段接入 |
| Q079 | 评论（全量） | `CommentApi.add_comment` | 否 | `pending` | QQ 全量阶段接入 |
| Q080 | 评论（全量） | `CommentApi.delete_comment` | 否 | `pending` | QQ 全量阶段接入 |
| Q081 | 用户社交（全量） | `UserApi.get_fans` | 否 | `pending` | QQ 全量阶段接入 |
| Q082 | 用户社交（全量） | `UserApi.get_friend` | 否 | `pending` | QQ 全量阶段接入 |
| Q083 | 用户社交（全量） | `UserApi.get_follow_user` | 否 | `pending` | QQ 全量阶段接入 |
| Q084 | 私信（全量） | `PrivateMessageApi.get_sessions` | 否 | `pending` | QQ 全量阶段接入 |
| Q085 | 私信（全量） | `PrivateMessageApi.delete_session` | 否 | `pending` | QQ 全量阶段接入 |
| Q086 | 私信（全量） | `PrivateMessageApi.get_messages` | 否 | `pending` | QQ 全量阶段接入 |
| Q087 | 私信（全量） | `PrivateMessageApi.send_message` | 否 | `pending` | QQ 全量阶段接入全部消息类型和分支 |
| Q088 | 私信（全量） | `PrivateMessageApi.delete_message` | 否 | `pending` | QQ 全量阶段接入 |
| Q089 | 私信（全量） | `PrivateMessageApi.clear_session` | 否 | `pending` | QQ 全量阶段接入 |
| Q090 | 私信（全量） | `PrivateMessageApi.set_config` | 否 | `pending` | QQ 全量阶段接入 |
| Q091 | 私信（全量） | `PrivateMessageApi.get_config` | 否 | `pending` | QQ 全量阶段接入 |
| Q092 | 私信（全量） | `PrivateMessageApi.get_musician_message_card` | 否 | `pending` | QQ 全量阶段接入 |
| Q093 | 私信（全量） | `PrivateMessageApi.report_card_message_action` | 否 | `pending` | QQ 全量阶段接入 |
| Q094 | 私信（全量） | `PrivateMessageApi.get_chat_entries` | 否 | `pending` | QQ 全量阶段接入 |
| Q095 | 私信（全量） | `PrivateMessageApi.get_media_message_details` | 否 | `pending` | QQ 全量阶段接入 |
| Q096 | 私信（全量） | `PrivateMessageApi.mark_all_messages_read` | 否 | `pending` | QQ 全量阶段接入 |
| Q097 | 私信（全量） | `PrivateMessageApi.get_safety_hint` | 否 | `pending` | QQ 全量阶段接入 |
| Q098 | 私信（全量） | `PrivateMessageApi.get_friendship_badge` | 否 | `pending` | QQ 全量阶段接入 |
| Q099 | 私信上传（全量） | `HelperApi.init_upload` | 否 | `pending` | QQ 全量阶段随媒体私信接入 |
| Q100 | 私信上传（全量） | `HelperApi.finish_upload` | 否 | `pending` | QQ 全量阶段随媒体私信接入 |

## 更新规则

- 每个上游公开方法只计一次；复用统一端点不等于合并或遗漏上游参数与分支。
- 任一必需参数、响应字段、分页/刷新分支或登录要求未完成时，条目最高只能是 `partial`。
- 需要真实账户、VIP 或写操作验证时，离线完成后标为 `implemented`，并明确写出待验证前置条件。
- 上游新增公开方法先加入本表并重算分母；删除或历史重写必须保留审计证据，不能直接抹去记录。
- Basic 条目全部收口后进入 B 站 Basic；Q074–Q100 仍在后续 QQ 全量阶段逐项实现。
