# 酷我项目范围覆盖账本

协议基线为 `qyhqiu/kuwoMusicApi@e8e720b90b4d7e3052078a3380906f2b3349e388`，并以当前酷我官网实际请求和平台真实响应校正参考项目中 2023 年的旧入口、静态 Cookie、签名与重试行为。状态沿用其他平台账本：`pending` 尚未实现，`implemented` 已完成代码和离线验证但缺真实成功态，`verified` 已完成统一 HTTP 与真实上游验收。

当前公开音源补充层共 7 个验收单元：`pending=2`、`implemented=0`、`verified=5`，完成度 `5/7 = 71.43%`。登录、账户和写操作属于后续酷我完整项目范围，不阻塞本层。

| ID | 验收单元 | 状态 | 实施与验收边界 |
| --- | --- | --- | --- |
| KW01 | 公开歌曲搜索与稳定身份 | `verified` | `GET /v1/search?platform=kuwo&kind=track` 使用当前官网匿名 HTTPS 搜索链和 0 起始页码，以 `MUSIC_<rid>` 严格返回稳定 `kuwo:<rid>`。平台物理页宽固定为 100，统一任意 offset 与 `limit<=100` 最多拼接 2 页并复核总数。强类型保留歌手、专辑、别名、时长、MV、公开目录、权益和媒体规格；已确认的 `s/h/p/ff/hr/dtsx` 映射统一音质，`zply/zpga*` 等未验证等级只保留原码。搜索目录不证明匿名播放权益，只有平台明确离线才设 `playable=false`。真实 Provider 已返回稳定结果；全新数据目录的统一 HTTP 以 `offset=99&limit=3` 跨两个物理页返回连续窗口、总数和下一 offset，且未泄漏旧响应中的不可信媒体地址。 |
| KW02 | 歌曲详情与身份补全 | `verified` | `GET /v1/tracks/kuwo:<rid>` 只接受规范正整数且不接受账户。Provider 通过固定 HTTPS 官网首页取得当次 32 位匿名跟踪 Cookie，在实例内最多缓存 30 分钟，并按当前官网算法用随机 8 位 nonce 生成一次请求 `Secret`；Cookie 与签名不持久化、不回显、不进入 Debug 或错误。会话只允许在 401/403 或平台精确 `The request is illegal!` 状态后刷新一次。详情必须同时回配 `MUSIC_<rid>` 和数字 `rid`，返回歌手、专辑、可信官方封面、时长、曲序、发行日、MV、无损目录、付费/试听标志、评分和有界专辑简介；离线外不猜测匿名可播。签名有浏览器算法差分固定向量，真实 Provider 与全新数据目录统一 HTTP 均已验收。 |
| KW03 | 普通与增强歌词 | `verified` | `GET /v1/tracks/kuwo:<rid>/lyrics` 并发访问固定 HTTPS LRCX 二进制链和移动端行级歌词链。LRCX 请求参数按平台 `yeelion` 循环 XOR 后作为单一 Base64 查询，响应要求 `tp=content` 信封、受限 zlib 解压、Base64 和同密钥解密，再严格解码 GB18030；逐字标记完整保存在 `word_synced` 并决定 `format=lrcx`。移动端成功时按浮点秒时间四舍五入为毫秒 LRC；该链当前会间歇返回查询失败，因此失败时从已验证的 LRCX 仅移除合法逐字标记派生 `plain`，不让低精度来源覆盖逐字正文。两条链独立失败，只有均不可用才报错；4/8 MiB 压缩与解压上限阻止大响应和压缩炸弹。真实 Provider 与统一 HTTP 已确认逐字优先、普通歌词可显示、移动端失败回退和端点/不透明查询不泄漏；当前公开链没有可验证翻译或音译时保持 `null`。 |
| KW04 | 公开播放、下载与权益 | `verified` | `GET /v1/tracks/kuwo:<rid>/availability`、`/stream` 与 `/download` 共用当前官网签名后的固定 HTTPS `playUrl` 链，只请求已真实验证的匿名 `128kmp3` 档。实时对照确认平台忽略 `320kmp3/2000kflac` 请求并签发同一 128 kbps MP3，因此即使调用方请求无损或 Hi-Res，也明确返回 `actual_quality=standard`、`bitrate=128000`，不把搜索目录中的无损规格冒充公开权益。`code=200` 且受信 `*-sycdn.kuwo.cn` 无查询 HTTPS MP3 才是完整全曲；`-1` 映射匿名权限拒绝，stream 返回 403、download 保持 `available=false/url=null`；`-1001` 表示资源不可用，当前链没有试听窗口时不伪造 trial。媒体 URL 禁止凭据、端口、片段、查询、嵌套主机和非 MP3 路径；Cookie、Secret、请求 ID 与端点不进入统一响应。免费与付费样本均已通过真实 Provider 和全新目录统一 HTTP 验收。 |
| KW05 | 统一播放回退与 302 | `verified` | 酷我已进入默认 resolver 的网易 → QQ → 酷狗 → 酷我 → 咪咕顺序，也支持显式 `playback_platform/source/fallback_platforms`；跨平台候选仍按标题、歌手、专辑、时长和版本严格评分，不因免费可播放宽阈值。真实统一 HTTP 已将网易“好运来”以 `match_score=1.0` 解析为免费酷我来源并保留 `requested_quality=lossless`、`actual_quality=standard`、128 kbps 与完整尝试轨迹；`source=kuwo` 使用同一解析器。调用方托管的 `kuwo:` Uni 项可无持久化播放。完整 stream/download 的 `/redirect` 只返回 provider 已校验的 HTTPS CDN，附 `private, no-store` 与 `no-referrer`；付费下载跳转返回 403 且没有 `Location`。默认顺序、显式来源、Uni 无状态播放和两类 302 均有服务端防回归测试。 |
| KW06 | 公开歌单与 Uni 导入 | `pending` | 使用当前可用歌单详情/歌曲分页，保持顺序与重复项并接入 Server/Client 两种 Uni 导入。 |
| KW07 | 协议、安全与真实权益验收 | `pending` | 固定官方 HTTPS 域名、响应上限、重定向和错误分类；全新数据目录执行真实搜索、详情、歌词、播放、歌单、Uni 与媒体探测。 |

参考项目的 `/api/www/search/searchMusicBykeyWord` 当前会返回拒绝或非法请求，官网已经改用 `/search/searchMusicBykeyWord` 与不同参数组合；TuneWeave 只实现实时验证可用的后者。详情和歌单当前仍可用，但需要官网根据当次匿名跟踪 Cookie 动态生成 `Secret`，不得把参考项目 2023 年的静态 Cookie、固定 CSRF、共享超时计数、递归重试或完整请求日志迁入。

后续完整项目范围将在公开音源层收口后扩展仍直接增强媒体体验的专辑、歌手、榜单、MV、广播目录，以及经实时验证后确有产品价值的账户能力；纯社交、直播互动、商城和营销活动不纳入。
