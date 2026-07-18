# 网易云 Basic 阶段验收账本

最后更新：2026-07-18。上游基线与逐模块状态仍以 [`netease.md`](netease.md) 的 416 项全量账本为准；本表只把 Basic 范围聚合成可独立验收的能力单元，不替代或合并掉任何上游 API。

状态沿用全量账本：`pending` 尚未实现，`partial` 只覆盖部分必要模块或分支，`implemented` 已完成代码和离线验证但缺真实账户/后续 provider 前置条件，`verified` 已完成对应真实网络路径验收。一个聚合单元只有列出的必要分支全部达到相应状态时才能升级。

当前共 64 个验收单元：`pending=2`、`partial=6`、`implemented=20`、`verified=36`。

- 完整实现率：`(implemented + verified) / 64 = 56 / 64 = 87.50%`。
- 已触达率：`(partial + implemented + verified) / 64 = 62 / 64 = 96.88%`。
- 完整联网验收率：`verified / 64 = 36 / 64 = 56.25%`。

这些百分比是 Basic 能力验收口径，不是 416 个全量上游模块的完成率。`implemented` 仍算代码完成，但不能当作真实账户或真实跨平台成功态已经验证；切换到 QQ Basic 前，网易云 Basic 的 `pending/partial` 必须清零，跨 provider 前置条件造成的 `implemented` 项要在对应 provider 可用后补验。

当前剩余功能排序以完整播放体验为准：L11/L12 云盘写入、读取、详情、源文件下载、直接播放和删除已经用 TuneWeave 自建音频完成真实事务及完整回滚；匹配和文件内嵌歌词在 TuneWeave 与参考实现中均返回相同业务失败，不伪造成功态。主线已进入 C10/C11/P10 播客、电台节目与声音内容链路：分类、热门目录、详情、节目目录、节目播放/302、专用播客搜索及声音逐词转写已经验收，创作者工作台声音详情、声音歌单详情、歌单内声音目录、全筛选声音查询及账户创建的声音歌单快照也已完成代码与匿名认证边界；P07 的公开会员摘要及客户端完整权益后端均已实现，客户端成功态同其余账户能力留待 Basic 末尾集中联网验收。精选、个性化、分类热门、分类精选、今日优选、付费目录、播客横幅、播客榜、主播榜、节目榜及账户订阅代码已完成并留待集中验收，下一步按常用度补其他常规目录与播放权益缺口。

