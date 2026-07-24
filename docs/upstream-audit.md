# 上游源码审计与平台能力边界

审计日期：2026-07-25

本文件记录 TuneWeave 首轮源码级审阅结论。参考仓库仅浅克隆到被 Git 忽略的 `.reference/repos/`，不会作为子模块或运行时依赖进入 TuneWeave。

## 固定快照

| 平台 | 参考项目 | 审阅提交 | 最近提交时间 | 许可证 |
| --- | --- | --- | --- | --- |
| 网易云 | `NeteaseCloudMusicApiEnhanced/api-enhanced` | `41bd6d82ce3b494d6375a784f5af391340ed9c1b` | 2026-07-19 | MIT |
| 网易云音乐合伙人 | `MOPELotus/Lotus-ReFactor` | `004bbff438bc811f0f28a9ddf4181e8b77a510ba` | 2026-07-22 | Lotus-ReFactor Source-Available Proprietary License |
| QQ 音乐 | `L-1124/QQMusicApi` | `261326eec051e7f444296b5c461e7412c4b25bb9` | 2026-07-25 | GPL-3.0-or-later |
| 酷狗 | `MakcRe/KuGouMusicApi` | `283f1e97b110726b208a64b486a657c0fc0a6126` | 2026-06-30 | MIT |
| 咪咕 | `Domdkw/miguMusic-api-enhanced` | `14c55ffbbbd1a90afe5e6ac45425f7b7988730bd` | 2026-07-21 | Apache-2.0 |
| 酷我 | `qyhqiu/kuwoMusicApi` | `e8e720b90b4d7e3052078a3380906f2b3349e388` | 2023-07-26 | Apache-2.0；README 未声明替代许可证，忽略误写为 ISC 的 `package.json` 元数据 |
| B 站 | `MOPELotus/BBDown` | `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed` | 2026-01-10 | MIT |
| B 站 API 文档 | `bilibili-plugins/bilibili-api-collect` | `cfc5fddcc8a94b74d91970bb5b4eaeb349addc47` | 2026-01-23 | CC BY-NC 4.0 |

TuneWeave 采用 `MIT OR Apache-2.0` 双许可。参考项目的许可证继续约束各项目自身源码；TuneWeave 不复制、翻译、链接、打包或再分发 QQMusicApi 等参考项目的源码，只独立实现经观察确认的请求与响应协议。`Lotus-ReFactor` 对外仍准确标注其 Source-Available Proprietary License；MOPELotus 声明其贡献了该项目的全部代码，并明确授权 TuneWeave 参考和复用其中的逻辑与实现。所有来源、固定提交和实际保留的第三方许可文本记录在 `THIRD_PARTY_NOTICES.md` 与 `licenses/`。

参考源码可以完整阅读并作为可执行协议说明，用于提取参数、默认值、分支、分页、状态机、签名、加密、错误码和字段优先级；但实现前必须审查其正确性、时效性与安全性。参考中的全局状态、无限重试、凭据泄漏、未执行边界、可疑对齐和不适合 Rust/多账户架构的控制流不会机械翻译，TuneWeave 以平台真实行为和自身强类型契约重新设计。已知结构不得长期停留在裸 JSON；已有实现也持续做差分审计、修正、单元测试和真实验证。

许可证元数据冲突时，以项目 README 中作者明确写出的许可证为准；README 没有替代声明时采用根许可证文件，并把 `package.json` 等生成/打包元数据的冲突记录为审计备注，不因此擅自阻断协议级独立实现。

## 2026-07-17 增量同步

8 个参考仓库均已 fetch 并在干净工作树上执行 fast-forward 检查：

