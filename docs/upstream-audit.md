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
- 当前 HTTPS 搜索响应以 `contentId` 作为稳定播放身份，同时返回 `songId`、`copyrightId`、`resourceType`、歌词地址、替代版本和平台音频规格。`PQ/HQ/SQ/ZQ24` 已有明确统一映射；`AV3A/Z3D` 等尚未确认的沉浸格式只保留平台原码，不猜测音质名称。
- `47d2edb` 新增的 `ninan_signInfo` 与 H5/用户接口重构属于后续项目范围，未改变公开音源补充层的实施优先级。
- 关键资源身份包括 `contentId + copyrightId + resourceType`，登录播放还可能使用 PACM token。
- 外部歌单匹配只接受经过验证的公开来源，不提供任意 URL 转发。

### 酷我

- 参考项目覆盖基础搜索、歌曲/歌单/专辑/歌手/榜单、歌词、评论、MV 和播放 URL，但缺少完整账户体系。
- 接入前逐项验证搜索、详情、歌单、歌词和播放 URL；只有真实可用的端点才声明能力。

## 公开音源补充范围

酷狗、咪咕和酷我先接入匿名搜索、公开详情、歌词、播放/下载、Uni Playlist 添加/导入与跨平台回退。登录、账户和写操作不阻塞这一层。参考项目若提供真实有效且不绕过访问控制的免登录高品质或会员歌曲播放分支，也纳入并明确标记权益与试听边界。

## 能力矩阵

符号：`✓` 表示参考项目存在对应能力，`—` 表示不存在，`?` 表示接入前需要实时验证。

| 能力 | 网易云 | QQ | B 站 | 酷狗 | 咪咕 | 酷我 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 单曲/内容搜索 | ✓ | ✓ | ✓（视频） | ✓ | ✓ | ? |
| 歌曲/视频详情 | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 专辑与歌手 | ✓ | ✓ | — | ✓ | ✓ | ? |
| 歌单读取 | ✓ | ✓ | ✓（合集/收藏夹） | ✓ | ✓ | ? |
| 歌词/字幕 | ✓ | ✓ | ✓（字幕） | ✓ | ✓ | ? |
| 音频流 | ✓ | ✓ | ✓（视频音轨） | ✓ | ✓ | ? |
| 视频流/MV | ✓ | ✓ | ✓ | ✓ | ✓ | ? |
| 二维码登录 | ✓ | ✓ | ✓ | ✓ | — | — |
| 手机登录 | ✓ | ✓ | — | ✓ | ✓ | — |
| 账户歌单 | ✓ | ✓ | ✓（收藏夹） | ✓ | ✓ | — |
| 收藏/喜欢 | ✓ | ✓ | ✓（视频） | ✓ | ✓ | — |

## 统一层约束

1. 平台 ID 按字符串保存，公开引用使用 `<platform>:<id>`。
2. 内容来源与播放来源分离，分别记录 `origin_track` 和 `resolved_track`。
3. `platform` 选择内容或账户平台，`playback_platform` 只选择播放来源。
4. 回退前使用标题、歌手、专辑、时长、ISRC 和版本标签严格匹配；Live、伴奏、翻唱、Remix 等差异必须降权或拒绝。
5. Provider 只声明已经实现的能力；未实现能力返回稳定的 `capability_not_supported`。
6. Cookie、token、VIP/PACM token、设备密钥和验证码不得进入普通日志或响应。
7. 媒体响应保留实际音质、必要请求头、有效期、试听区间和失败原因，不只返回裸 URL。