| ID | 范围 | 验收单元 | 状态 | 证据或当前缺口 |
| --- | --- | --- | --- | --- |
| S01 | 搜索与发现 | 11 类参考搜索、专用播客搜索及新版 `cloudsearch` | `verified` | 全部参考类型、分页和真实 HTTP 已验收；1009 明确映射按需播客而非直播广播，缺省播客搜索走 `/api/search/voicelist/get`；声音搜索优先专用字段，空旧数组/空结果不遮蔽非空兼容结构 |
| S02 | 搜索与发现 | 默认搜索词 | `verified` | `search_default` 已验收；空展示摘要会回退样式关键词 |
| S03 | 搜索与发现 | 简略及详细热搜 | `verified` | `search_hot/search_hot_detail` 已验收 |
| S04 | 搜索与发现 | Web、移动端及 PC 搜索建议 | `verified` | `search_suggest/search_suggest_pc` 已验收；未知首选类型不会遮蔽有效资源类型 |
| S05 | 搜索与发现 | 多重匹配与本地歌曲匹配 | `verified` | `search_multimatch/search_match` 命中和空结果均已验收；`orders=null` 会回退有效 `order` |
| S06 | 搜索与发现 | PC/Android/iPhone/iPad 横幅 | `verified` | `banner` 四分支已验收；并存时优先非空大图和主标题，空白首选值会继续回退普通图片/类型标题 |
| S07 | 搜索与发现 | 普通音乐榜单目录及详情 | `verified` | `toplist/toplist_detail/toplist_detail_v2/toplist_artist` 三类目录、四地区歌手榜及榜单曲目均已真实 HTTP 验收；新版首图优先于旧版首图 |
| S08 | 搜索与发现 | 首页个性化货架、新歌和 MV 推荐 | `pending` | `personalized*` 模块族未接入 |
| S09 | 搜索与发现 | 每日歌曲及歌单推荐 | `verified` | `recommend_songs` 已验证；2026-07-17 持久化真实账户实测 `recommend_resource` 返回 5 项 |
| S10 | 搜索与发现 | 音频指纹识别 | `implemented` | 无命中真实路径及映射已验证；无效 `startTime` 会回退可解析的 `start_time`，待有效指纹成功命中 |
| C01 | 内容展示 | 歌曲详情 | `verified` | `song_detail` 与统一 `Track` 已验收 |
| C02 | 内容展示 | 普通专辑目录、详情、曲目和动态统计 | `verified` | `album*` 常规展示链已验收 |
| C03 | 内容展示 | 数字专辑目录、详情及销量榜 | `verified` | `digitalAlbum*` 已接入的 Basic 展示链已验收；多艺人名称优先于单艺人摘要 |
| C04 | 内容展示 | 歌手目录、详情、专辑、歌曲及热门歌曲 | `verified` | 常规 `artist*` 展示链已验收 |
| C05 | 内容展示 | 歌单详情及完整曲目列表 | `verified` | `playlist_detail/playlist_track_all` 已验收 |
| C06 | 内容展示 | 普通、翻译、罗马音及逐字歌词 | `verified` | `lyric` 统一映射已验收；YRC 与 LRC 并存时以 `format=yrc` 标记最高同步能力并同时保留两者 |
| C07 | 内容展示 | MV/视频搜索、歌手视频目录和收藏态 | `partial` | 搜索与歌手目录已完成；实际视频 ID、专用标题/封面、正时长及非空完整创作者优先于空/零包装摘要，独立目录/收藏列表仍缺 |
| C08 | 内容展示 | MV/视频详情、分辨率和资源信息 | `implemented` | MV 详情及统计已真实验收；站内视频离线成功映射、真实失效资源 404 及统计路径已覆盖，待当前有效视频 ID 的详情成功态 |
| C09 | 内容展示 | 广播电台分类、地区、列表和当前节目 | `verified` | `broadcast_category_region_get/broadcast_channel_list/currentinfo` 已验收；收藏兼容结构的空包装及空分页别名不会遮蔽后续有效值 |
| C10 | 内容展示 | 播客/电台节目分类、详情和节目列表 | `partial` | `dj_catelist/dj_hot/dj_detail/dj_program/dj_program_detail/voicelist_search` 已通过 provider 与真实统一 HTTP/联网验收；精选、个性化、分类、今日、付费目录、播客横幅、新晋/热门/付费播客榜、新人/热门/24 小时主播榜、节目榜以及 `dj_sub/dj_sublist` 已完成稳定统一映射和离线 HTTP 验证；搜索解包排名包装为完整 `Podcast` 并保留算法/理由，1009 不再伪装直播广播；横幅目标明确使用 `podcast_episode` 而不伪装歌曲，榜单显式分离排名包装与完整资源，主播榜额外保留粉丝数和完整用户身份，节目榜可直接进入播放链，真实不生效的 offset 不会伪装成分页，不存在的翻页控制会被拒绝，账户列表语义优先于条目陈旧收藏态；登录成功分支留待 Basic 末尾集中验收，声音详情、声音歌单及其他常规目录仍待接入 |
| C11 | 内容展示 | 声音及声音歌单详情、目录和歌词 | `partial` | `voice_lyric` 已通过 provider 与真实统一 HTTP 验收，覆盖 675 段非空转写和 `data=null`；`voice_detail`、`voicelist_detail`、`voicelist_list`、`voicelist_list_search` 与 `voicelist_my_created` 分别以详情/目录的 `backend=workbench`、`/v1/account/podcast-episodes` 或 `/v1/account/podcasts/created` 接入独立能力和类型化输出，完整保留名称、七种审核状态、公开性、付费性、所属播客、包装字段合并、空/畸形首选列表回退、最大 200 条分页及不可续页快照语义，并实测匿名 301 认证边界，成功态留待 Basic 末尾集中验收；声音歌单与声音写入链路仍待接入 |
| C12 | 内容展示 | 用户公开资料与当前账户完整资料 | `partial` | 会员摘要已验证，`user_detail/user_detail_new` 未接入，账户资料待登录验收 |
| P01 | 播放与权益 | 可听性及请求/实际码率 | `verified` | `check_music` 可播与不可播路径已验收 |
| P02 | 播放与权益 | 旧版歌曲播放 URL 与精确 `br` | `verified` | `song_url` 单/批量和任意码率已真实验收；空白编码与零时长不遮蔽有效格式/歌曲时长 |
| P03 | 播放与权益 | 新版九档音质歌曲播放 URL | `implemented` | 九档真实 HTTP 均成功；跨平台成功源待后续 provider |
| P04 | 播放与权益 | 原生批量取流、保序、重复项和逐项失败 | `verified` | GET/POST 批量及旧/新版真实 HTTP 已验收 |
| P05 | 播放与权益 | 严格跨平台匹配、账户选择和失败回退 | `implemented` | 解析器、尝试轨迹和未注册来源回落已验收，待真实 QQ/酷狗等成功取流 |
| P06 | 播放与权益 | 专辑曲目可播、下载和最高音质权益 | `verified` | `album_privilege` 已验收；192/320 kbps 分别映射 `higher/high`，可用档位固定按能力升序去重，零新版最高码率回退有效兼容值 |
| P07 | 播放与权益 | 当前/公开 VIP 状态和完整客户端权益 | `implemented` | `vip_info` 已验证；`vip_info_v2` 以显式 `backend=client` 和独立能力接入，保留五类权益包并按服务器时间映射激活态/最长有效期，认证前置及离线成功映射已覆盖，待持久化真实账户成功态 |
| P08 | 播放与权益 | 广告换免费听、免费听时长及播放权益 | `implemented` | `ad_get` 与 `ad_listening_rights_gain` 已以独立统一能力接入，覆盖完整类型数组、`req_id` 提取、显式/自动请求 ID、完整领取参数、参考 GET/统一 POST、v3 checkToken 和不猜测未知 `gainFlag`；匿名真实目录返回合法空投放，领取链真实返回登录边界 `code=2001` 并映射 401，待持久化真实账户验证非空广告及成功领取 |
| P09 | 播放与权益 | MV/视频播放地址与清晰度 | `implemented` | MV 四档真实播放地址和 302 已验收；零首选清晰度/有效期不遮蔽兼容字段；站内视频离线成功与真实空 URL 业务态已覆盖，待当前有效视频 ID 的可播放成功态 |
| P10 | 播放与权益 | 播客、电台节目和声音播放地址 | `partial` | 节目先解析独立 `audio.ref`，再复用完整歌曲音质、VIP、账户、跨平台回退和 302 链路；声音逐词转写已接入且真实验证，工作台详情、声音歌单目录及账户声音查询都会把 `songId/trackId` 稳定映射为独立 `audio.ref`，待登录成功态确认后即可复用现有取流；声音写入后的完整播放事务仍待验收 |
| P11 | 播放与权益 | 歌曲下载地址及 302 重定向 | `verified` | `song_download_url/song_download_url_v1/song_url_v1_302` 的旧版、新版九档、无 URL 和播放兜底均已真实验收；空白编码与零时长回退有效元数据 |
| A01 | 账户与身份 | 国家和电话区号目录 | `verified` | `countries_code_list` 已验收 |
| A02 | 账户与身份 | 手机号注册状态和密码状态 | `verified` | `cellphone_existence_check` 两分支已验收 |
| A03 | 账户与身份 | 验证码独立校验 | `implemented` | 错误码真实路径已验收；空白 `message` 不遮蔽有效 `msg`，待有效验证码成功态 |
| A04 | 账户与身份 | 发送验证码及事务式验证码登录 | `implemented` | 完整代码和认证前置已覆盖，自动测试不主动发送短信 |
| A05 | 账户与身份 | 邮箱/账号密码登录 | `implemented` | `login` 已实现并脱敏，待真实账户成功态 |
| A06 | 账户与身份 | 手机号密码登录 | `implemented` | `login_cellphone` 密码分支已实现，待真实账户成功态 |
| A07 | 账户与身份 | 二维码 key、创建、图片和轮询确认 | `verified` | 2026-07-17 真实扫码已覆盖 waiting/scanned/confirmed，并验证凭据按 `platform/account` 落盘和无扫码重启恢复；空顶层 key/业务码不遮蔽嵌套有效值，真实 HTTP 创建同时返回 URL 与自包含 SVG data URL，不依赖外部二维码服务 |
| A08 | 账户与身份 | 登录状态查询 | `verified` | `login_status` 匿名真实路径已验收；空白/零账户身份不会误报已登录 |
| A09 | 账户与身份 | 会话刷新及退出 | `implemented` | 2026-07-17 真实账户刷新、凭据代际替换和重启恢复均已验收；退出会删除登录态，留待需要重新扫码时受控验证 |
| A10 | 账户与身份 | 当前账户资料 | `partial` | 2026-07-17 持久化真实账户的当前资料成功态已验收；空 `userId` 不遮蔽有效账户 ID，`user_detail/user_detail_new` 仍未接入 |
| L01 | 个人音乐库 | 喜欢歌曲 ID 及统一歌曲列表 | `verified` | 2026-07-17 持久化真实账户实测返回 5 项，ID 获取、详情映射和分页链路成功 |
| L02 | 个人音乐库 | 收藏/取消收藏专辑及专辑收藏列表 | `implemented` | 2026-07-17 真实账户收藏列表返回 5 项；收藏/取消收藏写入回滚仍待验收 |
| L03 | 个人音乐库 | 收藏/取消收藏广播电台及收藏列表 | `implemented` | 2026-07-17 真实账户收藏列表成功返回空列表；兼容结构中空旧列表不再遮蔽嵌套非空列表；收藏/取消收藏写入回滚仍待验收 |
| L04 | 个人音乐库 | 关注/取消关注歌手及关注列表 | `implemented` | 2026-07-17 真实账户关注列表返回 5 项；关注/取消关注写入回滚仍待验收 |
| L05 | 个人音乐库 | 当前账户歌单列表 | `implemented` | 2026-07-17 真实账户内容成功返回，但请求 `limit=5` 时上游仍返回完整列表，需先收口分页契约 |
| L06 | 个人音乐库 | 创建、编辑、删除歌单及增删/排序歌曲 | `implemented` | `playlist_create/delete/update/name/desc/tags/cover`、普通歌曲增删及 512 重试、VIDEO 歌单增删、歌曲顺序和账户歌单顺序均已接入统一 HTTP；零创建 ID 与空快照不会遮蔽有效别名，离线协议/认证前置完整，待真实账户事务写入与回滚 |
| L07 | 个人音乐库 | 全部/周播放历史 | `verified` | 2026-07-17 持久化真实账户实测全部历史返回 5 项，周历史成功返回空列表 |
| L08 | 个人音乐库 | 每日推荐歌曲 | `verified` | `recommend_songs` 匿名可用真实路径已验收 |
| L09 | 个人音乐库 | 每日推荐歌单 | `verified` | 2026-07-17 持久化真实账户实测返回 5 项 |
| L10 | 个人音乐库 | 私人 FM、跳过/不喜欢反馈和模式 | `pending` | `personal_fm/personal_fm_mode/recommend_songs_dislike` 未接入 |
| L11 | 个人音乐库 | 云盘上传、直传事务、导入、匹配和歌词 | `implemented` | 2026-07-17 以唯一生成的 MP3 完成代理上传、上传检查、NOS 票据与字节写入、登记发布、详情/下载/播放及同 MD5 导入，最终精确删除并恢复原有 209 项；标签按字段在主/备用标签间回退，零导入 ID 不遮蔽有效结果；合成素材的匹配与内嵌歌词请求在 TuneWeave 和参考实现中均未得到成功业务态，待合适素材补验 |
| L12 | 个人音乐库 | 云盘列表、详情、删除和直接播放 | `verified` | 2026-07-17 持久化真实账户在刷新及重启前后读取全部 209 项三页数据，兼容历史条目的空歌手/专辑字段；空 `simpleSong`、空云盘 ID 和零匹配 ID 不遮蔽有效兼容数据；详情、源文件下载、302、统一直接播放均成功，专用 `downloadUrl` 不会被通用 `url` 覆盖，并以唯一生成测试音频验证删除返回 200、列表恢复 209 项且无标记残留 |
| F01 | 平台基础协议 | EAPI 请求、响应解密与错误映射 | `verified` | 通用 API 与真实搜索已验收；2026-07-17 修正 Cookie 为 JavaScript `encodeURIComponent` 字符集后，真实账户作品流、喜欢列表、刷新及云盘下载均通过 |
| F02 | 平台基础协议 | WeAPI 双层 AES/RSA 请求 | `verified` | 通用 API 与真实搜索已验收 |
| F03 | 平台基础协议 | 未加密 API 请求 | `verified` | 通用 API 与真实搜索已验收 |
| F04 | 平台基础协议 | LinuxAPI 请求 | `verified` | 通用 API 与真实搜索已验收 |
| F05 | 平台基础协议 | XEAPI 密钥注册、签名和响应解密 | `verified` | 公钥注册和真实请求已验收 |
| F06 | 平台基础协议 | `e_r` 加密响应解包 | `verified` | EAPI 真实加密响应已验收 |
| F07 | 平台基础协议 | 安全原始 API 与批量 API 扩展 | `verified` | 五协议、动态键、域名和凭据边界已验收 |
| F08 | 平台基础协议 | 设备身份、匿名 token 与实时 checkToken | `implemented` | `register_checktoken_v2/v3` 已完成分版本路由、独立缓存、日志脱敏、EAPI/XEAPI 受控头注入及真实注册/缓存/强刷验收；`register_anonimous` 已完整接入参考设备编码、独立能力、三路由别名、私有持久化、重启恢复和默认公开请求自动复用，匿名身份不混入或覆盖多账户登录态。2026-07-18 TuneWeave 与当前参考实现同机实测均收到上游 `code=400` 且无 Cookie，代码会明确失败而不伪造身份，待上游恢复后补成功态验收 |
| F09 | 平台基础协议 | 随机中国 IP 与安全服务端代理/真实 IP 配置 | `implemented` | 已接入仅启动时可配置的 HTTP(S) 代理、固定 IPv4 或逐请求随机中国 IPv4，固定/随机身份互斥并同时写入 `X-Real-IP/X-Forwarded-For`；五种协议和 XEAPI 密钥注册共用策略，媒体下载/NOS 上传不附加来源头；随机生成采用参考的 `116.25–94.*.*` 紧凑兜底而不嵌入 68 KiB CIDR 表。通用 API 继续拒绝 `proxy/realIP/randomCNIP/headers` 注入；默认关闭、固定头、随机范围、冲突、代理 URL/凭据脱敏和配置边界均有测试，待在受控代理环境补真实出口验收 |

## 更新规则

- 每完成一个 Basic 小功能，同一提交或紧随其后的文档提交必须更新对应单元及四类计数。
- 一个单元包含多个上游模块时，任一必需模块仍为 `pending`，该单元最高只能是 `partial`。
- 真实账户、付费权益或后续 provider 是唯一未满足条件时使用 `implemented`，并在证据列写明待验收前置条件。
- 新上游模块若属于 Basic，先加入现有单元或新建验收单元，再重新计算分母和百分比；不得为保持百分比而省略新增能力。
