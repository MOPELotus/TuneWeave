# B 站 Basic 覆盖账本

协议基线为 `nilaoda/BBDown@259a5558b1edc8aed054cd113f4ce3213886c929` 与 `bilibili-plugins/bilibili-api-collect@cfc5fddc446f8e82ea15ea32c42de425274779cc`。BBDown 用于核对视频身份解析、分 P 与 DASH 音视频取流行为；`bilibili-api-collect` 用于核对登录、搜索、用户空间、公开合集和收藏夹协议，不作为源码依赖。

状态沿用其他平台账本：`pending` 尚未实现，`partial` 缺少必要分支，`implemented` 已完成代码和离线验证但缺真实账户或真实网络成功态，`verified` 已完成对应真实路径验收。当前共 34 个验收单元：`pending=19`、`partial=0`、`implemented=6`、`verified=9`；完整实现率与已触达率均为 `15/34 = 44.12%`。

Basic 只覆盖普通音视频客户端必需的登录、搜索、个人/公开列表、Uni Playlist 导入、视频信息、封面、分 P、仅音频播放及下载链。专栏、直播、漫画、游戏、钱包、装扮和纯社交功能不纳入 B 站范围；与视频/音频、播放列表或账户直接相关但低频的能力仍登记到后续 B 站全量账本，不能因不属于 Basic 而静默遗漏。

