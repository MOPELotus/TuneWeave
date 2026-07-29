# 参考实现与平台能力边界

TuneWeave 将参考项目作为协议资料，用于理解请求参数、默认值、分页、状态机、签名、加密、错误码和兼容分支。参考仓库只存在于 Git 忽略的本地目录，不作为子模块、构建依赖或运行时组件分发。

## 参考来源

| 平台 | 参考项目 | 参考修订 | 许可证 |
| --- | --- | --- | --- |
| 网易云 | `NeteaseCloudMusicApiEnhanced/api-enhanced` | `63d89aa906f78c286a7f838258fa29220d7f41dd` | MIT |
| 网易云音乐合伙人 | `MOPELotus/Lotus-ReFactor` | `004bbff438bc811f0f28a9ddf4181e8b77a510ba` | Lotus-ReFactor Source-Available Proprietary License；作者另行授权 TuneWeave 参考其逻辑与实现 |
| QQ 音乐 | `L-1124/QQMusicApi` | `873255f2774361ac97366bd89a14b8ed9d230aae` | GPL-3.0-or-later |
| 酷狗 | `MakcRe/KuGouMusicApi` | `283f1e97b110726b208a64b486a657c0fc0a6126` | MIT |
| 咪咕 | `Domdkw/miguMusic-api-enhanced` | `47d2edb7175cf2874882273ed14be0fdfe7db796` | Apache-2.0 |
| 酷我 | `qyhqiu/kuwoMusicApi` | `e8e720b90b4d7e3052078a3380906f2b3349e388` | Apache-2.0；README 与根许可证优先于过时的包元数据 |
| 酷我、汽水 | `guohuiyuan/music-lib` | `b299302e3163765d3efcc9df592700b41867c3d8` | AGPL-3.0；仅作协议研究，不复制、翻译或链接源码 |
| 酷我 | `UnblockNeteaseMusic/server` | `39e21bfb4b7581f39785b190aeced201d23f0d41` | LGPL-3.0-only；仅研究酷我移动播放、DES 与失败分支 |
| 酷我、汽水 | `CharlesPikachu/musicdl` | `e623653d1db0cd8f6eadb7326cea57e2b2e3d6ad` | PolyForm-Noncommercial-1.0.0；只研究官方候选链，排除第三方解析服务 |
| 酷我 | `listen1/listen1-api` | `aa4b9d34aad577a254a70b2754415adcbb17294d` | MIT；只作历史功能和数据模型基线 |
| 汽水 | `SaKongA/PopDownloader` | `8e48fd1d01b7d3d4262149863818ae15ee7e3bc9` | `package.json` 声明 ISC，但无根许可证文本；仅作研究快照 |
| 汽水 | `520Qiuyu/qishuiMusicAnalysis` | `b8f4e4f00be7c77ae6d12ca94d849c7f534cd3a9` | 未声明许可证；仅人工研究协议事实，不复用源码 |
| 汽水 | `baizeyv/SodaDownloader` | `893b49c35b7e11ada029e78782092f2553904281` | MIT |
| 汽水 | `naiyQAQ/qishui-decrypt` | `d360c20a697f9988c6b567c924af5b9784d18390` | MIT |
| B 站 | `MOPELotus/BBDown` | `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed` | MIT |
| B 站接口文档 | `bilibili-plugins/bilibili-api-collect` | `cfc5fddcc8a94b74d91970bb5b4eaeb349addc47` | CC BY-NC 4.0 |

TuneWeave 采用 `MIT OR Apache-2.0`。项目不复制、翻译、链接、打包或再分发参考项目的表达性源码，而是提取协议事实后按 Rust 强类型模型、统一 HTTP API、Provider 架构、多账户、调用方凭证、Uni Playlist 和跨平台回退重新设计。参考中的全局状态、无限重试、凭据泄漏、过时分支和不适合 TuneWeave 的控制流不会照搬。

许可证和授权说明见 [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md)。

## 项目范围与扩展候选

参考项目是协议事实来源，不是 TuneWeave 的功能分母。只有能直接增强媒体检索、整理、播放或必要账户体验的能力进入平台项目范围，并逐项得到以下结果之一：

1. 映射到稳定统一端点；
2. 因缺少合理跨平台语义而映射到 `/v1/extensions/{platform}`；
3. 经真实请求确认上游失效，并保留稳定错误和兼容说明。

