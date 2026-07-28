# B 站 Basic 覆盖账本

协议基线为 `nilaoda/BBDown@259a5558b1edc8aed054cd113f4ce3213886c929` 与 `bilibili-plugins/bilibili-api-collect@cfc5fddc446f8e82ea15ea32c42de425274779cc`。BBDown 用于核对视频身份解析、分 P 与 DASH 音视频取流行为；`bilibili-api-collect` 用于核对登录、搜索、用户空间、公开合集和收藏夹协议，不作为源码依赖。

状态沿用其他平台账本：`pending` 尚未实现，`partial` 缺少必要分支，`implemented` 已完成代码和离线验证但缺真实账户或真实网络成功态，`verified` 已完成对应真实路径验收。当前共 34 个验收单元：`pending=29`、`partial=0`、`implemented=3`、`verified=2`，代码完成度 `5/34 = 14.71%`。

Basic 只覆盖普通音视频客户端必需的登录、搜索、个人/公开列表、Uni Playlist 导入、视频信息、封面、分 P、仅音频播放及下载链。专栏、直播、漫画、游戏、钱包、装扮和纯社交功能不纳入 B 站范围；与视频/音频、播放列表或账户直接相关但低频的能力仍登记到后续 B 站全量账本，不能因不属于 Basic 而静默遗漏。

| ID | 阶段 | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- | --- |
| BF01 | 平台基础 | `bilibili:` 资源命名空间 | `verified` | `Platform`、统一引用解析和 API 包络已支持 B 站；视频、合集与收藏夹仍必须使用下列带类型身份，不能把数字 ID 混用 |
| BF02 | 平台基础 | AV/BV/EP/SS 与 URL 输入解析 | `pending` | 参考 BBDown 先归一到视频 AID/BVID；有分集身份时保留 EP/SS 来源，URL 只解析固定 B 站主机，不发起任意跳转 |
| BF03 | 平台基础 | 固定域名 HTTP 客户端与业务错误 | `pending` | 只允许已审查的 B 站 API、Passport、搜索和媒体域名；统一映射登录失效、权限、风控、限流、资源不存在和上游错误 |
| BF04 | 平台基础 | WBI、buvid 与设备身份 | `pending` | WBI 密钥、混淆表、时间戳和设备 Cookie 由 provider 管理；不得允许调用方覆盖签名、URL、代理或请求头 |
| BF05 | 平台基础 | 强类型凭证、多账户与调用方托管 | `implemented` | `bilibili_cookie_v1` 强类型保存并校验 `DedeUserID/DedeUserID__ckMd5/SESSDATA/bili_jct/sid/refresh_token`，Debug 与错误不回显秘密；二维码确认从首个账户功能起支持 `(bilibili, account)` 及 `server/client/both`，调用方凭证平台、类型、到期语义和内部字段均在发网前验证。三种归属模式和账户隔离已离线验收，待真实扫码确认后联合升为 `verified` |
| BA01 | 登录账户 | Web 二维码创建 | `verified` | 固定调用 `x/passport-login/web/qrcode/generate?source=main-fe-header`，同时兼容并严格校验当前 `account.bilibili.com/.../scan-web` 与旧版 Passport 扫码地址；二维码由进程内生成自包含 SVG，平台 key 只进入有期限的服务端事务。已真实创建并验证可轮询的二维码 |
| BA02 | 登录账户 | Web 二维码轮询与状态机 | `implemented` | 按 BBDown 链路固定调用 `x/passport-login/web/qrcode/poll`，完整区分 `86101` 未扫码、`86090` 已扫码待确认、`86038` 过期、`0` 成功及其他失败码；确认时优先从重复 `Set-Cookie` 提取凭据，仅在必需字段缺失时从固定 `crossDomain` 地址回填，成功凭证只按事务固定归属模式交付一次。未扫码真实网络态及全部响应分支已通过，真实扫码成功态待账户联合验收 |
| BA03 | 登录账户 | 登录 captcha 挑战 | `pending` | 获取 GeeTest challenge/gt/token；TuneWeave 不绕过验证码，由调用方提交人工验证结果继续密码或短信流程 |
| BA04 | 登录账户 | Web 密码登录 | `pending` | 获取 RSA 公钥与 salt、加密密码并保留风控二次验证分支；密码与验证码不得持久化或进入日志 |
| BA05 | 登录账户 | 国家/地区电话区号 | `pending` | 对接 Web country list 并映射统一国家区号模型 |
| BA06 | 登录账户 | Web 短信验证码发送 | `pending` | 复用同一 captcha 事务和设备身份；手机号不持久化，发送与登录不得切换网络身份 |
| BA07 | 登录账户 | Web 短信验证码登录 | `pending` | 复用发送阶段产生的 captcha key，完整处理绑定、风控和登录成功 Cookie |
| BA08 | 登录账户 | 会话状态与账户资料 | `implemented` | `GET /v1/auth/session` 与 `/v1/account/profile` 固定调用 `x/web-interface/nav`，强类型映射登录态、UID、昵称、头像、验证状态、等级、认证、挂件、大会员、钱包及 WBI 实时口令等已知结构；登录 UID 必须与选中凭证一致，头像只接受 B 站 HTTPS 图片域名。`-101` 和 `isLogin=false` 作为未认证正常结果，不存在的精确别名不发网且不回退 `default`；调用方凭证与服务器多账户共用同一链路。匿名真实网络态及离线完整/畸形分支已通过，登录账户成功态待扫码联合验收 |
| BA09 | 登录账户 | Cookie 刷新与退出 | `pending` | 完成 refresh_csrf/correspondPath 刷新链及退出；只有新凭证完整有效才原子替换，退出删除精确账户 |
| BS01 | 搜索 | 视频直接搜索 | `pending` | Web 综合搜索中的 `video` 分支；完整保留页码、页大小、总数、排序、时长和分区筛选，不把专栏/直播混入视频结果 |
| BS02 | 搜索 | 搜索建议 | `pending` | Web suggestion；关键词与展示高亮分离，空建议返回空列表而非错误 |
| BS03 | 搜索 | 热门搜索 | `pending` | 热搜词、展示文本、排行与跳转元数据强类型映射；不跟随任意外部 URL |
| BP01 | 列表与 Uni | 当前账户创建的收藏夹目录 | `pending` | 完整列出默认与自建收藏夹，保留 `media_id/fid`、隐私、数量和所有者身份 |
| BP02 | 列表与 Uni | 当前账户收藏的收藏夹目录 | `pending` | 与创建目录分离并保留分页，不把“收藏他人列表”误作本地创建 |
| BP03 | 列表与 Uni | 用户公开合集/系列目录 | `pending` | `x/polymer/web-space/seasons_series_list`；Season 与 Series 分型，Basic 首先支持公开视频合集 |
| BP04 | 列表与 Uni | 公开合集详情 | `pending` | `bilibili:season:<season_id>`，同时保留所有者 `mid`、封面、简介、数量与公开状态 |
| BP05 | 列表与 Uni | 公开合集视频分页 | `pending` | `x/polymer/web-space/seasons_archives_list`；保留上游顺序、分页、AID/BVID 和各视频可播放身份 |
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