- 网易云从 `6946dc8e14b6fb125191bc43525d4faa8123d8ae` 更新到 `321c25bd7d041711f1a9ab9e4b55997ce661313c`。公开模块由 404 增至 407，新增 `ad_get`、`ad_listening_rights_gain`、`register_checktoken`；请求层改为从网易易盾实时取得 anti-cheat token，而不是固定配置 token。这三项属于播放权益和底层协议 Basic，已加入覆盖账本。
- 咪咕从 `45cda48aeee995121ff7987a81e52949732a917c` 更新到 `07303dfaa1ebcfe4f24a291de6a536e3403d6043`。新增 `recommend_radio_all`，覆盖私人 FM、时序/分类电台、乐游播客和 YOU 乐电台；同时调整登录、资源详情、播放 URL、PACM token 与请求工具，咪咕 Basic 阶段必须按新快照复核这些链路。
- 网易云音乐合伙人、QQ、酷狗、酷我、BBDown 和 B 站 API 文档快照均无新提交。

## 2026-07-18 增量同步

8 个参考仓库再次 fetch；有更新的网易云、QQ 和咪咕参考工作树均保持干净并执行 fast-forward，其余 5 个仓库无新提交：

- 网易云从 `321c25bd7d041711f1a9ab9e4b55997ce661313c` 更新到 `35d1c61cb4dccd1c55c25bf791a915cd29f7fedf`。公开模块由 407 增至 416：原 `register_checktoken` 明确更名为 v3，同时新增 v2 令牌注册、6 个云小编活动/账户模块和 2 个云小编审核模块；评论切换为 EAPI + v2 checkToken，歌单收藏也强制 v2，广告目录和广告换听明确使用 v3。覆盖账本已加入全部新增模块，并把尚未迁移新协议的评论能力退回 `partial`。
- QQ 从 `b859d8e01566b92c27e78dd400f4f8c6950685f2` 更新到 `9b48d99efbf96ef86a88e579415f183f6db111f0`。变更仅修正 `niquests` 内置 urllib3 的 `Retry` 导入路径，没有新增、删除或修改公开音乐能力；QQ Basic 开始时仍按新快照实现和验证。
- 咪咕从 `07303dfaa1ebcfe4f24a291de6a536e3403d6043` 更新到 `ae5581a1e82f481aaaa16b7e78a3e443da036c45`。`src/modules` 由 56 增至 66，新增歌词、短视频/视频彩铃详情/搜索建议/播放/用户内容、收藏歌单增删和视频彩铃计数/播放地址；同时把视频搜索、收藏列表模块更名并扩展资源与路由。咪咕阶段必须以 66 项新模块基线生成逐项账本。

## 2026-07-22 增量同步

切换到 QQ 音乐 Basic 前再次 fetch 全部 8 个参考仓库。网易云、Lotus-ReFactor 和咪咕工作树干净并完成 fast-forward；QQ 上游对 `main` 做了非快进强制更新，因此保留旧 `9b48d99` 工作树不做 reset，审阅分叉后另建干净的只读快照 `qqmusic-current@1b0aae0`。其余 4 个仓库无新提交：

- 网易云从 `35d1c61` 更新到 `41bd6d8`，公开模块仍为 416。`song_url_v1` 新增仅在 `level=sky` 生效的 `immerseType=c51|ste|aac`，缺省 `c51`；同步后已补统一输入、三分支协议测试及单曲/批量/跨平台解析链透传，并以公开歌曲逐项真实取流成功，不再把固定 `c51` 当作完整输入。
- Lotus-ReFactor 从 `646400c` 更新到 `004bbff`，变化是验证码通知批处理和签到进度展示，不改变音乐合伙人请求协议或 TuneWeave 端点清单。
- QQ 旧、新分支共同祖先为 `b859d8e`。新远端用 `RetryConfiguration` 修复同一 niquests 重试兼容点，并给歌词查询新增 `song_type` 及 Web 参数；18 个模块文件、许可证和其余公开能力均未变化。QQ Basic 以 `1b0aae0` 为新基线，歌词不能把歌曲类型固定为 1。
- 咪咕从 `ae5581a` 更新到 `14c55ff`（`v2.7.0`），`src/modules` 由 66 增至 71：新增喜欢/取消喜欢、自建歌单创建/改名/删除，修改 v2/H5 播放 URL 的固定资源类型和请求头，并加入 URL SQLite 缓存及 `/db` 查询服务。咪咕账本届时须同时登记 71 个模块和服务级缓存端点，不把缓存服务误算成音乐实体能力。

