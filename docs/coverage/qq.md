# QQ 音乐 API 全量覆盖账本

上游快照：`L-1124/QQMusicApi@1b0aae0db3ee6876b3a77b8d1ce3057b4b3c9cd5`

本表逐项登记该快照 14 个公开 API 类的 100 个公开方法。QQMusicApi 是异步 Python SDK，不是 HTTP 服务；方法名用于固定验收分母，TuneWeave 将独立实现观察到的 QQ 音乐协议，不复制、翻译、链接或打包上游源码。内部辅助函数、会话封装、分页器和模型不重复计入业务方法分母，但 Basic 所需的平台协议单独列入 [`qq-basic.md`](qq-basic.md) 验收。

状态含义：

- `pending`：尚未完成统一映射或 QQ 扩展端点。
- `partial`：只完成部分参数、响应、协议分支或统一链路。
- `implemented`：代码与离线测试已完成，仍缺真实网络或账户前置验证。
- `verified`：统一端点、测试以及相应真实网络路径均已验证。

当前统计：`pending=96`、`partial=0`、`implemented=1`、`verified=3`。其中 QQ Basic 为 73 项，QQ 全量后续项为 27 项。实施顺序按普通音乐 App 的使用频率、播放依赖和底层必要性排列，不按类名或方法名字母排序。

