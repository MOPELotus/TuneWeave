# 网易云 Basic 阶段验收账本

最后更新：2026-07-22。上游基线与逐模块状态仍以 [`netease.md`](netease.md) 的 416 项全量账本为准；本表只把 Basic 范围聚合成可独立验收的能力单元，不替代或合并掉任何上游 API。

状态沿用全量账本：`pending` 尚未实现，`partial` 只覆盖部分必要模块或分支，`implemented` 已完成代码和离线验证但缺真实账户/后续 provider 前置条件，`verified` 已完成对应真实网络路径验收。一个聚合单元只有列出的必要分支全部达到相应状态时才能升级。

当前共 64 个验收单元：`pending=0`、`partial=4`、`implemented=18`、`verified=42`。

- 完整实现率：`(implemented + verified) / 64 = 60 / 64 = 93.75%`。
- 已触达率：`(partial + implemented + verified) / 64 = 64 / 64 = 100.00%`。
- 完整联网验收率：`verified / 64 = 42 / 64 = 65.63%`。

这些百分比是 Basic 能力验收口径，不是 416 个全量上游模块的完成率。`implemented` 仍算代码完成，但不能当作真实账户或真实跨平台成功态已经验证；切换到 QQ Basic 前，网易云 Basic 的 `pending/partial` 必须清零，跨 provider 前置条件造成的 `implemented` 项要在对应 provider 可用后补验。

当前剩余功能排序以完整播放体验为准：全部 64 个 Basic 单元现已触达，MV 与站内视频的目录、详情、收藏和播放链也已真实验收。下一步依重要度把 C10、C11、P10 三个 `partial` 单元补齐，优先完成声音歌单、常规播客目录和声音播放链；其余账户写入、完整权益及工作台成功态在 Basic 末尾使用现有持久化账户集中验收。只有这三项全部清零后才进入 Uni Playlist。