## 2026-07-25 增量同步

恢复 QQ Basic 开发时按 6–12 小时规则检查上游，`QQMusicApi` 从 `1b0aae0` 更新到 `261326e`（v0.7.0 后续修复），公开方法由 100 增至 104：

- `SongApi.get_song_urls` 新增 `R500/R400/R200` 三种彩铃规格；TuneWeave 已把稳定规格索引扩展到 `0..47`，保持普通、加密、特殊和彩铃四个家族，参考声明但未实际执行的 100 项上限由 TuneWeave 明确校验。
- `SearchApi.search_by_type` 新增 `RINGTONE=10`、请求 `selectors + vec_selectors` 和二维 selectors 响应；旧九类实现退回 `partial`。
- `SongApi.query_song` 改为逐项 `id|mid + song_type`。参考代码允许混合 ID/MID，却把标识拆成两个数组而保留一条原始 `types` 顺序，存在类型对齐疑点；TuneWeave 先登记缺口，下一块用真实差分请求确认协议后再设计强类型输入。
- `LyricApi.get_lyric` 新增助唱标注分支，同时新增助唱存在性、多风格翻译、AI 词典存在性和详情 4 个公开方法；QQ Basic 分母由 73 增至 77，全量分母由 100 增至 104。
- Web 层引入可选认证策略和 adapter 注册表，并修复凭据从错误参数位置读取的问题。TuneWeave 已有的显式 `(qq, account)` 私有凭据注入不沿用 Web 全局上下文，继续保持账户隔离。

同日重新审查统一音源链：QQ 文件、版本 MID、`song_type`、现代音质数组和试听窗口已先进入内部强类型元数据；192k 与空间/环绕层级错误已修正。2026-07-22 的真实统一播放、无损下载、302 和网易云→QQ 严格匹配成功证据仍有效；2026-07-25 新匿名设备对四个规格族的复验被上游 `code=1000` 拒绝，稳定映射为认证前置错误，未伪造新增彩铃的成功态。

后续按 [实施顺序与持续上游同步](implementation-plan.md) 执行：每完成 3 个上游 API 模块检查活跃参考仓库；阶段切换和发布前检查全部参考仓库，并同步固定 SHA、模块数与覆盖状态。

### 定期检查记录

