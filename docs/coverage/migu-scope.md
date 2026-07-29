# 咪咕项目范围覆盖账本

协议基线为 `Domdkw/miguMusic-api-enhanced@47d2edb7175cf2874882273ed14be0fdfe7db796`，并以当前咪咕平台真实响应校正参考项目中的无效参数和过时传输方式。状态沿用其他平台账本：`pending` 尚未实现，`implemented` 已完成代码和离线验证但缺真实成功态，`verified` 已完成统一 HTTP 与真实上游验收。

当前公开音源补充层共 7 个验收单元：`pending=0`、`implemented=0`、`verified=7`，完成度 `7/7 = 100%`。登录、账户和写操作属于后续咪咕完整项目范围，不阻塞本层。

| ID | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- |
| MG01 | 公开歌曲搜索与稳定身份 | `verified` | `GET /v1/search?platform=migu&kind=track` 使用固定 HTTPS 歌曲搜索端点，以 `contentId` 返回稳定 `migu:<contentId>`，同时保留歌手、专辑、时长、MV、歌词资源、替代版本、版权标志与精确平台音频格式。平台物理页宽固定为 20，参考项目文档中的 `size` 未进入请求；统一 offset/limit 由最多 6 个连续页受限拼接。`PQ/HQ/SQ/ZQ/ZQ24` 映射已确认音质，`AV3A/Z3D` 等未知语义只保留原码。搜索权益不用于猜测 `playable`。真实 HTTPS 与统一 HTTP 已验证跨页非对齐窗口。 |
| MG02 | 歌曲详情与身份补全 | `verified` | `GET /v1/tracks/migu:<contentId>` 固定访问 HTTPS `resourceinfo.do`，要求单条响应的 `contentId` 与请求完全一致、`resourceType=2` 且 `copyrightId` 合法；返回别名、歌手、专辑、封面、时长、关联 MV、标签、统计、歌词资源、试听窗口、VIP/下载标志和平台关联资源。旧 `rateFormats` 与 `newRateFormats` 分别强类型保留，统一音质取二者并集，避免新列表遮掉只存在于旧列表的 LQ；`AV3A/Z3D` 等未知格式仍只保留原码。只有平台明确返回素材失效才设 `playable=false`，有效目录标志不冒充实时可播。真实 Provider 与统一 HTTP 已验证“告白气球”的严格身份、低码率至无损规格及权益诊断。 |
| MG03 | 普通与逐字歌词 | `verified` | `GET /v1/tracks/migu:<contentId>/lyrics` 复用严格歌曲详情，分别下载普通 LRC、加密 MRC 逐字歌词和可选 TRC 翻译。资源地址只接受固定咪咕 HTTPS 媒体域名及 `/data/oss/` 路径，三种格式并发但独立失败，单个响应限制为 4 MiB。MRC 按平台 64 位有符号分组算法解密并严格解码 UTF-16LE，保留原始逐字时间；有 MRC 时统一格式始终为 `mrc`，同时保留独立 LRC，LRC 缺失或失败时才从 MRC 派生行级歌词，不以低级格式覆盖高级格式。真实 MRC/LRC 和统一 HTTP 已验证。 |
| MG04 | 公开播放、下载与权益 | `verified` | `GET /v1/tracks/migu:<contentId>/availability` 固定调用 `can-listen/v1.0`，严格回配唯一 `contentId` 并分别返回完整可听和限时试听标志。播放与下载先刷新严格资源详情，再调用加密 H5 v2.4 链；参考项目的 v1 只返回权益数据，匿名 v2 当前成功但无 URL，均不冒充可用备用源。H5 的 `AB CD 01` 信封解密为强类型响应，`auto` 从目录最高规格起请求，`PQ/HQ/SQ/ZQ24` 分别承接标准、高品、无损和 Hi-Res 目标，平台实际降档必须通过 `actual_quality` 与原始 tone 如实返回。媒体只接受 `freetyst.nf.migu.cn` 的 HTTPS `product8th/product` 或 `product9th/product` 路径，且 `Tim/Key/playSessionId` 各恰好一次；不把 `Tim` 猜成到期时间。真实免费歌曲确认完整 PQ 流和可下载文件；真实会员歌曲确认 65–125 秒试听，下载保持 `available=false`、隐藏试听 URL，统一 HTTP 验收通过。 |
| MG05 | 统一播放回退与 302 | `verified` | 咪咕已进入默认 resolver 与显式 `playback_platform/source/fallback_platforms` 顺序，跨平台候选仍按标题、歌手、专辑、时长和版本严格评分，不因能取得 URL 放宽阈值。真实统一 HTTP 已将网易云歌曲以 `match_score=1.0` 解析为咪咕来源，并保留匿名链实际降为 PQ 的音质及 65–125 秒试听窗口；调用方托管 Uni 项可无持久化播放同一来源。完整流和下载的 `/redirect` 只返回 provider 已校验的可信 CDN、`private, no-store` 与 `no-referrer`；受限试听的下载跳转返回 403 且不携带 `Location`。默认顺序、显式咪咕优先、Uni 无状态播放及两类 302 均有服务端回归覆盖。 |
| MG06 | 公开歌单与 Uni 导入 | `verified` | `GET /v1/playlists/migu:<musicListId>` 与 `/tracks` 只接受规范正整数公开歌单身份，不接受账户。详情固定访问 `resource/playlist/v2.0` 并严格回配 `resourceType=2021 + musicListId`，保留创建者、标签、曲数、统计和经过安全过滤的沉浸展示；歌曲固定访问 `MIGUM3.0/resource/playlist/song/v2.0`。平台把过大的 `pageSize` 静默压为 50，因此 Provider 固定 50 首物理页并用最多 3 页实现任意统一 `offset` 和 `limit<=100`，复核跨页总数与发布时间，保持顺序、重复项和全局位置。真实统一 HTTP 已读取 195 首公开歌单、验证 `offset=49` 跨页窗口，将全部 195 项原子导入 Server Uni 后播放首项，并在 Client 模式完整展开来源后仅返回请求页且不持久化。 |
| MG07 | 协议、安全与真实权益验收 | `verified` | 六个上游 API 入口均为编译期固定的官方 HTTPS 标准端口，客户端禁用重定向、连接超时 10 秒、总超时 20 秒；公开调用不接收账户、Cookie、目标 URL、请求头、请求级代理、设备或签名覆盖，部署代理只能来自环境配置。API 与歌词响应分别限制 8 MiB 和 4 MiB，既检查声明长度也限制分块累计读取；429 单独映射可重试限流，5xx 可重试，4xx 不重试，业务码和身份漂移不伪装成功。歌词及展示资源、播放 CDN、路径和签名查询均有独立白名单。全部 8 条真实网络用例一次通过；全新数据目录的统一 HTTP 再确认免费全曲、会员试听、非法传输覆盖与非法歌单身份均正确，并从免费 CDN 实际读取 1 KiB 媒体内容而不下载整首或记录签名 URL。 |

后续完整项目范围将在公开音源层收口后扩展移动统一账号、短信登录、多业务身份、多账户、调用方凭证、账户资料与会员状态、个人歌单/喜欢、登录权益播放及仍直接增强媒体体验的目录能力；纯社交、直播互动、商城和营销活动不纳入。