范围内的低频或平台特有分支不能静默遗漏。范围外接口可以登记到平台功能扩展候选池，但候选项不构成实施承诺、完成率或平台欠账；只有出现明确需求时才选择实施。已知响应结构使用强类型模型；无法预知的扩展字段可以有界保留，但不能长期用裸 JSON 代替已知结构。

项目范围账本：

- [网易云音乐](coverage/netease-scope.md)
- [QQ 音乐](coverage/qq-scope.md)
- [B 站](coverage/bilibili-scope.md)
- [Uni Playlist](coverage/uni-playlist.md)
- [酷狗](coverage/kugou-scope.md)
- [咪咕](coverage/migu-scope.md)
- [酷我](coverage/kuwo-scope.md)
- [汽水音乐](coverage/soda-scope.md)

扩展候选池：

- [网易云](coverage/extensions/netease.md)
- [QQ 音乐](coverage/extensions/qq.md)
- [B 站](coverage/extensions/bilibili.md)

B 站只覆盖登录、账户、列表以及视频和音频直接相关能力，不包含专栏、直播、漫画、游戏等业务，也不再建立以全部 B 站业务为分母的全量账本。

## 平台协议摘要

### 网易云

- 参考项目按模块公开 API，并使用 `api`、`weapi`、`eapi`、`linuxapi`、`xeapi` 等协议。
- 搜索、详情、歌单、歌词、播放、云盘、播客、MV、账户和权益能力进入统一模型；平台特有能力进入扩展端点。
- 音乐合伙人逻辑来自作者明确授权参考的 Lotus-ReFactor，实现时仍遵循 TuneWeave 的多账户与安全边界。

### QQ 音乐

- 核心请求是 QQ Music CGI 的 `module + method + param` 单项或批量调用。
- 歌曲数字 ID、MID、媒体 MID、`songType`、文件规格、GUID、UIN 和 VKey 分别建模，不能压成一个无类型字符串。
- 登录包括 QQ、微信、QQ 音乐客户端二维码和手机验证码；凭证只进入选中的账户或调用方所有权范围。

### B 站

- 登录优先参考 BBDown 的真实 Cookie 与二维码链，其他账户字段和业务接口由 B 站接口文档补齐。
- BV、AV、EP、SS 等输入归一为稳定视频身份，再保留 AID、BVID、CID、分 P、封面、字幕和 DASH 音视频轨道。
- Playlist 同时支持公开视频合集/Season 与收藏夹，两种 ID 使用独立命名空间；视频项目可以解析仅音频流，但不虚构已经取消的独立音频投稿能力。

### 酷狗

- 公开搜索覆盖歌曲、歌单、歌词、专辑、歌手和 MV。
- 当前 Android 歌曲搜索即使刚取得匿名 `dfid` 仍会返回业务码 152；官方 `songsearch.kugou.com/song_search_v2` Web 搜索实测可匿名返回歌曲、音质哈希和权益字段，公开搜索继续采用该真实可用分支。匿名 GUID/MID 在启动时持久化，`dfid` 只在歌曲详情、歌单、歌词、权益和播放等真正进入移动协议的请求前注册，不把失效前置条件扩散到 Web 搜索。
- 播放链保留 `hash`、`album_audio_id`、设备身份、普通 token、VIP token、实际音质和试听状态。
- KRC/LRC 搜索与下载所需的 `id + accesskey` 作为强类型平台扩展保存。

### 咪咕

