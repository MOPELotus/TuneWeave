# 上游更新审计（2026-09-07）

## 范围与结论

- TuneWeave 基线：`dab254e7578cc5f85ced7befa124a73d6c48c31f`，提交时间 **2026-08-06 08:51:08 +08:00**，版本 `0.1.0-alpha.8`。
- 按用户要求，只审计此时间之后进入参考上游默认分支的变化；不把此前未接入的功能计为本轮新增。
- 2026-09-07 联网 fetch 了第三方声明中的全部 16 个不同参考仓库。QQ 的两份本地 checkout 视为同一个来源。没有切换参考工作树，审查使用 `origin/HEAD` 的 Git 对象。
- 各仓库取默认分支第一父链上截止基线时间的最近提交，与本次 fetch 得到的 HEAD 比较。此方法包含之后合并、但作者时间更早的 PR；不是简单按作者日期过滤。
- 16 个仓库中 8 个有变化。最紧急的是汽水现有搜索、详情链实测失败；其次是网易云 NMTID、易盾 V2 和新增音质。酷狗与咪咕主要是新能力接入。
- 本轮仅审计并记录；未修改业务代码，未提交或推送，也未使用现有账户凭据或执行平台写操作。此报告不等于各新协议已完成实现验收。

## 快照清单

“新增提交”是两个快照之间的可达提交数，包含合并提交和其分支历史，不等于新增功能数量。

| 参考仓库 | 时间基线快照 | 审计快照 | 新增提交 | 处理结论 |
| --- | --- | --- | ---: | --- |
| NeteaseCloudMusicApiEnhanced/api-enhanced | `5859944563803b4eacf2cfd76129e879ccb6b057` | `d55d92cd0031d7c7746b7068faecd7ade1d354ac` | 32 | 会话、易盾、音质及歌曲接口变化 |
| MakcRe/KuGouMusicApi | `06560e3e053bda1ab830750db6f645bab703f824` | `b8637b73fc54c786186c19d7512815ac5af542e8` | 24 | 新登录、授权播放、媒体库能力 |
| Domdkw/miguMusic-api-enhanced | `bef52654d72d36ee9cb57c01608fef4063f6e6d0` | `d6f281326bfd3bb2d968a46773f7591693f03d30` | 61 | 新播放链、大量目录/账户能力、许可证变化 |
| guohuiyuan/music-lib | `3c4c32fd2aff2ffb9737856e52a45fd46b7ca12d` | `3b22e851f4fa2f55ceab943fa846a71536fed4f9` | 8 | 汽水协议迁移；酷我分页；Go 依赖更新 |
| CharlesPikachu/musicdl | `7d6f8fca6e59c89beb7b467f32aef5d677334b64` | `6edc24b0da029bdc12e15382fbb5edae541e0c96` | 54 | 汽水 Android 搜索线索；大量第三方解析服务及 Python 重构 |
| MOPELotus/Lotus-ReFactor | `8d6ad59ca754a8055711c0b05c0e602442d406fe` | `69235396403664e31bf6cd91ef7f70646e5bba2f` | 19 | 音乐合伙人相关目录未变；变更是游戏及 B 站下载清理等 |
| UnblockNeteaseMusic/server | `39e21bfb4b7581f39785b190aeced201d23f0d41` | `1d64281c0fae81a4e3cf21f176706b0baa3a606d` | 8 | JS 依赖和 CI 更新，无酷我协议差异 |
| 520Qiuyu/qishuiMusicAnalysis | `b8f4e4f00be7c77ae6d12ca94d849c7f534cd3a9` | `1c9f8e40b14e3a35db49289225347bfc399797ff` | 1 | Docker 镜像标签变化，无新增源码协议 |
| L-1124/QQMusicApi | `108617ffe80abefec6358717b9f4d3677550db10` | 同左 | 0 | 本时间窗口无更新 |
| MOPELotus/BBDown | `259a5558cee0a349a7ebb60bd31e40c88e5bc1ed` | 同左 | 0 | 无更新 |
| bilibili-plugins/bilibili-api-collect | `cfc5fddcc8a94b74d91970bb5b4eaeb349addc47` | 同左 | 0 | 无更新 |
| qyhqiu/kuwoMusicApi | `e8e720b90b4d7e3052078a3380906f2b3349e388` | 同左 | 0 | 无更新 |
| listen1/listen1-api | `aa4b9d34aad577a254a70b2754415adcbb17294d` | 同左 | 0 | 无更新 |
| SaKongA/PopDownloader | `8e48fd1d01b7d3d4262149863818ae15ee7e3bc9` | 同左 | 0 | 无更新 |
| baizeyv/SodaDownloader | `893b49c35b7e11ada029e78782092f2553904281` | 同左 | 0 | 无更新 |
| naiyQAQ/qishui-decrypt | `d360c20a697f9988c6b567c924af5b9784d18390` | 同左 | 0 | 无更新 |