| 日期 | 触发点 | 仓库 | 检查前/远端提交 | 结果 |
| --- | --- | --- | --- | --- |
| 2026-07-17 | 完成国家区号、旧版全类型搜索、默认搜索词 | `api-enhanced` | `321c25bd7d041711f1a9ab9e4b55997ce661313c` | 无更新，仍为 407 个公开模块 |
| 2026-07-17 | 完成简略/详细热搜及 Web、移动端、PC 搜索建议 | `api-enhanced` | `321c25bd7d041711f1a9ab9e4b55997ce661313c` | 无更新，仍为 407 个公开模块 |
| 2026-07-17 | 完成多重搜索、本地歌曲匹配、公开会员摘要 | `api-enhanced` | `321c25bd7d041711f1a9ab9e4b55997ce661313c` | 无更新，仍为 407 个公开模块 |
| 2026-07-17 | 完成播客详情、节目列表、节目详情 | `api-enhanced` | `321c25bd7d041711f1a9ab9e4b55997ce661313c` | 无更新，仍为 407 个公开模块 |
| 2026-07-18 | 完成客户端会员、v3 checkToken、广告权益目录 | 全部 8 个参考仓库 | 网易云 `321c25b→35d1c61`；QQ `b859d8e→9b48d99`；咪咕 `07303df→ae5581a` | 网易云 407→416；QQ 无公开 API 变化；咪咕模块 56→66；其余无更新 |
| 2026-07-22 | Uni Playlist 全部 11 个能力收口并切换 QQ Basic | 全部 8 个参考仓库 | 网易云 `35d1c61→41bd6d8`；Lotus `646400c→004bbff`；QQ 远端强制改写 `9b48d99↔1b0aae0`；咪咕 `ae5581a→14c55ff` | 网易云新增 `sky` 沉浸声类型；QQ 新增歌词 `song_type` 且保留旧快照；咪咕模块 66→71；其余无更新 |
| 2026-07-22 | 用户要求在 QQ Basic 开发前再次检查 | 全部 8 个参考仓库 | 网易云 `41bd6d8`；Lotus `004bbff`；QQ `1b0aae0`；酷狗 `283f1e9`；咪咕 `14c55ff`；酷我 `e8e720b`；BBDown `259a555`；B 站文档 `cfc5fdd` | 全部无更新；QQ 旧审计副本仍为 1 ahead/2 behind，最新只读快照精确对齐 `origin/main@1b0aae0` |
| 2026-07-25 | 恢复 QQ 统一播放开发并执行 6–12 小时检查 | `QQMusicApi` | `1b0aae0→261326e` | 100→104 个公开方法；新增彩铃搜索/文件、selectors、逐项歌曲类型、助唱标注及 4 个歌词方法；覆盖账本和状态已重算 |

## 完整覆盖验收基线

上述参考项目不是只用于挑选常用能力，而是 TuneWeave 最终的平台功能覆盖基线。每个参考快照中对调用者公开的 API 都必须逐项登记并得到以下结论之一：

1. 映射到稳定的统一端点；
2. 因缺少合理的跨平台语义而映射到 `/v1/extensions/{platform}`；
3. 经真实请求确认上游已经失效，并保留验证证据、稳定错误和兼容说明。

不能因为功能不常用、只对单个平台有意义或难以归一化而静默遗漏。只有端点清单全部完成“实现 + 测试 + 真实可用度验证”，对应平台才能标记为完成。后续审计将以固定提交生成逐端点覆盖清单，源码更新时只增量复核差异。B 站阶段同时以 BBDown 的公开能力和 `bilibili-api-collect` 固定快照中的公开端点为覆盖基线；后者是协议文档来源，不作为源码依赖或可复制实现。

逐项账本：

- [网易云 416 个公开模块](coverage/netease.md)
- [QQ 音乐 104 个公开方法](coverage/qq.md)

登录也遵循完整覆盖原则。以网易云为例，二维码、邮箱账号密码、手机号密码、手机号验证码、登录状态、刷新和退出都属于账户能力，不以二维码登录代替其余流程。密码、验证码、Cookie 和 token 只能进入请求处理与服务端账户仓库，不写入日志或普通响应。

## 源码结构与可复用逻辑

### 网易云

- `module/*.js` 的文件名默认转换成 HTTP 路由，例如 `playlist_detail.js` 对应 `/playlist/detail`；`daily_signin`、`fm_trash`、`personal_fm` 是保留下划线的特殊路由。
- 请求层统一处理 `api`、`weapi`、`eapi`、`linuxapi`、`xeapi`。TuneWeave 已独立实现五种协议，并于 2026-07-16 以真实搜索请求逐项验证；XEAPI 同时完成动态公钥注册、响应签名校验、X25519/AES-GCM 会话协商及加密响应解包。
- `search` 支持单曲、专辑、歌手、歌单、用户、MV、歌词、电台和视频；`song/detail`、`playlist/detail`、`playlist/track/all`、`user/playlist` 能组成统一目录模型。
- `song/url/v1` 已包含解灰入口，但 TuneWeave 不沿用它的单平台响应形态；跨平台匹配与回退放到核心解析器中。
- `Lotus-ReFactor/services/neteasePartner/service.js` 补足音乐合伙人任务读取、待评作品、分项评分和提交逻辑。该能力没有跨平台等价物，应位于网易云扩展端点而不是伪装成通用端点。