- 搜索、资源信息、可听性与多条播放 URL 链使用不同响应结构。歌曲搜索当前仍固定返回每页 20 项，参考项目暴露的 `size` 没有进入实际请求；TuneWeave 通过受限多页窗口实现统一 offset/limit，不把无效参数写进契约。
- 当前 HTTPS 搜索响应以 `contentId` 作为稳定播放身份，同时返回 `songId`、`copyrightId`、`resourceType`、歌词地址、替代版本和平台音频规格。`PQ/HQ/SQ/ZQ/ZQ24` 已有明确统一映射，详情中的 `ZQ` 与搜索和播放请求中的 `ZQ24` 属于同一 Hi-Res 家族；`AV3A/Z3D` 等尚未确认的沉浸格式只保留平台原码，不猜测音质名称。
- 当前 `resourceinfo.do` 可匿名按 `contentId` 返回详情、关联资源、旧/新两套音频规格、歌词资源、标签、统计与试听/VIP 标志；两套规格不是替代关系，`newRateFormats` 会省略仍存在于 `rateFormats` 的 LQ，因此必须分别保留并从并集计算统一音质。
- 参考项目的歌词模块只下载 `lrcUrl`，但真实资源详情还返回加密 `mrcUrl` 和可选 `trcUrl`。MRC 是 16 字符一组的十六进制密文，使用平台 64 位有符号分组算法解密后按 UTF-16LE 解释，正文同时包含行级与逐字毫秒时间。TuneWeave 将 LRC、MRC、TRC 作为独立通道；MRC 优先但不覆盖普通歌词，单通道失败也不会丢弃其他有效格式。
- 三条参考播放链并非等价备用源：v1 当前只返回权益、歌词和目录数据；匿名 v2 返回成功但没有 URL；H5 v2.4 才返回可用的加密公共媒体响应。H5 匿名请求会把 HQ、SQ、ZQ24 降为实际 PQ，必须保留请求与实际档位。真实免费歌曲由 `can-listen` 返回完整可听，会员歌曲返回不可完整播放但允许限时试听；下载不得复用后者的试听 URL。
- H5 媒体当前分布在同一 `freetyst.nf.migu.cn` 下仍并存的 `product8th/product` 和 `product9th/product` 两代目录。参考项目把 product8 标为可能不再使用，但实时响应证明它仍有效，因此白名单明确包含两代路径，不放宽为任意咪咕子域名或目录。
- 统一 resolver 已用真实跨平台歌曲确认咪咕精确匹配、实际音质和试听窗口不会在回退中丢失；调用方托管 Uni 项、完整媒体 302 与受限试听拒绝下载也已通过统一 HTTP 验收。该层复用 TuneWeave 通用解析契约，不另造咪咕特例或放宽匹配阈值。
- 公开歌单详情会回配 `resourceType=2021 + musicListId`，歌曲页使用另一套 M3 协议。真实响应证明传入 `pageSize=100` 仍只返回 50 首，不能相信参考项目把任意 `size` 直接透传后的表面语义；TuneWeave 固定物理页宽 50，再以受限连续页实现统一分页和完整 Uni 导入。
- 参考项目的公开模块直接使用普通 HTTP 客户端且不设响应上限。TuneWeave 将六个所需 API 固定为官方 HTTPS、关闭重定向并分别限制 API/歌词响应，同时独立校验资源与媒体链接；外部调用方不能覆盖传输参数。真实匿名差分仍是免费全曲与会员限时试听，不存在免登录伪造会员权益的分支。
- `47d2edb` 新增的 `ninan_signInfo` 与 H5/用户接口重构属于后续项目范围，未改变公开音源补充层的实施优先级。
- 关键资源身份包括 `contentId + copyrightId + resourceType`，登录播放还可能使用 PACM token。
- 外部歌单匹配只接受经过验证的公开来源，不提供任意 URL 转发。

### 酷我