| 编号 | 类别 | 上游公开方法 | Basic | 状态 | TuneWeave 映射/缺口 |
| --- | --- | --- | ---: | --- | --- |
| Q001 | 搜索与发现 | `SearchApi.get_hotkey` | 是 | `verified` | `GET /v1/search/trending?platform=qq&detail=...` 精确调用 Android `music.musicsearch.HotkeyService/GetHotkeyForQQMusicMobile` 并提交参考算法生成的 `search_id`。`vec_hotkey` 原始顺序映射为从 1 开始的稳定排名，实际搜索 `query` 不被活动展示 `title` 覆盖；`detail=full` 提供说明、字符串分值转无符号整数、趋势/序列类型、图标与跳转，`brief` 只收敛关键字和排名，但两种模式都在条目扩展保留标题、封面、热词/直达/歌曲 ID、置顶态、排序、趋势、来源及完整原项。`ret_code` 非零、缺失或目录缺失均拒绝为假成功；实验 ID、榜单时段、列表 ID 与完整响应保留在列表扩展。2026-07-22 provider 与 release 统一 HTTP 真实返回 30 项，首项排名 1“周杰伦”，full 分值存在、brief 富字段为空，上游码 0 |
| Q002 | 搜索与发现 | `SearchApi.complete` | 是 | `verified` | `GET /v1/search/suggestions?platform=qq&client=mobile&q=...` 精确调用 Android `music.smartboxCgi.SmartBoxCgi/GetSmartBoxResult`，参考固定的 `search_id/query/num_per_page=0/page_idx=0` 均保留。`items` 普通补全、`vec_related_items` 相关词和按 `insert_pos` 插入的 `vec_direct_items` 直达结果不会合并丢失；歌手直达结果提升为统一 `Artist`，其他已知类型保留 `kind`，无法安全提升的直达结构以含完整原文的 `opaque` 资源表达。搜索会话、展示高亮、图标、跳转、分值、关联 ID 和完整响应均保留，非数组桶拒绝为假空结果。2026-07-22 同一持久匿名设备的 provider 与 release 统一 HTTP 真实搜索“周杰伦”，返回 21 项，首项为 `artist/qq:0025NhlN2yWrP4`，上游码 0 |
| Q003 | 搜索与发现 | `SearchApi.quick_search` | 是 | `verified` | `GET /v1/search/suggestions?platform=qq&client=web&q=...` 精确调用固定 HTTPS `c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg`，查询参数经 URL 编码且不会开放任意域名、请求头或凭据注入。响应按各分区 `order` 动态排序，单曲、歌手、专辑、MV 分别提升为统一 `Track/Artist/Album/Video`，不会因 JSON 对象字段顺序变化而乱序；未来新增的未知分区仍逐项以携带完整原文的 `Opaque` 资源返回，不会静默丢弃。分区名称、顺序、类型、计数、原项和完整响应均保留；非零或缺失 `code/subcode`、缺失数据、已知分区缺失或畸形 `itemlist` 均拒绝为假成功。2026-07-22 provider 与 release 统一 HTTP 真实搜索“周杰伦”均通过，返回 10 项，依次覆盖 4 首单曲、2 位歌手、2 张专辑、2 个 MV，首项为 `track/qq:0039MnYb0qxYhV`“晴天”，上游 `code/subcode=0` |
| Q004 | 搜索与发现 | `SearchApi.search_by_type` | 是 | `implemented` | `GET /v1/search?platform=qq&kind=...` 已完整接入 Android `DoSearchForQQMusicMobile` 的歌曲、歌手、专辑、歌单、MV、歌词、用户、节目专辑和节目 9 类，并保留 `searchid` 搜索会话及 `highlight` 分支：自动申请并持久化 QIMEI/设备会话；按已验收的静默失败边界分别使用歌曲/专辑/MV/歌词 60、歌手 40、歌单 30 的页宽，用户/节目专辑/节目使用上游公开测试覆盖的 10，统一 `limit<=100` 与任意 `offset` 由同批子请求按上游逻辑槽位切片。歌单桶偶发少一项时不跨窗口补项或导致续页重复，`next_offset` 仍按槽位推进，`omitted_slots/upstream_item_counts` 明示缺口；非稀疏分类缺项及缺少码/总数/列表均拒绝为假成功。用户优先保留加密 UIN 并另存数字 UIN；节目专辑映射为 `Podcast`，节目映射为含完整可播放 `Track` 的 `PodcastEpisode`；所有类别均保留稳定身份、核心展示字段及完整原项。2026-07-22 Rust provider 与 release 统一 HTTP 已真实验收前 6 类及稀疏歌单跨页；最后 3 类的类型、字段优先级、分页和单批请求已有离线测试，但真实合并请求当前被 QQ 匿名风控返回 `code=2001`，待窗口解除后补统一 HTTP 验收，故标为 `implemented` 而非 `verified` |
| Q005 | 搜索与发现 | `SearchApi.general_search` | 是 | `pending` | 综合搜索及多字段续页游标 |
| Q006 | 搜索与发现 | `RecommendApi.get_home_feed` | 是 | `pending` | 首页推荐卡片和防重复游标 |
| Q007 | 搜索与发现 | `RecommendApi.get_recommend_songlist` | 是 | `pending` | 推荐歌单 |
| Q008 | 搜索与发现 | `RecommendApi.get_recommend_newsong` | 是 | `pending` | 分地区/语种新歌 |
| Q009 | 搜索与发现 | `RecommendApi.get_guess_recommend` | 是 | `pending` | 猜你喜欢 |
| Q010 | 搜索与发现 | `RecommendApi.get_radar_recommend` | 是 | `pending` | 雷达推荐 |
| Q011 | 搜索与发现 | `TopApi.get_category` | 是 | `pending` | 榜单目录 |
| Q012 | 搜索与发现 | `TopApi.get_detail` | 是 | `pending` | 榜单歌曲及分页 |
| Q013 | 内容展示 | `SongApi.query_song` | 是 | `pending` | 批量歌曲查询，分别保留数字 ID、MID、媒体 MID 和 `songType` |
| Q014 | 内容展示 | `SongApi.get_detail` | 是 | `pending` | 单曲详情 |
| Q015 | 内容展示 | `SongApi.get_similar_song` | 是 | `pending` | 相似歌曲 |
| Q016 | 内容展示 | `SongApi.get_labels` | 是 | `pending` | 歌曲标签 |
| Q017 | 内容展示 | `SongApi.get_related_songlist` | 是 | `pending` | 相关歌单 |
| Q018 | 内容展示 | `SongApi.get_related_mv` | 是 | `pending` | 相关 MV |
| Q019 | 内容展示 | `SongApi.get_other_version` | 是 | `pending` | 同曲其他版本 |
| Q020 | 内容展示 | `SongApi.get_producer` | 是 | `pending` | 制作人信息；排在高频链路之后 |
| Q021 | 内容展示 | `SongApi.get_sheet` | 是 | `pending` | 曲谱详情；排在高频链路之后 |
| Q022 | 内容展示 | `SongApi.has_sheet` | 是 | `pending` | 曲谱存在性；排在高频链路之后 |
| Q023 | 内容展示 | `SongApi.get_fav_num` | 是 | `pending` | 歌曲收藏人数 |
| Q024 | 内容展示 | `LyricApi.get_lyric` | 是 | `pending` | 普通/逐字、翻译、罗马音及 `song_type`，高精度格式优先 |
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
| Q037 | 内容展示 | `SonglistApi.get_detail` | 是 | `pending` | 歌单详情、标签、用户和完整歌曲分页 |
| Q038 | 内容展示 | `MvApi.get_detail` | 是 | `pending` | 批量 MV 详情 |
| Q039 | 内容展示 | `MvApi.get_mv_list` | 是 | `pending` | 地区、版本、排序 MV 目录 |
| Q040 | 播放与权益 | `SongApi.get_cdn_dispatch` | 是 | `pending` | CDN 调度 |
| Q041 | 播放与权益 | `SongApi.get_song_urls` | 是 | `pending` | 全文件规格、GUID/UIN/VKey、试听与 VIP 播放地址 |
| Q042 | 播放与权益 | `MvApi.get_mv_urls` | 是 | `pending` | MV 多清晰度播放地址 |
| Q043 | 登录与账户 | `LoginApi.check_expired` | 是 | `pending` | 凭据有效性和账户状态 |
| Q044 | 登录与账户 | `LoginApi.refresh_credential` | 是 | `pending` | 凭据刷新并原子替换账户代际 |
| Q045 | 登录与账户 | `LoginApi.logout` | 是 | `pending` | 上游退出并删除本地对应账户 |
| Q046 | 登录与账户 | `LoginApi.get_qrcode` | 是 | `pending` | QQ、微信和 QQ 音乐移动端二维码创建 |
| Q047 | 登录与账户 | `LoginApi.check_qrcode` | 是 | `pending` | QQ/微信扫码、确认、拒绝、过期和成功状态 |
| Q048 | 登录与账户 | `LoginApi.checking_mobile_qrcode` | 是 | `pending` | 移动端二维码 MQTT 状态链 |
| Q049 | 登录与账户 | `LoginApi.send_authcode` | 是 | `pending` | 手机验证码发送；同一挑战上下文贯穿验证链 |
| Q050 | 登录与账户 | `LoginApi.phone_authorize` | 是 | `pending` | 手机验证码登录及多账户持久化 |
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
| Q062 | 个人音乐库 | `UserApi.get_created_songlist` | 是 | `pending` | 用户创建歌单，可供 Uni Playlist 导入 |
| Q063 | 个人音乐库 | `UserApi.get_fav_song` | 是 | `pending` | 喜欢歌曲列表，可供 Uni Playlist 导入 |
| Q064 | 个人音乐库 | `UserApi.get_fav_songlist` | 是 | `pending` | 收藏歌单列表，可供 Uni Playlist 导入 |
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