### QQ 音乐

- 当前项目是异步 Python SDK，不是 HTTP 服务；核心调用形态是 QQ Music CGI 的 `module + method + param` 批量请求。
- 目录能力覆盖综合/分类搜索、歌曲详情、专辑、歌手、榜单、歌单详情、用户创建/收藏歌单、歌词和 MV。
- 播放地址由歌曲 MID、媒体 MID、文件规格前后缀、GUID、账号 UIN 和 VKey 共同生成。返回的 `purl` 需要与 CDN 域名拼接。
- 登录支持 QQ、微信、移动端二维码和手机验证码；账号凭证必须由账户仓库管理，不能作为普通响应字段回传。
- 歌单写操作支持创建、删除、添加/删除歌曲和“我喜欢”。歌曲数字 ID、MID、媒体 MID 与 `songType` 必须分别保存，不能压成一个无类型字符串。

### 酷狗

- 与网易云项目相似，`module/*.js` 默认按下划线生成路由。
- 搜索覆盖歌曲、歌单、歌词、专辑、歌手和 MV；歌曲 URL 同时依赖 `hash`、`album_audio_id`、设备 MID/GUID、用户 ID、普通 token 与 VIP token。
- 二维码状态明确区分过期、待扫码、待确认和成功；成功后产生 `token + userid`。
- 歌词通常需要先搜索得到 `id + accesskey`，再下载 KRC/LRC；这要求统一模型允许保存平台扩展字段。

### 咪咕

- Hono 路由已按搜索、专辑、歌单、榜单、推荐、歌手、URL、认证、资源、MV 和用户分组。
- 播放地址至少有 v1、v2、H5 v2.4 三条链路，关键标识是 `contentId + copyrightId + resourceType`；登录播放还使用 `pacmtoken`。
- 登录覆盖 SIM 一键认证和手机号验证码，账户接口覆盖资料、今日推荐、收藏和自建歌单。
- 搜索、资源信息和可听性检测的响应形状不同，必须在适配器内规范化，不能把上游 JSON 原样透传。

### B 站

- BBDown 先把 BV/AV/EP/SS 等输入解析成 AID，再通过视频信息接口获得标题、简介、封面、UP 主、发布时间与分 P 的 CID。
- 播放信息使用 DASH 音视频轨道；音频轨道包含 ID、码率、编码、主 URL 和备用 URL。调用者通常还需要 `Referer`/Cookie 请求头。
- `bilibili-api-collect` 补齐 BBDown 未覆盖的站点业务接口，包括账户、用户空间、公开合集/系列、收藏夹及其写操作。TuneWeave 会对该固定快照逐端点登记：能统一的进入稳定端点，其余进入 `/v1/extensions/bilibili`。
- B 站 `Playlist` 至少统一两类资源：公开的视频合集/Season（`season_id + mid`）和个人收藏夹（`media_id/fid + mid`）。前者使用 `bilibili:season:<season_id>`，后者使用 `bilibili:favorite:<media_id>`，不得把两种可能重号的 ID 压成同一命名空间。
- 公开合集内容使用 `x/polymer/web-space/seasons_archives_list`，收藏夹内容使用 `x/v3/fav/resource/list`；私有收藏夹通过 `account` 选择登录态。合集/收藏夹内的视频在统一音乐目录中规范化为可播放的 `Track`，并在扩展字段保留原始 `Video` 引用、AID/BVID、CID 和资源类型。
- 需求样例分别为公开合集 `https://space.bilibili.com/327961371/lists/3629748?type=season` 与个人收藏夹 `https://space.bilibili.com/47275982/favlist?fid=2883236382&ftype=create`；实现和测试必须覆盖这两种 URL/引用解析。
- TuneWeave 把 B 站内容建模为视频资源，并允许从视频资源解析仅音频流；不虚构已经取消的独立音频投稿能力。
- BBDown 服务端的任务 API 面向下载任务，不适合作为 TuneWeave 的外部协议。TuneWeave 直接实现信息与媒体解析，并把下载留给客户端。
- 本地既有运行经验表明，使用 BBDown 产物时必须排除数字分片并优先选择标题型成品文件；TuneWeave 直接返回轨道 URL，可避开这类产物选择歧义。
- B 站接口默认按可用能力推进并逐项做真实请求验证，不因参考仓库活跃度做整体降级判断。只有已经实测失败的端点才标记不可用；已知少数分区类接口若在接入时仍失效，单独记录证据和稳定错误，不影响合集、收藏夹等其余能力的实现。