## 应优先处理的现有能力

### A1：汽水公开详情、歌词、可听性、播放与下载入口迁移

- 上游：8 月 28 日 [02402db](https://github.com/guohuiyuan/music-lib/commit/02402db) 报告 PC `track_v2` 下线，引入 `https://beta-luna.douyin.com/luna/h5/seo_track`。
- 当前代码：`crates/tuneweave-provider-soda/src/client.rs:731` 的 `fetch_track_v2_body` 固定 GET `/luna/pc/track_v2`。详情、歌词、可听性以及 `authorized_media` 共用它；因此不只是详情页面受影响。
- 实测：现有匿名歌曲详情用例失败，错误为 `Soda track detail returned malformed JSON`。对同一歌曲 `7304719759323564095`，SEO 端点 HTTP 200，返回匹配的 `seo_track.track.id` 及 `track_player.url_player_info`。
- 应做：独立建立 SEO 响应类型，正确处理 `seo_track.track`、根级/嵌套歌词、播放器入口与权益字段；串起详情、歌词、可听性、流和下载，重新验证免费全曲与付费试听边界。
- 不能照搬：上游失败后把 VIP 试听当下载成功返回。TuneWeave 必须保留试听属性与完整下载限制。SEO 有播放器入口不代表已验证完整媒体可播；本轮没有下载媒体，也没有验证会员流。
- 我们已有明文/加密媒体分支，不能把上游新增“无 play_auth 时不解密”误记成整套解密能力都要重写。
- “PC 下线”仅作为上游描述；本轮证据确认我们使用的匿名 GET 链失败，不推断所有带签名 PC POST 形态均永久失效。

### A2：汽水搜索迁移，随后补专辑与歌单搜索

- 上游：8 月 27 日 [095a3b9](https://github.com/guohuiyuan/music-lib/commit/095a3b9) 将歌曲、专辑、歌单搜索切到 `/luna/search/{track,album,playlist}`，处理多结果组及封面模板。musicdl 9 月 7 日的汽水调整也采用 Android 歌曲搜索。
- 当前代码：`crates/tuneweave-provider-soda/src/client.rs:29` 仍固定 `/luna/pc/search/track`。
- 实测：现有匿名搜索用例失败，错误为 `Soda search returned malformed JSON`；新的官方 Android 路径用精简公开参数即可返回 HTTP 200 和结果组，无需复制上游硬编码的设备 ID、安装 ID 或签名。
- 应做：先恢复歌曲搜索；核验游标、物理页宽、分类与资源身份，随后评估统一专辑/歌单搜索接入。按实际响应更新封面 URL 组合与允许主机。
- 验收缺口：新端点完整分页、空页、专辑/歌单结果和封面实际访问仍待实现阶段验证；不能用第一页 HTTP 200 代替全部验收。

### A3：网易云 NMTID 的来源、传递和生命周期

- 上游：8 月 24–25 日 [5c98a9a](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/5c98a9a)、[8376699](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/8376699)、[22003c2](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/22003c2)。协议线索是：无 NMTID 的 EAPI 请求可收到服务端 Set-Cookie；已有值应保留，并纳入 EAPI 请求头数据和 Cookie。
- 当前代码：`crates/tuneweave-provider-netease/src/client.rs:2026` 对非登录请求无条件随机覆盖 NMTID；`EapiHeader` 也没有 NMTID 字段。仅删除随机生成逻辑不足以完成更新。
- 应做：验证服务端下发条件；优先保留调用方已有值；按匿名设备、服务器账户、调用方凭证的所有权隔离身份、采集响应并持久化适当状态。覆盖重复请求、账户切换、重启、并发和服务端未下发的有限失败分支。
- 不能照搬：上游进程全局 `NMTID` 和全局三次预算会混用不同账户状态；随机保底值也不能被记录成服务端签发值。
- 本轮为源码确认的实现差异，尚未做账号级网络差分，不声称现有所有网易云请求已失效。

### A4：网易云易盾 checkToken V2 改为真正的 SDK Token

- 上游：8 月 7 日 [c4a3d83](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/c4a3d83)，8 月 9 日合并 [c1d14a8](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/c1d14a8)。原先读取 `/v2/config/js` 的 `result.conf`，现在由官方 Watchman SDK 对 businessId 产生 Token。
- 当前代码：`crates/tuneweave-provider-netease/src/client.rs:33`、`:1040`、`:1880` 仍取旧配置响应当 Token，并无期限缓存；克隆的账户 client 共享 Token 缓存。现有评论发布/删除明确使用 V2。本轮旧配置端点仍返回 HTTP 200、code 200 和非空 conf；问题是其业务含义与生命周期，不能描述为端点已经无法访问。
- 应做：重新确认配置材料与业务 Token 的区别，设计适合 Rust 的获取方式及受限生命周期；不要把配置串继续当反作弊 Token。明确不同业务版本、调用方 Token、账户隔离与超时失败的契约。
- 不能照搬：把上游 jsdom、动态脚本执行、全局浏览器实例直接引入轻量 Rust 服务。上游失败时返回空 Token 的行为也不应冒充注册成功。
- 上游同批将广告听歌权益领取切到 V3；TuneWeave 的 `gain_listening_rights` 已走 `request_xeapi_with_check_token`，其版本是 V3，因此此处无需重复迁移。但 V3 的共享缓存策略也应随生命周期审查，不能仅因旧用例能读出字符串就认为业务验收通过。
- 本轮未发送、删除评论，也未执行领取权益或其他账户写操作；这些业务成功态仍待后续受控验收。

### A5：网易云 vivid 和新沉浸声规格

- 上游：8 月 16–18 日 [881267c](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/881267c)、[0b5c392](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/0b5c392)；8 月 20 日合并 [a7e8d48](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/a7e8d48)。
- `vivid` 为臻音全景声。播放请求的 `encodeType` 改为 `mp3`，并使用 `os=android, appver=9.5.61`；下载也有该客户端身份分支。`sky` 新增 `c512/ste2/aac2` 三种 immerseType，保留旧三种。
- 当前代码：`netease_stream_request` 固定 FLAC；`netease_stream_level` 无 vivid；核心 `ImmersiveAudioType`、HTTP 解析器、网易云映射只支持 `c51/ste/aac`。
- 应做：同步强类型规格、HTTP 输入、请求身份、实际音质响应、下载/302 和 resolver 缓存键；确认 QQ 等平台遇到新平台规格时的兼容策略。不能把新规格静默压成旧规格，或用 B 站 HDR Vivid 视频枚举代替音频规格。
- 仅有请求协议差异证据；会员权益、实际编码、降级与完整下载需要真实验证。

## 其他值得接入的新能力

| 平台 | 本轮新增/调整 | TuneWeave 建议 |
| --- | --- | --- |
| 网易云 | 8 月 18 日 [a7437f7](https://github.com/NeteaseCloudMusicApiEnhanced/api-enhanced/commit/a7437f7)：`like_v1`（XEAPI V3 `/api/v1/radio/like`）、歌曲百科 `song_wiki_info`、插播相似歌曲 `song_simi_get` | 三条独立候选。红心接口应映射现有喜欢语义并核对歌单一致性；百科/插播保留各自数据结构，不能假定老百科摘要/相似推荐与新接口等价。没有证据要求立即删除老歌单增删链。 |
| 网易云 | 私人漫游文档新增 `PUZZLE_MODE_RCMD` 和更多场景子模式 | 当前 mode/subMode 是受长度约束的字符串透传，协议请求体没变；主要补文档和代表性验证，无需重写 FM。 |
| 酷狗 | 9 月 2 日 [2934a32](https://github.com/MakcRe/KuGouMusicApi/commit/2934a32)：Web 签名将原始 JSON 请求体纳入；新增 `/v2/scan` → `/v2/authorize` | 接入账户基础层时实现。两阶段必须匹配已登录 userid；appid 应匹配 Token 所属客户端。当前酷狗 Provider 没有账户登录能力，不能把这次修复描述为现有扫码登录故障。 |
| 酷狗 | 8 月 6 日设备踢下线 [7a60b70](https://github.com/MakcRe/KuGouMusicApi/commit/7a60b70)：专用 RSA 无填充 Token 封装与 Web 签名；8 月 14 日 QQ 登录；8 月 17 日登录透传 dev | 统一规划多账户、设备身份与登录事务。踢设备是独立账户写操作。不要沿用上游截断缓冲区、不完整参数检查或全局 standard/lite 切换。 |
| 酷狗 | 8 月 25 日 [ec5e0fb](https://github.com/MakcRe/KuGouMusicApi/commit/ec5e0fb)：`user_verify` → `song_auth` → `/tracker/v5/url`，以及聚合 Auth 播放和概念版免费听 | 有价值的新授权播放链。用户 auth、歌曲 auth、open_time、hash、album_audio_id 应独立建模；按账户持久化适当凭据。当前公开播放链未改，不能未经验证强制替换。上游若使用明文 HTTP，需先验证官方 HTTPS；聚合分支失败后应立即终止。 |
| 酷狗 | 歌手歌曲 V2（每项作者列表，页宽不超过 100）、歌单导入任务（URL/图片）、内容黑名单、AI 推荐与青年频道歌曲 | 先选能直接增强音乐检索/整理的能力；歌单导入是平台任务状态机，不是 Uni Playlist 本地导入的同名替代品。 |
| 酷狗 | 听书搜索/标签/免费书库，用户资料和头像、音效目录、听歌等级/时长、一起听房间与组队 | 按产品范围排期。资料/听书可后续接入；营销组队、时长奖励、社交房间不作为恢复现有播放的前置工作。 |
| 咪咕 | 新增 Android M2 播放、按 songId 下载、TV `tvMusicSongFileService` | TV 请求有新 3DES-ECB/PKCS7 与 MD5 鉴权封装，值得单独验证；现有 H5 解密未因本轮上游而变更。不能把“支持全球访问”的提交标题当成跨地区验证结果。 |
| 咪咕 | 批量歌曲详情、歌单搜索、广场标签/推荐/分页，歌手专辑分页与歌手 MV、字母索引，数字专辑详情与曲目，推荐页面/随心听 | 当前 Provider 仅有公开歌曲搜索、详情、歌词、歌单、可听性、流/下载；这些是新增统一目录能力，优先于消息、活动和支付功能。 |
| 咪咕 | 喜欢列表/取消、收藏类型扩充、关注歌手、已购歌曲、云盘上传链接申请/播放/下载/删除、退出、账户其他目录 | 先补账户与 PACM Token 所有权、轮换持久化，再接媒体库。上传模块先计算文件 MD5 并申请上传地址，不应误计为完整文件传输已由该模块完成。新支付接口只登记，不纳入本轮审计的执行动作。 |
| 汽水 | Android 专辑与歌单搜索、封面模板和多结果组 | 在 A1/A2 恢复现有能力后接入，避免同时扩大修复范围。 |

## 已覆盖、无需同步或不应直接采用

1. **咪咕 MRC/TRC**：上游 8 月 20 日新增歌词解密，但 TuneWeave 已有独立 MRC 解密、逐字与普通歌词分离、TRC 通道；不是本轮缺失的新功能。
2. **酷我歌单分页**：music-lib 的更新修复其旧 `nplserver/pl.svc` 只取 100 首的问题。TuneWeave 使用 HTTPS 官方 Web `playListInfo`，已有统一分页和总数漂移校验，不应改回历史端点或照搬全局去重。
3. **网易云推荐歌单**：music-lib 此轮将解析函数抽出并加回归测试，没有新增远端请求协议。
4. **咪咕音质处理**：上游把 PQ URL 改目录/扩展名来声称 HQ/SQ/ZQ32/I3D 等，并默认删除查询参数。只把这些代码当作音质目录线索；必须使用平台授权返回及真实格式，不移植删除签名、构造高音质 URL 的行为。
5. **第三方解析服务**：musicdl 大量提交是轮换外部解锁/解析站点及其签名，不属于官方音乐平台协议；不接入 TuneWeave。B 站和咪咕 source 的此轮差异只是 Python 类型标注。
6. **参考项目依赖和 CI**：Node/jsdom、Go x/text、Python typing、Docker/Actions 的更新不等于 Rust 依赖有相同问题。本轮不包含 TuneWeave Cargo 依赖安全审计。
7. **QQ、B 站主参考**：严格按 8 月 6 日时间基线，QQMusicApi、指定 BBDown fork 和 B 站文档库均没有之后的提交。QQ 上一次更新在 8 月 5 日，不计入本轮。未扩展审计到未指定的 BBDown 原作者仓库或其他 fork。
8. **音乐合伙人**：Lotus-ReFactor 音乐合伙人路径没有差异；本轮 B 站临时文件清理与游戏业务变化不迁入音乐合伙人 Provider。
9. **许可证记录**：咪咕根 LICENSE 8 月 15 日从 Apache-2.0 改为 MIT，但新 `quality.ts/url_m2.ts` 文件头写有 `cc-nc-4.0`，`tv2.ts` 写有 `cc-by-nc-sa-4.0`。应按文件记录原始声明，不能把新快照整体简化成 MIT。现有 THIRD_PARTY_NOTICES 描述旧快照，不能在尚未采用新协议时无说明地覆盖成“已迁移到新快照”。

## 实测记录与后续顺序

在 TuneWeave 基线代码上执行：

```text
cargo test -p tuneweave-provider-soda live_anonymous_track_detail_preserves_identity_and_hides_player_tokens -- --ignored --exact client::tests::live_anonymous_track_detail_preserves_identity_and_hides_player_tokens
结果：1 failed；Soda track detail returned malformed JSON。

cargo test -p tuneweave-provider-soda live_anonymous_search_uses_the_public_aid_without_device_or_signatures -- --ignored
结果：1 failed；Soda search returned malformed JSON。
```

另外，以不携带 Cookie 的直接 HTTPS 请求验证了官方 SEO 详情和 Android 搜索返回有效 JSON：SEO 的歌曲 `7304719759323564095` 身份匹配且有播放器入口；Android 精简请求的“落了白”查询返回 20 个带 track.id 的条目。另一首免费样本 `6911353635137914887` 的 SEO 请求在 15 秒内超时，未重试，因此新链的免费歌曲稳定性和完整播放仍待验收。旧易盾配置端点返回 HTTP 200、code 200、776 字符 conf，仅确认配置接口可达，不是业务 Token 可用性验收。

没有运行上游源码，没有执行外部解析站点，没有把原始播放器 URL、Token 或 Cookie 写进本报告。未进行全仓门禁：本轮没有代码变更，两项失败是本次审计复现的问题。

建议实施顺序：

1. A1/A2：恢复汽水详情链与歌曲搜索，完成免费/试听、歌词、分页和统一 resolver 验收。
2. A3：网易云 NMTID，先修请求层与隔离持久化。
3. A4：易盾 V2 的获取方式与生命周期；如果排期只关注播放，可先实施 A5，再回到评论等依赖 V2 的能力。
4. A5：网易云新音质及完整流/下载/302/回退契约；随后歌曲百科、插播相似和新版喜欢。
5. 酷狗：账户基础层 → 新授权播放 → 媒体目录和账户库。
6. 咪咕：先验证新播放入口与真实音质，再接公开目录；账户基础层完成后接个人媒体库。

实施时分别补协议向量/单元测试、旧新请求差分、真实服务验证，再更新对应已采用快照；不要一次性把所有候选都标成已接入。