| ID | 范围 | 验收单元 | 状态 | 证据或当前缺口 |
| --- | --- | --- | --- | --- |
| S01 | 搜索与发现 | 11 类参考搜索、专用播客搜索及新版 `cloudsearch` | `verified` | 全部参考类型、分页和真实 HTTP 已验收；1009 明确映射按需播客而非直播广播，缺省播客搜索走 `/api/search/voicelist/get`；声音搜索优先专用字段，空旧数组/空结果不遮蔽非空兼容结构 |
| S02 | 搜索与发现 | 默认搜索词 | `verified` | `search_default` 已验收；空展示摘要会回退样式关键词 |
| S03 | 搜索与发现 | 简略及详细热搜 | `verified` | `search_hot/search_hot_detail` 已验收 |
| S04 | 搜索与发现 | Web、移动端及 PC 搜索建议 | `verified` | `search_suggest/search_suggest_pc` 已验收；未知首选类型不会遮蔽有效资源类型 |
| S05 | 搜索与发现 | 多重匹配与本地歌曲匹配 | `verified` | `search_multimatch/search_match` 命中和空结果均已验收；`orders=null` 会回退有效 `order` |
| S06 | 搜索与发现 | PC/Android/iPhone/iPad 横幅 | `verified` | `banner` 四分支已验收；并存时优先非空大图和主标题，空白首选值会继续回退普通图片/类型标题 |
| S07 | 搜索与发现 | 普通音乐榜单目录及详情 | `verified` | `toplist/toplist_detail/toplist_detail_v2/toplist_artist` 三类目录、四地区歌手榜及榜单曲目均已真实 HTTP 验收；新版首图优先于旧版首图 |
| S08 | 搜索与发现 | 首页个性化货架、新歌和 MV 推荐 | `verified` | `personalized/personalized_newsong/personalized_mv/personalized_djprogram/personalized_privatecontent/personalized_privatecontent_list` 六模块已按资源类型接入统一歌单、曲目、视频和播客节目端点，严格区分不可续页快照与独家放送真实分页；算法、文案、可反馈态及完整平台包装不丢失，浮点播放量不会被整数 DTO 拒绝。2026-07-18 匿名真实联网一次覆盖六分支，全部返回非空类型化资源 |
| S09 | 搜索与发现 | 每日歌曲及歌单推荐 | `verified` | `recommend_songs` 已验证；2026-07-17 持久化真实账户实测 `recommend_resource` 返回 5 项 |
| S10 | 搜索与发现 | 音频指纹识别 | `implemented` | 无命中真实路径及映射已验证；无效 `startTime` 会回退可解析的 `start_time`，待有效指纹成功命中 |
| C01 | 内容展示 | 歌曲详情 | `verified` | `song_detail` 与统一 `Track` 已验收 |
| C02 | 内容展示 | 普通专辑目录、详情、曲目和动态统计 | `verified` | `album*` 常规展示链已验收 |
| C03 | 内容展示 | 数字专辑目录、详情及销量榜 | `verified` | `digitalAlbum*` 已接入的 Basic 展示链已验收；多艺人名称优先于单艺人摘要 |
| C04 | 内容展示 | 歌手目录、详情、专辑、歌曲及热门歌曲 | `verified` | 常规 `artist*` 展示链已验收 |
| C05 | 内容展示 | 歌单详情及完整曲目列表 | `verified` | `playlist_detail/playlist_track_all` 已验收 |
| C06 | 内容展示 | 普通、翻译、罗马音及逐字歌词 | `verified` | `lyric` 统一映射已验收；YRC 与 LRC 并存时以 `format=yrc` 标记最高同步能力并同时保留两者 |
| C07 | 内容展示 | MV/视频搜索、歌手视频目录和收藏态 | `verified` | 搜索、歌手目录、全部/最新/网易出品 MV 目录、站内全部/推荐/分组视频时间线、9 项分类与 107 项标签，以及 MV/普通视频分协议收藏和账户混合收藏列表均已完成并真实验证；时间线外层算法与内层视频不丢失，`datas=null` 合法为空页，累计 63 次当前分组请求均保留上游 200 空态；两次收藏闭环后账户状态均已恢复 |
| C08 | 内容展示 | MV/视频详情、分辨率和资源信息 | `verified` | MV 详情及统计已真实验收；2026-07-22 又从账户收藏列表取得当前有效普通视频，真实验证详情、正时长、4 档资源及统计成功态；失效资源 404 仍保持原始业务语义 |
| C09 | 内容展示 | 广播电台分类、地区、列表和当前节目 | `verified` | `broadcast_category_region_get/broadcast_channel_list/currentinfo` 已验收；收藏兼容结构的空包装及空分页别名不会遮蔽后续有效值 |
| C10 | 内容展示 | 播客/电台节目分类、详情和节目列表 | `partial` | `dj_catelist/dj_category_excludehot/dj_category_recommend/dj_hot/dj_detail/dj_program/dj_program_detail/voicelist_search/program_recommend/record_recent_voice/dj_difm_all_style_channel/dj_difm_playing_tracks_list/dj_difm_subscribe_channels_get` 已通过 provider 与真实统一 HTTP/联网验收；2026-07-22 分类链分别返回 19 个完整分类、13 个非热门分类和 12 个含完整播客的推荐分组，分类节目推荐的 offset 0/2 也返回两组不同完整节目及可播放音频；持久账户最近声音返回 2 条记录，节目、承载音频、播放时间和终端完整分离；DiFM 电子/古典/爵士三源真实返回 15/8/12 个风格及 252/70/103 个频道，来源、风格、频道层级和 source-qualified 引用完整保留；频道 `netease:difm:0:10505` 的当前队列真实返回 5 条独立 DiFM 播放项，首条 351 秒、2001 个波形采样且直链可用，不伪装普通歌曲；三源账户收藏快照真实返回 `code=200/total=0`，订阅与取消链已完整接入同一 source-qualified 资源路径但未改动真实账户收藏；分组没有被错误压平或丢弃，上游不可靠的 `more=false` 和没有续页控制的账户快照均不会被伪造成续页；精选、个性化、分类热门、今日、付费目录、播客横幅、新晋/热门/付费播客榜、新人/热门/24 小时主播榜、节目榜以及 `dj_sub/dj_sublist` 已完成稳定统一映射和离线 HTTP 验证；搜索解包排名包装为完整 `Podcast` 并保留算法/理由，1009 不再伪装直播广播；横幅目标明确使用 `podcast_episode` 而不伪装歌曲，榜单显式分离排名包装与完整资源，主播榜额外保留粉丝数和完整用户身份，节目榜可直接进入播放链，真实不生效的 offset 不会伪装成分页，不存在的翻页控制会被拒绝，账户列表语义优先于条目陈旧收藏态；登录成功写入分支留待有可安全回滚内容时集中验收 |
| C11 | 内容展示 | 声音及声音歌单详情、目录和歌词 | `partial` | `voice_lyric` 已通过 provider 与真实统一 HTTP 验收，覆盖 675 段非空转写和 `data=null`；`voice_detail`、`voicelist_detail`、`voicelist_list`、`voicelist_list_search` 与 `voicelist_my_created` 分别以详情/目录的 `backend=workbench`、`/v1/account/podcast-episodes` 或 `/v1/account/podcasts/created` 接入独立能力和类型化输出，完整保留名称、七种审核状态、公开性、付费性、所属播客、包装字段合并、空/畸形首选列表回退、最大 200 条分页及不可续页快照语义，并实测匿名认证边界；`voicelist_trans` 已以独立统一写能力接入声音排序，完整保留参考分页定位、同平台身份和 1 基序号，并真实到达上游所有权边界；`voice_delete` 已按实际声音 ID 语义接入单条/批量删除，完整保留原始顺序、重复项、平台身份与参考逗号协议；`voice_upload` 已完整迁移令牌、10 MiB NOS 分片、XML 完成、预检查和正式提交事务，全部发布/隐私/分类/排序/包含歌曲参数均进入稳定统一模型且不泄露音频或 token；三条写链均已验证发网前账户边界，真实上传后的详情/播放、拥有者排序及破坏性删除成功态留待创作者账户和可丢弃声音集中验收 |
| C12 | 内容展示 | 用户公开资料与当前账户完整资料 | `verified` | `user_detail/user_detail_new` 已以同一 `UserProfile` 的显式 legacy/modern 后端完整接入，平台原始资料不丢失；2026-07-22 公开两后端及持久账户 modern 路径均真实 HTTP 验收成功 |
| P01 | 播放与权益 | 可听性及请求/实际码率 | `verified` | `check_music` 可播与不可播路径已验收 |
| P02 | 播放与权益 | 旧版歌曲播放 URL 与精确 `br` | `verified` | `song_url` 单/批量和任意码率已真实验收；空白编码与零时长不遮蔽有效格式/歌曲时长 |
| P03 | 播放与权益 | 新版九档音质歌曲播放 URL | `partial` | 九档真实 HTTP 均成功；2026-07-22 上游新增 `sky` 的 `immerseType=c51|ste|aac` 选择，当前只固定 `c51`，需补输入与协议测试；跨平台成功源待后续 provider |
| P04 | 播放与权益 | 原生批量取流、保序、重复项和逐项失败 | `verified` | GET/POST 批量及旧/新版真实 HTTP 已验收 |
| P05 | 播放与权益 | 严格跨平台匹配、账户选择和失败回退 | `implemented` | 解析器、尝试轨迹和未注册来源回落已验收，待真实 QQ/酷狗等成功取流 |
| P06 | 播放与权益 | 专辑曲目可播、下载和最高音质权益 | `verified` | `album_privilege` 已验收；192/320 kbps 分别映射 `higher/high`，可用档位固定按能力升序去重，零新版最高码率回退有效兼容值 |
| P07 | 播放与权益 | 当前/公开 VIP 状态和完整客户端权益 | `implemented` | `vip_info` 已验证；`vip_info_v2` 以显式 `backend=client` 和独立能力接入，保留五类权益包并按服务器时间映射激活态/最长有效期，认证前置及离线成功映射已覆盖，待持久化真实账户成功态 |
| P08 | 播放与权益 | 广告换免费听、免费听时长及播放权益 | `implemented` | `ad_get` 与 `ad_listening_rights_gain` 已以独立统一能力接入，覆盖完整类型数组、`req_id` 提取、显式/自动请求 ID、完整领取参数、参考 GET/统一 POST、v3 checkToken 和不猜测未知 `gainFlag`；匿名真实目录返回合法空投放，领取链真实返回登录边界 `code=2001` 并映射 401，待持久化真实账户验证非空广告及成功领取 |
| P09 | 播放与权益 | MV/视频播放地址与清晰度 | `verified` | MV 四档真实播放地址和 302 已验收；零首选清晰度/有效期不遮蔽兼容字段；2026-07-22 当前有效普通视频真实返回 480p 非空 URL、`available=true/actual_resolution=480`，统一重定向返回 302，空 URL 业务态仍有独立回归 |
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
| A10 | 账户与身份 | 当前账户资料 | `verified` | 账户摘要、持久化恢复及完整资料均已验收；空/零 `userId` 不遮蔽有效账户 ID，`GET /v1/account/profile` 会按所选账户解析身份并调用显式 legacy/modern 资料后端，2026-07-22 `manual-sms` modern 路径真实成功 |
| L01 | 个人音乐库 | 喜欢歌曲 ID 及统一歌曲列表 | `verified` | 2026-07-17 持久化真实账户实测返回 5 项，ID 获取、详情映射和分页链路成功 |
| L02 | 个人音乐库 | 收藏/取消收藏专辑及专辑收藏列表 | `implemented` | 2026-07-17 真实账户收藏列表返回 5 项；收藏/取消收藏写入回滚仍待验收 |
| L03 | 个人音乐库 | 收藏/取消收藏广播电台及收藏列表 | `implemented` | 2026-07-17 真实账户收藏列表成功返回空列表；兼容结构中空旧列表不再遮蔽嵌套非空列表；收藏/取消收藏写入回滚仍待验收 |
| L04 | 个人音乐库 | 关注/取消关注歌手及关注列表 | `implemented` | 2026-07-17 真实账户关注列表返回 5 项；关注/取消关注写入回滚仍待验收 |
| L05 | 个人音乐库 | 当前账户歌单列表 | `implemented` | 2026-07-17 真实账户内容成功返回，但请求 `limit=5` 时上游仍返回完整列表，需先收口分页契约 |
| L06 | 个人音乐库 | 创建、编辑、删除歌单及增删/排序歌曲 | `implemented` | `playlist_create/delete/update/name/desc/tags/cover`、普通歌曲增删及 512 重试、VIDEO 歌单增删、歌曲顺序和账户歌单顺序均已接入统一 HTTP；零创建 ID 与空快照不会遮蔽有效别名，离线协议/认证前置完整，待真实账户事务写入与回滚 |
| L07 | 个人音乐库 | 全部/周播放历史 | `verified` | 2026-07-17 持久化真实账户实测全部历史返回 5 项，周历史成功返回空列表 |
| L08 | 个人音乐库 | 每日推荐歌曲 | `verified` | `recommend_songs` 匿名可用真实路径已验收 |
| L09 | 个人音乐库 | 每日推荐歌单 | `verified` | 2026-07-17 持久化真实账户实测返回 5 项 |
| L10 | 个人音乐库 | 私人 FM、跳过/不喜欢反馈和模式 | `implemented` | `personal_fm/personal_fm_mode` 已以同一统一队列端点的经典/模式后端接入，分别精确使用 WeAPI/EAPI `/api/v1/radio/get`，不可续页快照不伪造分页；2026-07-18 匿名真实联网两分支均返回非空统一曲目。`recommend_songs_dislike` 已接入统一写入端点，精确提交 `resType=4/sceneType=1`，匿名真实路径正确映射登录要求，待持久化账户成功写入验收 |
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