### 酷我

- 现有代码只覆盖基础搜索、歌曲/歌单/专辑/歌手/榜单、歌词、评论、MV 和播放 URL，没有完整账户体系。
- 运行栈仍声明 Node.js 8 与早期 TypeScript，最后提交停在 2023-07。
- 在第六阶段接入前必须逐条做真实网络验证：至少验证搜索、歌曲详情、歌单、歌词和播放 URL；失败接口不进入能力声明。

## 统一能力矩阵

符号：`✓` 表示已从上游源码确认，`—` 表示上游没有对应能力，`?` 表示接入前需要实时验证。

| 能力 | 网易云 | QQ | B 站 | 酷狗 | 咪咕 | 酷我 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 单曲/内容搜索 | ✓ | ✓ | ✓（视频） | ✓ | ✓ | ? |
| 歌曲/视频详情 | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 专辑与歌手 | ✓ | ✓ | — | ✓ | ✓ | ? |
| 歌单读取 | ✓ | ✓ | ✓（公开合集/收藏夹） | ✓ | ✓ | ? |
| 歌词 | ✓ | ✓ | — | ✓ | ✓（搜索） | ? |
| 音频流 | ✓ | ✓ | ✓（视频音轨） | ✓ | ✓ | ? |
| 视频流/MV | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 二维码登录 | ✓ | ✓ | ✓（Cookie/扫码链） | ✓ | — | — |
| 手机登录 | ✓ | ✓ | — | ✓ | ✓ | — |
| 账户歌单 | ✓ | ✓ | ✓（创建/收藏的收藏夹） | ✓ | ✓ | — |
| 歌单写操作 | ✓ | ✓ | ✓（系列/收藏夹） | ✓ | ? | — |
| 收藏/喜欢 | ✓ | ✓ | ✓（收藏视频） | ✓ | ✓ | — |
| 每日推荐 | ✓ | ✓ | — | ✓ | ✓ | — |
| 音乐合伙人 | ✓（扩展） | — | — | — | — | — |

## 对统一层的硬性约束

1. 所有平台 ID 以字符串保存，公开引用使用 `<platform>:<id>`；不得假设 ID 是整数。
2. 原目录歌曲与最终播放歌曲分别记录为 `origin_track` 和 `resolved_track`。
3. `platform` 选择目录或账户平台；`playback_platform` 只影响播放来源，二者不能复用同一含义。
4. 回退前必须用标题、歌手、专辑、时长、ISRC（若有）做匹配并给出分数；Live、伴奏、翻唱、Remix 等版本差异必须降权或拒绝。
5. 每个适配器声明能力，未实现的能力返回稳定的 `capability_not_supported`，不把上游 404 或任意 JSON 冒充成统一实体；只有明确登记的 `/v1/extensions/{platform}` 通用协议端点可以在统一包络内返回原始上游 JSON。
6. Cookie、token、VIP token、PACM token 等只存在服务端账户仓库；日志和 API 响应默认脱敏。
7. 媒体响应保留必要请求头、链接过期时间、试听区间和实际音质，不能只返回裸 URL。
