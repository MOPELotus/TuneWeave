# 上游源码审计与平台能力边界

审计日期：2026-07-14

本文件记录 TuneWeave 首轮源码级审阅结论。参考仓库仅浅克隆到被 Git 忽略的 `.reference/repos/`，不会作为子模块或运行时依赖进入 TuneWeave。

## 固定快照

| 平台 | 参考项目 | 审阅提交 | 最近提交时间 | 许可证 |
| --- | --- | --- | --- | --- |
| 网易云 | `NeteaseCloudMusicApiEnhanced/api-enhanced` | `6946dc8e14b6fb125191bc43525d4faa8123d8ae` | 2026-07-12 | MIT |
| 网易云音乐合伙人 | `MOPELotus/Lotus-ReFactor` | `646400c1cf098c3887ef90886617625169fb58de` | 2026-07-13 | Lotus-ReFactor Source-Available Proprietary License |
| QQ 音乐 | `L-1124/QQMusicApi` | `b859d8e01566b92c27e78dd400f4f8c6950685f2` | 2026-07-12 | GPL-3.0-or-later |
| 酷狗 | `MakcRe/KuGouMusicApi` | `283f1e97b110726b208a64b486a657c0fc0a6126` | 2026-06-30 | MIT |
| 咪咕 | `Domdkw/miguMusic-api-enhanced` | `45cda48aeee995121ff7987a81e52949732a917c` | 2026-07-13 | Apache-2.0 |
| 酷我 | `qyhqiu/kuwoMusicApi` | `e8e720b90b4d7e3052078a3380906f2b3349e388` | 2023-07-26 | `LICENSE` 为 Apache-2.0，`package.json` 标为 ISC，需在接入前澄清 |
| B 站 | `MOPELotus/BBDown` | `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed` | 2026-01-10 | MIT |

TuneWeave 采用 `MIT OR Apache-2.0` 双许可。参考项目的许可证继续约束各项目自身源码；TuneWeave 不复制、翻译、链接、打包或再分发 QQMusicApi 等参考项目的源码，只独立实现经观察确认的请求与响应协议。`Lotus-ReFactor` 对外仍准确标注其 Source-Available Proprietary License；MOPELotus 声明其贡献了该项目的全部代码，并明确授权 TuneWeave 参考和复用其中的逻辑与实现。所有来源、固定提交和实际保留的第三方许可文本记录在 `THIRD_PARTY_NOTICES.md` 与 `licenses/`。

## 完整覆盖验收基线

上述参考项目不是只用于挑选常用能力，而是 TuneWeave 最终的平台功能覆盖基线。每个参考快照中对调用者公开的 API 都必须逐项登记并得到以下结论之一：

1. 映射到稳定的统一端点；
2. 因缺少合理的跨平台语义而映射到 `/v1/extensions/{platform}`；
3. 经真实请求确认上游已经失效，并保留验证证据、稳定错误和兼容说明。

不能因为功能不常用、只对单个平台有意义或难以归一化而静默遗漏。只有端点清单全部完成“实现 + 测试 + 真实可用度验证”，对应平台才能标记为完成。后续审计将以固定提交生成逐端点覆盖清单，源码更新时只增量复核差异。

逐项账本：

- [网易云 404 个公开模块](coverage/netease.md)

登录也遵循完整覆盖原则。以网易云为例，二维码、邮箱账号密码、手机号密码、手机号验证码、登录状态、刷新和退出都属于账户能力，不以二维码登录代替其余流程。密码、验证码、Cookie 和 token 只能进入请求处理与服务端账户仓库，不写入日志或普通响应。

## 源码结构与可复用逻辑

### 网易云

- `module/*.js` 的文件名默认转换成 HTTP 路由，例如 `playlist_detail.js` 对应 `/playlist/detail`；`daily_signin`、`fm_trash`、`personal_fm` 是保留下划线的特殊路由。
- 请求层统一处理 `api`、`weapi`、`eapi`、`xeapi`。首批能力可以用 `eapi`/`weapi` 覆盖搜索、歌曲详情、歌单、歌词、旧版播放 URL 与二维码登录；新版高音质 URL 后续再接 `xeapi`。
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
- TuneWeave 把 B 站内容建模为视频资源，并允许从视频资源解析仅音频流；不虚构已经取消的独立音频投稿能力。
- BBDown 服务端的任务 API 面向下载任务，不适合作为 TuneWeave 的外部协议。TuneWeave 直接实现信息与媒体解析，并把下载留给客户端。
- 本地既有运行经验表明，使用 BBDown 产物时必须排除数字分片并优先选择标题型成品文件；TuneWeave 直接返回轨道 URL，可避开这类产物选择歧义。

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
| 歌单读取 | ✓ | ✓ | ✓（收藏夹/合集） | ✓ | ✓ | ? |
| 歌词 | ✓ | ✓ | — | ✓ | ✓（搜索） | ? |
| 音频流 | ✓ | ✓ | ✓（视频音轨） | ✓ | ✓ | ? |
| 视频流/MV | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 二维码登录 | ✓ | ✓ | ✓（Cookie/扫码链） | ✓ | — | — |
| 手机登录 | ✓ | ✓ | — | ✓ | ✓ | — |
| 账户歌单 | ✓ | ✓ | ✓（收藏夹） | ✓ | ✓ | — |
| 歌单写操作 | ✓ | ✓ | — | ✓ | ? | — |
| 收藏/喜欢 | ✓ | ✓ | ✓（收藏视频） | ✓ | ✓ | — |
| 每日推荐 | ✓ | ✓ | — | ✓ | ✓ | — |
| 音乐合伙人 | ✓（扩展） | — | — | — | — | — |

## 对统一层的硬性约束

1. 所有平台 ID 以字符串保存，公开引用使用 `<platform>:<id>`；不得假设 ID 是整数。
2. 原目录歌曲与最终播放歌曲分别记录为 `origin_track` 和 `resolved_track`。
3. `platform` 选择目录或账户平台；`playback_platform` 只影响播放来源，二者不能复用同一含义。
4. 回退前必须用标题、歌手、专辑、时长、ISRC（若有）做匹配并给出分数；Live、伴奏、翻唱、Remix 等版本差异必须降权或拒绝。
5. 每个适配器声明能力，未实现的能力返回稳定的 `capability_not_supported`，不返回上游 404 或任意 JSON。
6. Cookie、token、VIP token、PACM token 等只存在服务端账户仓库；日志和 API 响应默认脱敏。
7. 媒体响应保留必要请求头、链接过期时间、试听区间和实际音质，不能只返回裸 URL。