- 酷我以当前官方网站和官方客户端的真实请求为实现基线。`music-lib` 是辅助资料中的最高优先级，用于核对新版歌词、逐字歌词、翻译、罗马音、专辑和歌单字段；所有端点、Cookie 和签名仍须按当前官方行为重新验证。
- `UnblockNeteaseMusic/server` 主要用于研究移动播放、`convert_url2`、DES 和失败回退；其历史搜索不能替代当前官网搜索。`musicdl` 只用于发现官方候选链，任何第三方托管解析 API 都不得进入 TuneWeave。
- `qyhqiu/kuwoMusicApi@e8e720b` 与 `listen1/listen1-api@aa4b9d3` 只保留为历史功能、旧接口类型和响应字段基线，不用于证明端点当前可用，也不迁入静态 Cookie、固定 Secret 或旧 H5 请求。
- 旧 `/api/www/search/searchMusicBykeyWord` 当前返回 403 或非法请求，当前官网改用匿名 HTTPS `/search/searchMusicBykeyWord`；该链已真实返回稳定歌曲、100 项物理页和总数，TuneWeave 不回退过时入口。
- 当前官网详情和歌单接口真实可用，但需要从当次匿名跟踪 Cookie 动态生成请求 `Secret`；参考项目内置的 2023 年静态 Cookie、固定 CSRF、完整 Cookie/响应日志、共享超时计数和递归重试均不可复用。
- 歌曲详情已按当前官网算法实现匿名会话和动态签名：Cookie 仅在 provider 实例内短期缓存，签名使用浏览器算法差分向量验证；成功响应同时复核字符串与数字两种歌曲身份，签名拒绝只允许刷新一次。
- 移动端 `songinfoandlrc` 当前仍能返回行级歌词。官网 `playUrl` 对免费样本签发完整 128 kbps MP3，对付费样本返回 `-1` 且不签发地址，对下线身份返回 `-1001`；更高 `br` 参数会被忽略而返回同一 128 kbps 文件，因此只声明已经验证的标准音质并保留真实权限边界。
- 旧 `newlyric.lrc` 二进制端点的 HTTPS 版本仍可返回含逐字时间的 LRCX；移动端行级歌词会间歇返回查询失败，因此实现以独立双来源读取和 LRCX 派生普通歌词收口，不把移动端失败误判为整首无歌词。
- 酷我已进入统一默认播放回退序列；resolver、Uni Client 模式与媒体跳转都复用同一严格匹配和 provider URL 校验，不额外维护平台专用解灰或重定向旁路。
- 当前 `playListInfo` 可匿名读取官方和用户公开歌单，页码从 1 开始且 `rn>100` 会被压到 100；100 首页偶发 504，因此实现只对瞬时错误做一次 250 ms 补发，不迁入参考项目的递归重试与共享超时状态。
- 公开音源层的全部入口已固定为官方 HTTPS 并禁用重定向，API、歌词传输和歌词解压分别执行独立上限；匿名 Cookie、动态 `Secret`、nonce 和媒体地址不进入日志或 Debug。全新目录的统一 HTTP 已验证免费全曲、付费拒绝、跨页歌单、Client Uni 与受限 1 KiB 媒体读取。

### 汽水音乐

- 汽水音乐尚无单一、完整且公认的协议项目，实施基线固定为官方分享页、官方 PC 客户端和官方 API 的真实请求。`PopDownloader` 与 `music-lib` 分别提供完整账户/媒体流程和结构化 `soda` 模型线索；`qishuiMusicAnalysis` 只用于人工核对 PC `track_v2`、设备字段和签名请求，因未声明许可证而禁止任何源码复用。
- `SodaDownloader` 用于研究 `aid/session`、分享链接、媒体规格和下载/解密串联；`qishui-decrypt` 用于研究 `spade_a`、MP4 `senc`、AES-CTR 及 FLAC/AAC/MP4 重组。`musicdl` 只用于发现官方 PC 搜索、分享页和歌词候选链，所含第三方解析回退必须删除。
- 官方 PC 歌曲搜索已真实消融到 `https://api.qishui.com/luna/pc/search/track`、查询词、游标和固定 `aid=386088`；20 首物理分页无需 Cookie、匿名设备、`x-helios` 或 `x-medusa`。实现不会照搬参考项目中的静态签名、共享设备或第三方搜索代理，也不会为公开搜索虚构持久设备前置条件。
- 官方数字 ID、`soda:` 引用和三类长分享 URL 已归一为同一歌曲身份；官方 `qishui.douyin.com/s/<code>` 短链真实返回一次 `music.douyin.com` 歌曲跳转。实现禁用客户端自动重定向，只接受固定短链路径并验证单次绝对 HTTPS `Location` 的主机、端口、路径和唯一歌曲 ID，不迁入参考项目的任意 URL 请求、页面正则扫描或无限跳转。
- Web `track_v2` 当前以固定 `track_id/media_type/aid/device_platform/channel` 即可匿名返回完整目录详情和歌词，并同时携带临时 player 数据；详情实现严格回配 ID 与媒体类型，只保留有界公开元数据，主动丢弃签名播放地址、player 模型和临时令牌。目录音质与会员标志不用于推导实时可播性，歌词和播放由后续独立能力重新请求并验证。
- `track_v2.lyric.content` 当前不是普通 LRC，而是以毫秒行头和相对字时间标签组成的 KRC 类格式；参考项目只删除标签生成普通歌词会丢失逐字能力。实现严格验证并原样保留逐字轨，再单向派生普通 LRC，未知第三字段仅随原始逐字内容保存且不猜测语义，缺失的翻译和音译不伪造。
- 分享页 `audioWithLyricsOption.lyrics.sentences` 对照样本比 `track_v2` 多两行页面前置信息；其后 26 行正文与 214 个字的文本及绝对时序逐项一致，最后一句页面级 `endMs` 使用 JavaScript 最大安全整数作展示哨兵，但末字真实结束时间仍与 `track_v2` 的 160000 ms 一致。实现以无哨兵的协议正文作为普通/逐字歌词，前置信息中的词作者进入 contributor，不把页面展示范围伪装成歌词时长。
- `x-helios`、`x-medusa`、设备身份和请求签名仍需在后续登录或鉴权请求确实要求它们时，逐链确认生成、有效期、轮换和重启恢复策略。搜索目录、媒体规格和 VIP 标志不能代替实时权益；匿名播放只返回实际获得的完整媒体或准确试听窗口。
- 官方合法响应若包含 CENC 加密媒体，可以在本地执行严格受限的解密和容器重组，但只能处理调用方已获授权的字节，不能以解密能力绕过会员或购买权限。MP4 Box、样本范围、密钥材料、输入/输出大小和失败类别必须独立验证。
- 公开专辑、歌单和分享集合应接入 Uni Playlist；登录、个人歌单、收藏、会员状态和登录权益播放留在后续完整项目范围。公开层的固定域名、重定向、响应上限、凭据脱敏、302 与真实媒体探测采用与既有 provider 相同的安全门槛。