| ID | 阶段 | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- | --- |
| BF01 | 平台基础 | `bilibili:` 资源命名空间 | `verified` | `Platform`、统一引用解析和 API 包络已支持 B 站；视频、合集与收藏夹仍必须使用下列带类型身份，不能把数字 ID 混用 |
| BF02 | 平台基础 | AV/BV/EP/SS 与 URL 输入解析 | `verified` | 强类型区分 AID、BVID、Episode 与 Season，接受规范短 ID、`bilibili:` 引用及固定 `bilibili.com` Web 主机 URL，并输出稳定 `aid:/bvid:/ep:/season:` 身份。BVID 不再像参考工具一样提前丢失为 AID，EP/SS 来源保持可辨；纯数字、冲突参数、短链、用户信息、非默认端口、外域和未知页面均在发网前拒绝，解析过程无重定向或页面抓取副作用 |
| BF03 | 平台基础 | 固定域名 HTTP 客户端与业务错误 | `implemented` | Passport、Web API、搜索和票据端点均为代码内固定 HTTPS 地址，调用方不能注入 URL、Cookie 或请求头；客户端统一限制重定向、连接/总超时和响应体大小，并把 HTTP/业务层的登录失效、权限、风控、限流、资源不存在和上游故障映射为统一错误。后续媒体链接入时仍需把 CDN/备用地址加入媒体专用白名单并完成真实取流验收 |
| BF04 | 平台基础 | WBI、buvid 与设备身份 | `verified` | provider 从固定 SPI 与主页响应取得并缓存 `buvid3/buvid4/b_nut`，从固定 Web Ticket 接口取得短期 `bili_ticket` 与 WBI 图片密钥；WBI 参数排序、过滤、Web 百分号编码、混淆表、时间戳和 MD5 签名按强类型独立实现，密钥 URL 仅接受可信 `hdslb.com/bfs/wbi`。设备、票据、导航回退及现行搜索签名链均已真实联网运行，票据、Cookie 和完整签名查询不会进入 Debug 或错误响应 |
| BF05 | 平台基础 | 强类型凭证、多账户与调用方托管 | `implemented` | `bilibili_cookie_v1` 强类型保存并校验 `DedeUserID/DedeUserID__ckMd5/SESSDATA/bili_jct/sid/refresh_token`，Debug 与错误不回显秘密；二维码确认从首个账户功能起支持 `(bilibili, account)` 及 `server/client/both`，调用方凭证平台、类型、到期语义和内部字段均在发网前验证。三种归属模式和账户隔离已离线验收，待真实扫码确认后联合升为 `verified` |
| BA01 | 登录账户 | Web 二维码创建 | `verified` | 固定调用 `x/passport-login/web/qrcode/generate?source=main-fe-header`，同时兼容并严格校验当前 `account.bilibili.com/.../scan-web` 与旧版 Passport 扫码地址；二维码由进程内生成自包含 SVG，平台 key 只进入有期限的服务端事务。已真实创建并验证可轮询的二维码 |
| BA02 | 登录账户 | Web 二维码轮询与状态机 | `implemented` | 按 BBDown 链路固定调用 `x/passport-login/web/qrcode/poll`，完整区分 `86101` 未扫码、`86090` 已扫码待确认、`86038` 过期、`0` 成功及其他失败码；确认时优先从重复 `Set-Cookie` 提取凭据，仅在必需字段缺失时从固定 `crossDomain` 地址回填，成功凭证只按事务固定归属模式交付一次。未扫码真实网络态及全部响应分支已通过，真实扫码成功态待账户联合验收 |
| BA03 | 登录账户 | 登录 captcha 挑战 | `pending` | 获取 GeeTest challenge/gt/token；TuneWeave 不绕过验证码，由调用方提交人工验证结果继续密码或短信流程 |
| BA04 | 登录账户 | Web 密码登录 | `pending` | 获取 RSA 公钥与 salt、加密密码并保留风控二次验证分支；密码与验证码不得持久化或进入日志 |
| BA05 | 登录账户 | 国家/地区电话区号 | `pending` | 对接 Web country list 并映射统一国家区号模型 |
| BA06 | 登录账户 | Web 短信验证码发送 | `pending` | 复用同一 captcha 事务和设备身份；手机号不持久化，发送与登录不得切换网络身份 |
| BA07 | 登录账户 | Web 短信验证码登录 | `pending` | 复用发送阶段产生的 captcha key，完整处理绑定、风控和登录成功 Cookie |
| BA08 | 登录账户 | 会话状态与账户资料 | `implemented` | `GET /v1/auth/session` 与 `/v1/account/profile` 固定调用 `x/web-interface/nav`，强类型映射登录态、UID、昵称、头像、验证状态、等级、认证、挂件、大会员、钱包及 WBI 实时口令等已知结构；登录 UID 必须与选中凭证一致，头像只接受 B 站 HTTPS 图片域名。`-101` 和 `isLogin=false` 作为未认证正常结果，不存在的精确别名不发网且不回退 `default`；调用方凭证与服务器多账户共用同一链路。匿名真实网络态及离线完整/畸形分支已通过，登录账户成功态待扫码联合验收 |
| BA09 | 登录账户 | Cookie 刷新与退出 | `implemented` | `POST /v1/auth/session/refresh` 完整执行 Cookie 刷新状态检查、固定公钥 RSA-OAEP `correspondPath`、实时 `refresh_csrf`、新 Cookie/refresh token 轮换、旧 refresh token 确认及新会话身份检查；无需刷新时验证旧会话并按归属模式返回同一代际，任一步失败均不覆盖服务器凭据。`DELETE /v1/auth/session` 固定调用 Web 退出接口，只在上游确认退出或明确返回失效登录页后删除精确账户；网络、CSRF 和未知错误保留旧凭据。`server/client/both` 的来源隔离、同 UID 检查、原子替换/删除、响应脱敏和全部状态解析已离线验收，待真实扫码账户完成刷新与退出联合验证 |
| BS01 | 搜索 | 视频直接搜索 | `implemented` | `GET /v1/search?platform=bilibili&kind=video` 已接入视频专用搜索：先尝试现行 WBI 端点，若平台明确返回风险票据则在十分钟内使用仍可用的公开兼容端点，不申请、回显或自动处理 captcha。统一分页可跨上游页满足 `limit/offset`，结果强类型保留 AID/BVID、UP 主、封面、时长、分区、标签、命中列、计数、发布时间及付费/合作标志，未知 HTML 不会进入标题。统一 `order` 完整覆盖综合、播放、最新、弹幕、收藏和评论排序，`duration` 覆盖平台五档时长，`category_id/tids` 保留正整数分区 ID；三类筛选在 HTTP、核心模型、provider 和两套搜索端点间均有强类型映射，其他平台会明确拒绝而不静默忽略。默认排序真实返回过结果，筛选分支完成离线验收；当前出口后续重复验收触发 HTTP 412，因此待筛选真实成功态后升级为 `verified` |
| BS02 | 搜索 | 搜索建议 | `pending` | Web suggestion；关键词与展示高亮分离，空建议返回空列表而非错误 |
| BS03 | 搜索 | 热门搜索 | `pending` | 热搜词、展示文本、排行与跳转元数据强类型映射；不跟随任意外部 URL |
| BP01 | 列表与 Uni | 用户创建的收藏夹目录 | `verified` | `GET /v1/users/{bilibili:mid}/playlists/created` 的收藏夹分支固定调用 `x/v3/fav/folder/created/list-all` 并限定视频收藏类型；公开目录可匿名读取，`account` 或调用方凭证只在明确选择时附带，不存在的账户别名在发网前失败。目录强类型校验所有者、数量、唯一完整 `media_id`、原始 `fid`、属性位、收藏状态和儿童模式字段，统一 Playlist 保留默认/自建、公开/私有、视频数量和稳定 `bilibili:favorite:<media_id>` 身份。该统一目录现在按收藏夹、Season/Series 的固定顺序合并 BP03 公开空间列表；收藏夹隐藏时只标记 `favorite_folders_hidden=true`，不会连带遮蔽公开视频合集。公开用户 `7792521` 的收藏夹与合并后 provider 链均已真实联网验收 |
| BP02 | 列表与 Uni | 用户收藏的播放列表目录 | `verified` | `GET /v1/users/{bilibili:mid}/playlists/favorite` 固定调用 `x/v3/fav/folder/collected/list`，以 `platform=web` 完整保留普通收藏夹 `type=11` 与用户收藏的视频合集 `type=21`，分别输出 `bilibili:favorite:<media_id>` 和 `bilibili:season:<season_id>`，不会混作本地创建目录。统一偏移分页可跨上游 70 项页，校验总数、续页、重复身份和类型漂移；条目保留所有者、封面、简介、时间、属性、收藏/失效/置顶状态、浏览与视频数量，HTTP 图片只升级到受信 B 站 HTTPS 图床。公开用户 `293793435` 的匿名混合类型目录和 provider 偏移分页均已真实联网验收；隐藏目录与精确账户选择分别保持权限错误和账户隔离 |
| BP03 | 列表与 Uni | 用户公开合集/系列目录 | `verified` | 现行 WBI、设备 Cookie 与 Web Ticket 链固定调用 `x/polymer/web-space/seasons_series_list`，强类型校验混合分页、所有者、Season/Series 身份、预览 BVID/AID、最近 AID 和重复项；Season 输出 `bilibili:season:<season_id>`，Series 输出 `bilibili:series:<series_id>`，保留封面、简介、显示标题、类别、数量、发布时间，以及系列状态、关键词、创建/更新时间和 creator mode。统一创建目录在收藏夹之后跨上游 20 项页裁出任意 `offset/limit`，总数与下一偏移覆盖两个来源；用户 `37737161` 的 WBI 混合目录、Season/Series 分型和合并 provider 链已真实联网验收 |
| BP04 | 列表与 Uni | 公开合集详情 | `verified` | `GET /v1/playlists/bilibili:season:<season_id>` 以强类型显式拒绝无类型、零值、前导零和未知列表身份，固定调用现行 WBI `x/polymer/web-space/seasons_archives_list`；请求使用平台允许的 `mid=0` 解析真实所有者，因此直接引用不要求调用方重复提供 owner mid。响应交叉校验 `season_id`、所有者、meta/page 总数、AID 顺序、BVID、分页、状态、付费与播放位置，并将首屏档案与合集封面、简介、类别、发布时间和数量共同映射为稳定 Playlist；精确账户和调用方凭证继续可选且隔离。需求样例 `season:3629748` 已真实解析出 owner `327961371`、总数 617 和 30 项首屏 |
| BP05 | 列表与 Uni | 公开合集视频分页 | `verified` | `GET /v1/playlists/bilibili:season:<season_id>/items` 复用现行 WBI 合集档案协议，以强类型 `VideoDetail` 保留视频身份；`/tracks` 提供面向纯音乐客户端的兼容视图并显式标记 `normalized_from_video`，不会影响 Uni 导入时的视频类型。任意 `offset/limit` 可跨上游固定 30 项页，遍历期间校验总数、合集、所有者和重复 AID/BVID，结果保留顺序、封面、时长、发布时间、互动/付费/状态、播放位置、播放及弹幕数；详情尚未解析的简介、UP 主昵称、分 P/CID 和清晰度不会伪造。需求样例 `season:3629748` 已真实验证从 offset 28 取 5 项跨两页，返回总数 617、下一偏移 33 和 5 个稳定 BV 视频引用 |
| BP06 | 列表与 Uni | 收藏夹详情 | `pending` | `bilibili:favorite:<media_id>`；同时保留 `media_id/fid` 与所有者 `mid`，公开和账户可见状态不能混淆 |
| BP07 | 列表与 Uni | 收藏夹视频分页 | `pending` | `x/v3/fav/resource/list`；处理失效条目、多 P 视频、公开/私有权限及完整分页 |
| BP08 | 列表与 Uni | 合集与收藏夹导入 Uni Playlist | `pending` | `type=season/favorite_folder` 均完整遍历分页并原子导入；覆盖需求样例 `season:3629748` 与 `favorite:2883236382`，视频保存为可播放项目而非伪造歌曲 |
| BV01 | 视频展示 | 视频详情与封面 | `pending` | 统一 `VideoDetail` 保留标题、简介、封面、UP 主、发布时间、AID/BVID、状态与可用清晰度 |
| BV02 | 视频展示 | 分 P 目录 | `pending` | `GET /v1/videos/{ref}/parts` 返回稳定 CID、页码、标题、尺寸与时长；多 P 不默认丢弃非首 P |
| BV03 | 视频展示 | 视频统计 | `pending` | 播放、点赞、投币、收藏、评论和分享计数按统一字段映射；账户点赞态与公开计数分离 |
| BV04 | 视频展示 | 字幕目录与正文 | `pending` | 字幕是 B 站最接近歌词的时间文本能力；保留语言、名称、AI/人工类型、时间段和文本，不伪装为逐字歌词 |
| BM01 | 播放下载 | DASH 播放信息 | `pending` | 以 AID/BVID + CID 请求 playurl，保留 DASH、DURL、格式、清晰度、编码、大小和备用 URL 分支 |
| BM02 | 播放下载 | 仅音频轨道选择 | `pending` | 按实际音频 ID、码率、编码和账户权益选择主/备用 URL；不下载或合并媒体字节到 API 服务端 |
| BM03 | 播放下载 | 视频轨道选择 | `pending` | 统一视频流保留请求/实际清晰度、帧率、HDR/Dolby/AV1 等真实能力，不把降级清晰度标成请求值 |
| BM04 | 播放下载 | 统一播放、下载与 302 | `pending` | 接入 `/videos/.../stream`、仅音频播放/下载和 redirect；返回必要 `Referer`/Cookie 时必须脱敏并限制到媒体请求 |
| BM05 | 播放下载 | 账户权益与跨平台回退 | `pending` | 高码率/大会员状态使用精确 B 站账户或调用方凭证；视频音频可作为 Uni 原始来源，失败后才按严格元数据匹配回退其他音乐平台 |

## 实施顺序

1. BF02–BF05 与 BA01–BA09：先形成可持久、可由调用方托管的真实登录与多账户底座。
2. BS01–BS03：接通直接视频搜索、建议与热搜。
3. BP01–BP08：完成个人目录、公开合集、收藏夹和 Uni Playlist 双来源导入。
4. BV01、BV02、BV04 与 BM01–BM05：完成封面、分 P、字幕、仅音频播放/下载及视频播放链。
5. BV03：统计等非播放阻塞展示在上述链路稳定后补齐。