## 公开音源补充范围

酷狗、咪咕、酷我和汽水音乐依次接入匿名搜索或公开发现、稳定身份、公开详情、歌词、播放/下载、Uni Playlist 添加/导入与跨平台回退。登录、账户和写操作不阻塞这一层。参考项目若提供真实有效且不绕过访问控制的免登录高品质或会员歌曲播放分支，也只能在官方链独立复现后纳入，并明确标记权益与试听边界。

## 能力矩阵

符号：`✓` 表示参考项目存在对应能力，`—` 表示不存在，`?` 表示接入前需要实时验证。

| 能力 | 网易云 | QQ | B 站 | 酷狗 | 咪咕 | 酷我 | 汽水 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 单曲/内容搜索 | ✓ | ✓ | ✓（视频） | ✓ | ✓ | ✓ | ? |
| 歌曲/视频详情 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 专辑与歌手 | ✓ | ✓ | — | ✓ | ✓ | ? | ? |
| 歌单读取 | ✓ | ✓ | ✓（合集/收藏夹） | ✓ | ✓ | ✓ | ? |
| 歌词/字幕 | ✓ | ✓ | ✓（字幕） | ✓ | ✓ | ✓ | ? |
| 音频流 | ✓ | ✓ | ✓（视频音轨） | ✓ | ✓ | ✓ | ? |
| 视频流/MV | ✓ | ✓ | ✓ | ✓ | ✓ | ? | ? |
| 二维码登录 | ✓ | ✓ | ✓ | ✓ | — | — | ? |
| 手机登录 | ✓ | ✓ | — | ✓ | ✓ | — | ? |
| 账户歌单 | ✓ | ✓ | ✓（收藏夹） | ✓ | ✓ | — | ? |
| 收藏/喜欢 | ✓ | ✓ | ✓（视频） | ✓ | ✓ | — | ? |

## 统一层约束

1. 平台 ID 按字符串保存，公开引用使用 `<platform>:<id>`。
2. 内容来源与播放来源分离，分别记录 `origin_track` 和 `resolved_track`。
3. `platform` 选择内容或账户平台，`playback_platform` 只选择播放来源。
4. 回退前使用标题、歌手、专辑、时长、ISRC 和版本标签严格匹配；Live、伴奏、翻唱、Remix 等差异必须降权或拒绝。
5. Provider 只声明已经实现的能力；未实现能力返回稳定的 `capability_not_supported`。
6. Cookie、token、VIP/PACM token、设备密钥和验证码不得进入普通日志或响应。
7. 媒体响应保留实际音质、必要请求头、有效期、试听区间和失败原因，不只返回裸 URL。
