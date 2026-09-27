# 登录与凭证

TuneWeave 支持服务器托管账户和调用方托管凭证。登录方式由平台能力决定，调用前可查询：

```http
GET /v1/capabilities?platform=qq
```

## 凭证归属模式

登录请求的 `credential_mode` 有三个值：

| 值 | 保存到服务器 | 返回给调用方 | `account` |
| --- | ---: | ---: | --- |
| `server` | 是 | 否 | 可选，默认 `default` |
| `client` | 否 | 是 | 不得提交非空账户别名 |
| `both` | 是 | 是 | 可选，默认 `default` |

`client` 和 `both` 成功时，响应的 `caller_credential` 是版本化的不透明 bearer secret：

```json
{
  "format": "tuneweave_credential_v1",
  "platform": "qq",
  "value": "twc1_<opaque-base64url>",
  "expires_at": null
}
```

调用方必须保存完整 `value`，不要解析内部内容。包含凭证的响应带有 `Cache-Control: no-store` 和 `Pragma: no-cache`。

## 使用凭证

服务器托管账户通过平台和账户别名选择：

```http
GET /v1/account/profile?platform=qq&account=personal
```

调用方托管凭证通过可重复请求头发送：

```http
X-TuneWeave-Credential: twc1_<opaque-base64url>
```

一次请求最多携带 8 份凭证，每个平台最多一份。跨平台搜索、Uni Playlist 和播放回退可以同时使用不同平台的凭证。一个平台不能同时使用调用方凭证和显式服务器账户别名。

不要把凭证放入 URL、查询参数、普通 JSON 请求体、资源引用或 Uni Playlist 文档。

平台可能在读取账户资料时轮换会话。`GET /v1/auth/session`、`GET /v1/account`、`GET /v1/account/profile`、`GET /v1/users/{ref}`、两条会员入口、`GET /v1/account/playlists`、歌单收藏 PUT/DELETE 入口，以及携带账户来源的歌曲详情/歌词/可播放状态入口的响应可在 `meta.caller_credential` 返回更新后的调用方凭证，格式与登录响应中的 `caller_credential` 相同。收到该字段时，调用方应保存新的完整 `value`，用于后续请求；这些响应带有 `Cache-Control: no-store`。没有该字段时保留原凭证。

账户资料、会员、个人歌单、歌曲元数据及歌单收藏操作包含多个上游请求时，后续请求失败也可能在错误响应的 `meta.caller_credential` 中返回前面成功请求已更新的凭证。客户端应先处理该字段，再处理业务错误。会话失效或代际冲突的错误不会回传这份凭证。

所有业务入口也可通过可重复的 `X-TuneWeave-Updated-Credential` 响应头返回本次请求中各平台最新的调用方凭证，每个平台至多一个值，格式为 `平台=完整凭证`，例如 `soda=twc1_<opaque-base64url>`。它覆盖音频字节、重定向、批量、跨平台回退和 Uni 播放响应；单平台 JSON 入口原有的 `meta.caller_credential` 仍保留。多平台响应头可能被客户端合并为逗号分隔的值：逐项按第一个 `=` 分开，左侧选择平台，右侧作为不透明凭证完整保存。同一平台以响应头的最新值为准，并覆盖该响应内媒体 `headers` 中更早的凭证。没有更新时保留原值；认证失效或会话冲突时丢弃受影响平台的本地凭证，不把缺少更新理解为仍然有效。

收到凭证更新的响应和使用账户来源的响应禁止缓存。服务器将更新头列入 `Access-Control-Expose-Headers`；跨域访问是否允许仍由部署方的 CORS 配置决定。凭证响应头与媒体 `headers` 中的凭证都属于 bearer secret，不应写入日志。

服务器账户的轮换仅更新被选中的账户，不返回调用方凭证。`both` 模式只在明确的登录或刷新操作中同时保存和返回凭证，普通读取不会自动同步服务器与调用方各自持有的副本。

汽水会员入口 `GET /v1/account/membership?platform=soda` 和 `GET /v1/users/soda:{id}/membership` 都读取当前选中账户；`backend=front` 与 `backend=client` 使用相同的官方 PC commerce v2 会员链，结果标记 `extensions.backend=official_pc_commerce_membership_v2`。先核验 UID，再只请求会员资料，最后用候选 Cookie 再次核验同一 UID，成功后才保存或交付本次会员响应的凭证轮换。指定其他用户 ID 会被拒绝；中途注销、重新登录或换号后的成功和错误响应都不能跨越原登录代际。

`active` 只映射会员正文中的明确 `is_membership`，不由到期时间或旧 `/me` 资料推断。正数 `expire_time` 转为 UTC RFC 3339 `expires_at`，原始 Unix 秒保留在 `extensions.expires_at_epoch_seconds`；零表示未提供有效到期时间，`expires_at` 保持 `null`，不解释为永久会员。`membership_type` 同时保留到既有 `vip_stage`；`is_paying_user`、`is_about_to_expire`、`in_grace_period`、`last_membership_type` 只保留明确字段。缺失的等级、年费数量、图标和其他状态保持未知。主会员状态不与独立广告／播放权益混合，也不据此承诺某首歌可播或可下载；未查清结构的会员明细表不透传。

整次会员读取限 45 秒，会员正文上限 1 MiB。缺少本人会员对象、字段损坏或任一步失败会返回错误，不退回营销资料或合成非会员结果，也不返回待发的调用方凭证更新。此前已经独立核验的账户更新可以保留在所选会话；未核验的会员 Cookie 不会覆盖它。账户响应设置 `no-store`。真实普通／赠送／付费／过期账户及明细形态仍待最终验收。

汽水个人歌单入口 `GET /v1/account/playlists?platform=soda` 按自建、收藏的顺序统一分页，先在有界请求内验证两类目录完整，再应用 `offset` 和 `limit`。每项的 `extensions.library_section` 为 `created` 或 `saved`，`subscribed` 分别为 `false` 或 `true`。混合收藏中的非歌单不会计入结果总数；同一歌单若出现在两类目录中，会分别保留各自的目录项。`source_user_id` 表示查询账户，`owner_id` 仅在上游明确提供时返回，不用查询账户补填收藏歌单的所有者。分页信息缺失、游标循环、总数矛盾或超过每类 64 页的请求上限时返回错误，不返回残缺列表。该协议映射已经离线测试，真实账户返回形态仍待最终验收。

创建汽水空歌单使用 `POST /v1/playlists`，请求示例 `{"platform":"soda","name":"夜听","kind":"normal","visibility":"private","account":"personal"}`。仅支持普通空歌单；名称须有 1–30 个 UTF-16 单元且不含控制字符，并须明确选择 `public` 或 `private`。服务器账户别名与调用方凭据均可用。写入前验证所选 UID，发送一次官方 PC 创建请求，再完整回读本人自建歌单核对新 ID、名称、空曲目数与来源 UID。平台返回的目录模型不含可见性字段，所以结果只说明请求了哪种可见性，并标记 `visibility_verified=false`；不会把请求值说成已确认状态。45 秒总时限覆盖身份、创建和回读；写入发出后的 ACK、回读、超时或取消错误均不自动重试，写结果未确认时标记 `write_outcome=unconfirmed`。响应使用 `no-store`，真实账户创建和可见性仍待最终验收。

汽水个人歌单目录读取与歌单收藏操作各有 45 秒总时限，包含身份核验、全部分页及写后回读。请求期间账户注销、重新登录或换号时，迟到的成功和失败响应均返回 `conflict`。超时或取消后不向后续响应遗留待返回的调用方凭证；此前已验证的凭证轮换不回滚。收藏写入发出后的超时仍属于 `write_outcome=unconfirmed`，不可自动重试。

汽水歌单详情和曲目 `GET /v1/playlists/soda:{id}`、`/tracks` 支持服务器 `account` 或调用方凭证。账户请求先核验选中 UID，再完整读取官方账户歌单，逐页接受经过校验的会话更新；失败不会改用匿名来源。每次读取最多请求 128 个上游页，每页最多 100 个原始项目；原始游标包含不可见项目，短页或空页仍可能继续。只有明确结束且可见曲目数一致，才返回元数据或应用统一 `offset`/`limit`，保留顺序和重复歌曲。因此获取一个小窗口也可能需要多次上游请求。总数、所有者、排序或更新时间中途变化时返回错误。响应带 `complete_snapshot` 和不透明内容版本 `source_snapshot_id`；后者不是凭证，客户端不应解析其内部格式。真实账户的私有可见性仍待最终验收。

汽水专辑详情和曲目 `GET /v1/albums/soda:{id}`、`/tracks` 也支持显式服务器 `account` 或调用方凭证。先独立核验选中 UID，再完整读取官方 PC 专辑，核对专辑身份、曲目归属与声明数量，最后应用分页（`limit` 为 1–100）。顺序、重复曲目及 `extensions.album_position` 保持原样。响应最多 8 MiB、10,000 首，身份核验和读取共限 45 秒；失败或读取期间重新登录、切换账户、注销时不返回旧结果，不改用匿名来源。没有显式选择账户时仍使用公开分享入口，已存储的默认账户不会改变来源。

账户专辑响应中的 `source_user_id` 表示选中 UID，`complete_read` 表示已完整读取，`source_snapshot_id` 是绑定本次登录及有序目录的不透明版本。它在 Cookie 更新后保持稳定，但重新登录或目录变化后改变；Uni 的 `type=album` 导入会核对详情与各页版本，矛盾时不创建部分歌单。字段不表示上游事务快照，也不证明收藏、购买或播放权益。真实账户可见性仍待最终验收。

汽水歌单收藏使用 `PUT /v1/account/favorites/playlists/soda:{id}`，取消收藏使用同一路径的 `DELETE`，支持服务器别名或调用方凭证。写入前核验选中账户身份，写入后完整回读收藏目录核对目标状态；只有核对一致才返回 `subscribed`。请求已发出但写入响应或回读失败时，错误中的 `details.write_outcome` 为 `unconfirmed`，表示最终状态尚未确认，不保证上游没有发生变化；这类错误不标记为可自动重试。调用方可先重新读取收藏目录再决定后续操作。此入口不会自动重发写入，也不会在注销或重新登录后保存旧响应中的会话。真实账户写入与回读仍待最终验收。

取消歌单收藏还要求混合目录的原始总数与完整读取一致，且没有无法识别的收藏类型。缺少总数或出现未知类型时，过滤后的歌单列表里没有目标，也不足以确认取消成功；仍返回 `write_outcome=unconfirmed`。已知的专辑条目按混合目录计数，不会被当作歌单。

汽水“喜欢”支持 `/v1/account/favorites/playlist?platform=soda` 和 `/v1/account/favorites/tracks?platform=soda`，以及 `/v1/users/soda:{uid}/favorites/playlist`、`/tracks`。用户 UID 必须属于选中的账户；服务器别名和调用方凭证均可使用。先完整读取账户目录，以明确的集合类型定位真实歌单，再校验详情的 ID、类型及上游提供的所有者，保留原名称、封面、顺序和重复曲目；不会按歌单标题选择，也不会用抖音喜欢替代。没有暴露喜欢歌单时返回错误，不生成虚拟歌单或假空列表。

喜欢/取消喜欢歌曲使用 `PUT` / `DELETE /v1/account/favorites/tracks/soda:{id}`。写后完整回读该账户的喜欢歌单，只有目标歌曲状态一致才返回 `subscribed`。歌单可能隐藏不可用歌曲，因此取消喜欢还会读取目标歌曲的账户状态，要求明确的 `is_collected=false`；字段缺失保持未知，不把列表中未找到当作删除证明。写入或后续回读失败时返回 `write_outcome=unconfirmed`，不自动重试写入。这些读取和写入的会话轮换由 `X-TuneWeave-Updated-Credential` 回传，响应为 `no-store`。协议和隔离逻辑已完成离线验证；真实账户下的目录形态及写后可见性仍待最终验收。

喜欢读取与写入各有 45 秒总时限，涵盖身份核验、目录定位、全部曲目页及取消喜欢的单曲状态确认。每次成功或失败响应均核对起始登录；中途注销、重新登录或换号时返回 `conflict`。取消、超时、认证失效及登录冲突不交付待返回的调用方凭证；此前已经验证的会话轮换不回滚，普通业务错误仍可返回这些更新。写入发出后的超时保持 `write_outcome=unconfirmed`，取消请求也不保证撤销已送达上游的写入。

汽水收藏专辑支持 `GET /v1/account/library/albums?platform=soda` 和 `GET /v1/users/soda:{uid}/favorites/albums`，后者只接受选中账户本人的 UID。服务器别名和调用方凭证均可使用。先完整读取混合收藏目录，再筛选专辑并应用 `offset` / `limit`（1–100）；每次最多读取 64 个上游页，每页最多 100 个原始项目。分页 `total` 是专辑数量，`extensions.upstream_raw_collection_count` 如有返回则包含歌单等原始收藏项，二者不能混用。专辑保留实际 ID、名称、歌手、简介和可用封面，缺失的曲目数保持未知；`extensions.subscribed=true` 表示来自收藏目录。分页矛盾或预算耗尽返回错误，不交付截断列表。收藏记录本身不证明账户具有该专辑的购买或播放权益。

收藏/取消收藏专辑使用 `PUT` / `DELETE /v1/account/library/albums/soda:{id}`；批量使用 `PUT` / `DELETE /v1/account/library/albums`，JSON 例如 `{"refs":["soda:11","soda:22"],"account":"personal"}`。汽水批量一次接受 1–100 个 ID，先校验全部输入，再按输入顺序逐项写入和完整回读，不提供原子事务。取消收藏还要求原始条目总数一致、目录中没有无法识别的条目类型，才能以目标缺席确认结果。写入或确认失败时返回 `write_outcome=unconfirmed`，不自动重发；批量错误还提供 `completed_refs`、`failed_ref`、`remaining_refs` 和 `atomic=false`，分别表示已确认项、结果未确认项和尚未尝试项。普通业务失败时，前面成功请求的会话更新仍通过更新头返回；认证失效、会话冲突、取消或总超时不交付更新；所有响应为 `no-store`。真实账户目录及写后可见性待最终验收。

专辑收藏读取、单项写入及整批写入各限 45 秒，包含身份核验、所有分页与回读；批量不会逐项重置总时限。写入发出前失败不声明写入结果未确认；发出后失败或超时不重试，批量保留此前已确认的项目并停止后续项。任何成功或失败网络响应均核对原登录代际，同 Cookie 重新登录也不能接收旧结果。取消、总超时、认证失效或会话冲突不遗留待交付的调用方凭证更新；此前已验证并保存的会话轮换不回滚。

汽水搜索建议 `GET /v1/search/suggestions?platform=soda&client=pc&q=周` 支持显式服务器 `account` 或调用方凭证。先独立核验选中 UID，再读取官方 PC 建议；所有关键词解析成功后才接受建议响应的 Cookie 更新，调用方通过更新头取得整次成功的最终凭证。读取中途注销、换号或重新登录会拒绝迟到结果；失败、超时或取消不留下待发的调用方更新。未指定账户时保持匿名，不读取存储默认会话。`authenticated=true` 和 `source_user_id` 只说明请求使用了核验后的账户，不保证上游返回个性化建议。真实账户行为待最终验收。

汽水统一搜索 `/v1/search?platform=soda&type=track&q=周` 也支持所选账户，另有 `album`、`artist`、`playlist` 三类。初始 UID 核验、分页 Cookie 更新、成功回传与失败清理遵循相同归属规则；整次搜索绑定原登录代际，不能中途换用新登录。账户搜索来源不等同于个性化结果或播放授权，参见[统一搜索](api-v1.md)。

汽水歌曲详情、歌词和可播放状态入口支持账户来源：`GET /v1/tracks/soda:{id}`、`/lyrics` 和 `/availability` 可使用 `account` 选择服务器会话，也可使用调用方凭证。未选账户时保留匿名公开链；选定账户后先核验 UID，再请求官方账号元数据，失败不会静默换为匿名结果。返回的 `backend=official_pc_track_v2` 表示使用了账号请求。可播放状态仍以歌曲响应中的实际授权、媒体身份、有效期及试听范围为准，不根据会员资料推定全曲权益。账号 `/stream`、`/download` 和 `/stream/content` 已接入同一账户链，复用返回的直接播放器模型验证及内容处理；只有试听授权时不提供完整下载 URL。直接模型缺失、为空或为 null 时，可读取官方 `url_player_info` 的明文或 CENC 媒体授权，并继续检查媒体身份、全曲/试听范围及各档有效期；损坏的非空直接模型不会触发回退。二级 CENC 要求明确的 `cenc-aes-ctr`、完整 `PlayAuth` 和 32 位十六进制 `PlayAuthID`；内容交付时继续核对媒体内的密钥标识，缺失、冲突或其他加密方式明确报错。密钥及其授权材料不会进入媒体 JSON 或发送给音频 CDN。授权 CENC 解密后的 AAC／ALAC 交付为不带活动加密标记的 M4A；已使用的保护信息以等长空闲区替换，保持样本偏移及预滚信息，避免下游把明文包识别为仍需解密。FLAC 交付为原生 FLAC。当前支持单音轨、非分片及单个已授权密钥；含额外轨道、分片或未支持的样本密钥分组时明确拒绝。真实账号响应形态与音质仍待最终验收。

汽水账户歌词与匿名歌词均已接入原文 LRC、逐字 KRC、官方明确声明 `type=text` 的无时间轴原文和官方中文译文。纯文本返回 `format=text`，不生成时间标签或逐字轨，损坏的定时歌词不回退纯文本。原文与 `translated` 分开返回；译文保留官方时间标签，没有译文时保持为空。账户身份、凭据归属和轮换规则适用于所有原文格式，真实账户和纯文本实包仍待最终验收。

汽水明文内容完整缓冲上限为 512 MiB，并须与授权声明的字节数一致。AAC／ALAC 检查单一音频轨的 MP4 容器、编解码配置、内部数据引用、样本位置及时间表；允许正常预滚分组，拒绝加密分组、其他保护标记、碎片化文件和额外音轨。原始 FLAC 检查元数据边界、子帧编码结构、帧序号、样本总数和 CRC；截断或多余尾部字节不作为完整内容交付。校验成功保留原字节，不重新编码。这些是容器及帧结构检查，不等同于完整 PCM 解码或真实账户播放验收。

汽水媒体 JSON 中的本地 `url` 必须与 `headers` 一起使用。服务器账户 URL 保留所选 `account`；调用方账户 URL 不含凭证或服务器别名，`headers` 携带本次操作接受的调用方凭证，仅用于请求同一 TuneWeave 服务。二级播放器和音频 CDN 请求都不会携带汽水账户 Cookie，也不会用它们的 Set-Cookie 更新账户。媒体授权失败不等于登录会话失效；二级播放器返回 401/403 时按媒体权限失败处理。二级请求或音频传输完成后若发现该账户已注销或重新登录，返回会话冲突，不交付旧会话结果。二进制响应的凭证轮换通过上述更新头返回。自动跟随重定向无法保证替换原凭证请求头，因此调用方模式应优先使用 JSON URL 与 headers，或先处理更新头再手动跳转。

咪咕会员入口 `GET /v1/account/membership?platform=migu` 和 `GET /v1/users/migu:{uid}/membership` 支持服务器账户或调用方凭据，`backend=front` 读取官方会员中心和图标，`backend=client` 额外读取跨业务会员身份。指定 UID 必须属于选中账户；读取过程中核验 UID、接受已验证的凭据轮换，再查询会员中心与会员图标。基础读取包含五个官方请求，详细模式包含七个，会员接口返回的新凭据会经账户资料核验后才接受；失败不回退为匿名会员资料。

咪咕本人资料 `GET /v1/users/migu:{uid}?backend=modern` 和 `GET /v1/account/profile?platform=migu&backend=modern` 复用已核验的官方账户资料接口。UID 必须等于所选账户身份；服务器别名和调用方凭据均可用，调用方来源不会写入服务器账号。资料只映射已确认的用户 ID、昵称和头像，歌单数量、等级、关注关系、生日等未由该接口提供的字段保持未知。响应中的 PACM 轮换仅在同一登录代际再次确认后保存或通过更新凭证交付；登录退出、换号或错误响应不会回退匿名资料，也不会泄露 PACM。

咪咕 `active` 依据当前权益身份或明确生效的订阅，不依据产品卡标题、付费推荐或图标。未知身份保持 `null`，用户成长等级不填入付费会员 `level`。`extensions.cards` 分别保留产品类型、当前身份、订阅及待生效／待激活项；不会返回订购链接、营销说明或账户秘密。`member_icons` 保留受控图标及名称，图标本身不证明可播放权益。

咪咕 `expires_at` 仅在唯一明确生效的订阅有日期且不存在其他未知当前身份或订阅生效状态时返回；多个并存会员的日期分别保留在 `cards[].subscriptions[].expires_at`。日期保持平台本地日历精度，例如 `2026-10-01`、`2026-10-01T12:30`，不补造时区或日末时刻；`extensions.expiry_format=platform_local_calendar` 明确这一格式。连续包月和平台 2099 哨兵值保持未知到期日。详细模式的 `extensions.media_member_identities` 单独保留跨业务身份、付费类型和日期，不将它们合并成音乐会员的 `active` 或到期日；详细来源请求失败会返回错误，不伪装成空列表。会员资料不是某一歌曲的实时播放／下载授权。响应与失败路径的凭据更新均遵循上述 `no-store` 规则，真实普通及付费账户仍待最终验收。

## 汽水官方签名服务

汽水 PC 接口的设备签名可使用 TuneWeave 进程内配置的本地运行时，也可以连接单独运行的签名服务。Windows 用户可下载 [`TuneWeave-SodaSigner.exe`](https://raw.githubusercontent.com/MOPELotus/TuneWeave/dev/tools/auxiliary/TuneWeave-SodaSigner.exe)；它默认只监听 `127.0.0.1:7833`，通过 `/healthz` 提供状态检查，并处理 `/v1/sign` 的 native 与 Passport 签名请求。首次启动时，服务会把随程序提供的运行文件解压到当前用户的 `%LOCALAPPDATA%\TuneWeave\SodaSigner`，不会写入 TuneWeave 发布包。

服务启动后会生成并显示一个随机 bearer token。把显示的同一 token 配置给 TuneWeave 服务进程：

```text
TUNEWEAVE_SODA_BDMS_SERVICE_URL=http://127.0.0.1:7833
TUNEWEAVE_SODA_BDMS_SERVICE_TOKEN=<签名服务显示的 token>
```

若服务由管理员显式配置了 token，TuneWeave 仍使用此变量传入同一值。服务端可用 `TUNEWEAVE_SODA_BDMS_BIND` 修改监听地址，默认地址保持 loopback；设置 `TUNEWEAVE_SODA_BDMS_TOKEN` 可指定服务 token。绑定到 loopback 以外的地址时，必须同时设置 `TUNEWEAVE_SODA_BDMS_TLS_CERT` 和 `TUNEWEAVE_SODA_BDMS_TLS_KEY`，并在 TuneWeave 端使用受信任证书对应的 HTTPS 地址。TuneWeave 只接受 HTTPS，只有 localhost/loopback 才允许 HTTP；URL 配置为服务 origin，不附加路径。

签名服务需要持续运行。关闭或无法连接时，汽水请求会返回“签名服务未运行或无法访问”；此时可启动服务或恢复本地运行时配置。服务不会保存账户凭证，但会在内存中短暂接收 Passport 签名所需的请求 Cookie；不要把 token、Cookie 或请求正文写入日志，也不要将 bearer token 公开给其他用户。远程连接必须使用 TLS。

## 二维码登录

创建事务：

```http
POST /v1/auth/qr
Content-Type: application/json

{
  "platform": "qq",
  "login_type": "qq_music",
  "account": "personal",
  "credential_mode": "server"
}
```

Soda QR clients may also send an optional `client_context` object containing the
browser environment fields used by the official PC login SDK. TuneWeave validates
and encodes this context for the upstream request, then keeps it only in the
in-memory QR transaction. Do not include cookies, tokens, passwords, or account
identifiers. Other platforms reject this field.

响应包含 TuneWeave 事务 ID、二维码内容和过期时间。使用事务 ID 轮询：

```http
GET /v1/auth/qr/{transaction_id}
```

状态为 `waiting`、`scanned`、`verification_required`、`confirmed`、`expired` 或 `failed`。凭证归属模式在创建事务时固定，确认阶段不能更改。

汽水 HTTP 扫码从创建时绑定目标账户和当时的服务器凭据状态。扫码、附加验证及最终身份核验共用不超过 5 分钟的期限；上游给出更短期限时使用更短值。注销、上游会话撤销或重新登录后，旧事务不能保存或交付凭证。取消正在执行的轮询／验证会消费该事务，须重新创建二维码；返回错误含 `challenge_consumed=true` 时，HTTP 事务也被移除；正常等待及可重试错误仍保留原有冷却和验证状态。

Rust SDK 可使用 `start_qr_login_for_account(login_type, account, mode)` 在创建时指定账户。汽水旧 `start_qr_login_with_mode` 仍允许首次轮询时选择服务器别名，但此前任何服务器账户变化都会保守地拒绝旧事务；需要互不影响的多账户登录时使用带账户的新方法。`client` 模式不读取或修改服务器账户。

汽水凭证导入的总期限为 45 秒；导入和扫码共用每个 Provider 及其克隆最多 128 个待完成事务的限制。服务器登录只在原目标状态未变时条件保存。同一 Provider 及其克隆中的本地注销、显式上游撤销也会取消尚未落盘的首次登录。独立 Provider／进程之间由凭据仓库的条件写入保护现存记录和首次写入竞争；不保证检测“原本为空、另一个进程完成登录后又删除、再次为空”的全部历史。


当状态为 `verification_required` 时，响应中的 `verification` 提供可用的验证方式、脱敏手机号及重发等待时间。使用原事务 ID 继续验证：

```http
POST /v1/auth/qr/{transaction_id}/verification
Content-Type: application/json

{"action":"send_sms"}
```

收到短信后提交 `{"action":"submit_sms","code":"<6-digit-code>"}`。如果平台提供 `up_sms` 方式，则由用户按返回的目标号码和内容发送短信，再提交 `{"action":"verify_up_sms"}` 校验。仅使用 `verification.methods` 中的平台可用方式，并遵守 `resend_after_secs`。

这些动作沿用原事务的账户、凭证归属和有效期，不能提交新的 `account`、`credential_mode` 或平台内部验证参数。验证响应为 `scanned` 时继续轮询二维码；只有 `confirmed` 才表示 TuneWeave 已取得并核验账户会话。

## 凭证导入

支持 `credential_import` 的平台可以导入已有会话。汽水 Cookie 导入示例：

```http
POST /v1/auth/import
Content-Type: application/json

{
  "platform": "soda",
  "account": "personal",
  "credential_mode": "both",
  "credential": {"kind": "cookie", "value": "sessionid_ss=<session-value>"}
}
```

导入会先向平台核验账户身份，成功后再按 `server`、`client` 或 `both` 保存或返回凭证。`client` 不接受服务器账户别名。汽水拒绝重复 Cookie 名和临时 MFA Cookie；所有平台都拒绝未知请求字段和超限请求。失败不覆盖已有账户。此入口只从 JSON 请求体读取凭证，不同时接受 `X-TuneWeave-Credential`。

咪咕使用同一入口，设置 `platform: "migu"`，凭证为 `{"kind":"cookie","value":"pacmtoken=<session-value>"}`。TuneWeave 先检查 PACM，再读取官方账户资料验证 UID，最后保存或返回凭证。重复、空或无效的 `pacmtoken` 会被拒绝；其他 Cookie 不会保存或发送到平台。密码、手机号、位置和平台中间登录 token 不会进入此导入结果。

咪咕当前支持密码与短信登录、图形验证接续、密码二次短信验证、导入、账户资料、会员读取、会话管理、个人歌单与喜欢集合、普通／数字专辑收藏、单曲／专辑已购记录、账户 PC 播放及普通非加密歌曲的独立下载授权。下载 URL／302 路径仍拒绝加密内容；`/download/content` 已支持经独立授权、解密和完整性校验的受限 MG3D MP3／FLAC 内容。MGM、云盘／云端已购的专属下载及未证实格式仍不支持，其余附加验证仍有未实现分支。已选咪咕账户的目录或媒体请求在对应能力实现前会明确拒绝，不自动使用匿名来源；真实账户与权益验收仍待完成。

咪咕两个服务器登录同时基于相同的空账户或旧凭据完成认证时，仅一个结果能够写入；另一个返回 `conflict`，不会覆盖胜出的会话或交付调用方凭据。`client` 登录不写服务器存储。Rust 自定义 `AccountCredentialStore` 必须原子实现 `insert_if_absent` 和 `compare_exchange` 才能支持这些服务器账户操作；默认实现明确返回不支持，不回退到先读取再无条件保存。

内置文件存储使用固定的私有 `.credential-store.lock` 文件协调同一目录下的读取、写入、替换和删除。多个进程共享目录时必须全部使用这一锁协议，底层文件系统须支持操作系统文件锁；服务运行时不能删除或替换锁文件。进程退出会释放锁。凭据文件名在时钟回拨后仍递增，旧代文件未清理完不会重新成为当前会话。

## 密码登录

```http
POST /v1/auth/password
Content-Type: application/json

{
  "platform": "netease",
  "principal_type": "phone",
  "principal": "<phone>",
  "password": "<password>",
  "country_code": "86",
  "credential_mode": "client"
}
```

平台可以要求特定 `principal_type`、`password_format` 或安全验证码。调用前读取平台能力，并按统一错误中的 `details` 处理额外验证要求。

`backend` 可省略或填 `default`，保留平台现有密码登录流程。酷狗还支持显式选择 `web` 或 `native`；酷我支持 `native` 和交互式 `web` 图形验证码流程；咪咕和网易目前只接受 `default`。未知或平台尚未实现的后端会在联网前拒绝，不自动切换登录协议。Rust 的 `PasswordLoginRequest.backend` 使用 `PasswordLoginBackend`，构造请求时可用 `Default::default()`；待验证收据固定最初的后端，续接动作不能更换后端。

咪咕使用同一密码入口，设置 `platform: "migu"`，支持 `principal_type` 为 `phone`、`email` 或 `username`，只接受 `password_format: "plain"`。手机号为中国大陆 11 位号码，`country_code` 可省略或为 `86` / `+86`。TuneWeave 获取当次平台公钥后加密登录字段，再交换音乐会话并核验同一 UID，成功后按 `server` / `client` / `both` 保存或交付凭证；密码和临时登录材料不写入账户存储，失败不覆盖原账户。服务端使用数据目录中的 `migu-music-device.json` 保持音乐设备身份。

咪咕普通密码登录成功时，继续返回账户资料与对应模式的凭据。平台要求算术、汉字图形、二次短信或语音呼叫验证时，返回 `state: "verification_required"`、`transaction_id` 和 `verification`，此时不包含登录凭据。所有密码登录及续接响应均为 `no-store`，真实账户仍待验收。

当 `verification.method` 为 `image` 时，向用户展示 `verification.image.image_data_url`，按 `answer_kind` 输入图形答案，并在同一事务中重新提交密码：

```http
POST /v1/auth/password/challenges/{transaction_id}/verify
Content-Type: application/json

{"action":"submit_image","answer":"<用户输入的图形答案>","password":"<原账户密码>"}
```

需要换图时提交 `{"action":"refresh_image"}`，遵守 `verification.image.refresh_after_secs`；图形最多回答 5 次、主动刷新 5 次，刷新间隔至少 2 秒。待验证事务不会保存密码，重新提交时使用新的平台公钥加密。图形动作不能更换原账号、手机号、密码格式或凭证归属。

当 `verification.method` 为 `sms` 时，二次短信已发往 `masked_destination` 对应的账户手机号。用户收到短信后，在上述密码续接入口提交 `{"action":"submit_sms","code":"<4位或6位短信验证码>"}`；需要重发时提交 `{"action":"resend_sms"}`，遵守 `resend_after_secs`。同一事务最多发送 3 次（含首次）、校验 5 次；同一登录标识的二次短信发送至少间隔 60 秒，发送失败或结果不确定也保留冷却。平台加密的手机号只留在服务端临时上下文，不交付调用方，也不从脱敏号码构造请求。

当 `verification.method` 为 `voice` 时，平台已改用自动语音呼叫，仍拨打脱敏字段 `masked_destination` 对应的号码。收到电话后提交 `{"action":"submit_voice","code":"<4位或6位语音验证码>"}`；重新拨打使用 `{"action":"resend_voice"}`，遵守 `resend_after_secs`。每个事务最多发送 3 次（含首次），每次至少间隔 60 秒，并共用原登录开始后的 5 分钟期限及 5 次验证码预算。语音验证码与短信验证码是不同通道；语音校验失败或结果不确定会消费当前事务，须重新开始登录，不能改用 `submit_sms` 重放。

密码事务与短信事务共用咪咕的 128 个待验证名额，有效期为 5 分钟；HTTP 侧二维码、短信和密码事务共用 128 个名额。容量在上游调用前预留，中间进展不延长有效期。同一事务并发操作被拒绝；取消、过期、未知结果或已消费错误后不可重放。明确错误的二次短信验证码可以在剩余次数内重试。新登录或注销会清理对应服务器账户的旧密码和短信事务，其他账户及调用方凭证相互独立。

Rust 使用 `begin_password_login(&request, mode)` 与 `advance_password_login(&receipt, &action)`，从 `PasswordLoginProgress::Pending` 保留原 `ProviderPasswordChallenge`，直到 `Confirmed`。旧 `password_login_with_mode` 保留普通单次登录语义，无法接续附加步骤。密码、短信、语音使用明确区分的动作；`secure_captcha` 不能用作任意验证码参数透传。滑动及手机号绑定等其他验证分支仍未接入，返回带平台状态码的明确错误。可通过凭证导入使用已有音乐会话。

## 验证码登录

创建认证事务并开始发送挑战：

```http
POST /v1/auth/challenges
Content-Type: application/json

{
  "platform": "netease",
  "method": "sms",
  "principal": "<phone>",
  "country_code": "86",
  "credential_mode": "server",
  "account": "personal"
}
```

提交验证码：

```http
POST /v1/auth/challenges/{transaction_id}/verify
Content-Type: application/json

{ "code": "<code>" }
```

发送前会预留服务端事务容量（二维码、验证码与密码共计最多 128 个），容量不足返回 `rate_limited`，不会发送短信。事务最长保留 10 分钟，平台可能有更短的有效期；手机号、平台和凭证归属在创建后固定。相同事务并发验证返回 `conflict`，成功后不可重复使用。上游允许重试的失败可以在原有效期内再次提交；平台明确消费事务时，错误中的 `details.challenge_consumed` 为 `true`，必须重新创建事务。验证请求被取消时，结果可能不确定，该事务会被移除，不自动重发。创建与验证的成功和失败响应均为 `no-store`。

Rust 调用方使用 `MusicProvider::begin_auth_challenge(&request, mode)` 保存返回的 `ProviderAuthChallenge`，再调用 `complete_auth_challenge(&challenge, code)`。收据只保存在内存中，包含原始请求和平台内部事务句柄，不是登录凭证；不要记录或序列化它。现有无状态平台通过默认适配保留原有行为，有状态平台自行校验收据绑定、有效期及一次消费。

咪咕短信使用同一入口，设置 `platform: "migu"`，只支持 `backend: "standard"` 和中国大陆 11 位手机号；国家码可省略或为 `86` / `+86`。支持 `server` / `client` / `both`，事务有效期为 5 分钟，同号码从创建开始至少间隔 60 秒才能再次发起；发送失败或结果不确定也保留该冷却，不自动重发。每个事务最多提交 5 次验证码，支持 4 位或 6 位数字，明确错误验证码会返回 `remaining_attempts`。会话交换失败、超时或尚未支持的附加验证会消费当前事务，避免重放；只在取得音乐会话并核验同一 UID 后保存或返回凭证。服务器别名在期间被重新登录或注销时，原短信事务不能覆盖它。

咪咕临时 passport Cookie 只在该事务内存中保留，按响应轮换及删除，不交付给调用方、不写入账户文件。短信和人工图形验证流程已通过离线测试，真实短信与账户仍待验收；滑动及二次验证的后续步骤尚未接入。Rust 必须使用收据方法，旧的无句柄 `start_auth_challenge` / `verify_auth_challenge` 不支持咪咕，以免两个同号码事务混用状态。

咪咕要求算术或汉字图形验证时，创建或验证接口返回 `state: "verification_required"` 和 `verification`，保留原 `transaction_id`。`verification.image_data_url` 是供用户查看的内嵌 JPEG，`answer_kind` 为 `arithmetic` 或 `chinese`。创建时出现这一状态，表示短信尚未发送。调用方展示图片，用户输入答案后提交：

```http
POST /v1/auth/challenges/{transaction_id}/verify
Content-Type: application/json

{"action":"submit_image","answer":"<用户输入的答案>"}
```

需要换图时提交 `{"action":"refresh_image"}`，并遵守 `refresh_after_secs`。每个事务最多提交 5 次格式正确的图形答案、主动刷新 5 次，刷新间隔至少 2 秒；错误图形回答后可能返回新的图片及 `remaining_attempts`。算术答案为 0–99（不含多余前导零），汉字答案为 2–4 个汉字。刷新和回答都不能更换手机号、账户或凭证归属。

当返回 `state: "waiting"` 时，短信发送已获平台确认，继续提交旧格式 `{"code":"<短信验证码>"}`（也兼容 `captcha`）或 `{"action":"submit_code","code":"<短信验证码>"}`。中间状态不包含 `profile` 或 `caller_credential`，也不延长有效期；只有 `state: "confirmed"` 才完成登录。若短信校验阶段要求图形验证，完成图形步骤后需再次由调用方提交短信验证码，不自动保存或重用上一份短信验证码。人工图形校验后尝试发送短信会重新开始 60 秒发送冷却。

Rust 调用方在 `begin_auth_challenge` 后使用 `auth_challenge_status(&receipt)` 读取初始状态，用 `advance_auth_challenge(&receipt, &action)` 继续；返回 `AuthChallengeProgress::Pending` 时保留同一收据。已有平台的普通验证码流程仍可用 `complete_auth_challenge`；咪咕在该方法遇到新增图形要求时返回可接续的 `authentication_required`，需读取状态并使用图形动作。

`POST /v1/auth/challenges/validate` 只校验验证码，不创建登录态。`POST /v1/auth/security-challenges` 用于已登录账户的安全操作验证码。国家和地区号可从 `GET /v1/auth/country-codes?platform=<platform>` 获取。

## 会话管理

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `GET` | `/v1/auth/session` | 返回脱敏会话状态 |
| `POST` | `/v1/auth/session/refresh` | 刷新凭证；调用方模式返回新凭证 |
| `POST` | `/v1/auth/session/revoke` | 显式请求上游会话失效并独立核验；当前支持汽水与酷我 |
| `DELETE` | `/v1/auth/session` | 清理选定会话，返回服务器别名删除结果与调用方凭证丢弃要求 |

服务器账户通过 `platform + account` 选择。调用方凭证通过 `X-TuneWeave-Credential` 提交。刷新可能包含多个上游步骤；失败响应也可能携带前面步骤已确认的凭证更新，具体语义见各平台说明。

汽水的刷新会回读官方账户身份，并接收平台返回的 Cookie 更新；`extensions.refresh_method` 为 `account_revalidation`，`extensions.refreshed` 表示本次回读是否改变了来源凭证。这不保证延长平台会话有效期，失效会话需要重新登录或导入。使用调用方凭证刷新到 `both` 时，必须指定属于同一次登录的现有服务器别名；发生冲突会返回 `conflict`，不会覆盖另一登录。

汽水 `DELETE /v1/auth/session` 只清理 TuneWeave 的选定别名，并在调用方模式返回 `caller_credential_discard_required: true`；它不会向汽水发送撤销请求，其他持有者的凭证仍可能有效。`removed` 仅表示服务器别名是否被删除。重复注销已移除的别名返回 `removed: false`；`both` 不会删除属于另一登录的别名。

显式上游撤销使用 `POST /v1/auth/session/revoke`，JSON 为 `{"platform":"soda","account":"personal","credential_mode":"server"}`，也支持 `client` / `both`；调用方凭据仍放在请求头。`client` 只允许默认别名，不读取服务器账户；`both` 要求显式服务器别名和同一次登录的调用方凭据，实际使用该别名当前 Cookie。默认模式为有调用方凭据时 `client`，否则 `server`。SDK 对应 `MusicProvider::revoke_session_with_ownership`，能力名为 `session_revocation`；未实现的平台明确返回不支持。

先用选定 Cookie 独立核验原 UID，再发出一次固定官方退出请求，最后用同一 Cookie 回读账户接口。前后身份回读与退出请求使用同一个持久安装设备身份和 `fp`；尚未绑定 UID 的输入在创建设备或联网前拒绝。结果 `state=invalidated` 表示先前认证有效、请求后已被明确拒绝；`already_invalid` 表示前置核验已明确失效，无需发出退出请求；`no_stored_session` 只表示服务器别名不存在，不证明其他凭据已失效。不会以 HTTP 200、退出响应内容或 Set-Cookie 删除作为成功证据，也不保存或交付过程中的新 Cookie。`revocation_request_started` 表示已进入发送步骤，不保证上游实际收到；`removed` 和 `caller_credential_discard_required` 分别表示本地删除确认和调用方丢弃要求。

整次操作限 45 秒，身份响应限 1 MiB，禁止自动重发退出请求。发送后仍能认证或无法完成回读时返回错误，`details.upstream_outcome=unconfirmed`，并报告本地清理状态、是否确认删除及调用方丢弃要求；`retryable=false`。前置身份未知且尚未发送时保留本地来源。已经发送后会清理原登录；清理只通过同代际条件更新，保留其他账号和新登录。取消会触发同样的有界清理尝试，但存储故障时不能保证删除，必要时仍可调用本地注销；取消也不能撤回上游可能已收到的请求。任何结果都不返回凭证更新，所有响应为 `no-store`。上述失效证据仅针对选定 Cookie，实际其他设备或会话的影响范围、真实账户退出效果仍待最终验收。

咪咕刷新依次检查 PACM 和核验同一账户 UID；`extensions.refresh_method` 为 `pacm_check_and_identity_verification`。对于已有身份绑定的会话，检查成功后会立即接受该步骤的轮换；后续资料请求只有业务成功且 UID 相同，才接受它返回的更新。昵称等展示字段损坏不会撤销此前已确认的凭证更新。普通账户资料读取使用相同的身份核验规则。

咪咕刷新后续步骤超时、响应损坏或发生普通业务错误时，服务器保留已确认的最新凭证；`client` / `both` 在错误响应的 `meta.caller_credential` 和 `X-TuneWeave-Updated-Credential` 中交付同一份更新，客户端应先保存它，再处理原业务错误。SDK 调用方通过 `TuneWeaveError::take_caller_credential_update()` 取出更新；原始凭证不会进入公开错误详情或日志。`server` 模式不返回调用方凭证，`client` 不修改服务器账户。没有已确认步骤的失败不会接收失败响应中的 token；首次导入仍须完整身份核验成功后才能保存或交付凭证。

咪咕刷新遇到明确会话失效或 UID 不一致时，不交付更新，并按精确快照删除所选服务器会话；并发重登、轮换或注销导致快照变化时返回 `conflict`，保留并发操作的结果。`both` 必须对应同一次登录，使用该服务器别名最新 PACM，并使交付值与保存值一致。刷新不承诺延长有效期或恢复已失效会话；所有刷新响应均为 `no-store`。

咪咕注销会请求官方清理所选 PACM；平台确认清理或明确已未登录后，才删除所选服务器别名。网络或未知业务失败会保留本地凭证并返回错误，不自动重发。调用方模式要求丢弃所提交凭证；官方清理可能使持有同一 PACM 的其他副本失效。迟到的读取、刷新或注销响应不能覆盖或删除重新登录后的服务器凭证；发生竞争时返回 `conflict`。真实账户全流程仍待最终验收。

汽水会话查询和刷新在处理上游成功或失败结果前，均核对原账户状态；迟到结果返回 `conflict`，不能覆盖重新登录的账户。`both` 刷新要求调用方与服务器凭据属于同一次登录，并使用服务器当前 Cookie；返回的新凭据可同步调用方副本。认证路由（公开国家区号目录除外）的参数、请求 ID 和方法错误也设置 `no-store`。

## 安全要求

- 对外部署时使用 HTTPS。
- 不记录 `X-TuneWeave-Credential`、Cookie、密码、手机号、验证码、二维码事务数据或平台 token。
- 不向浏览器开放通配 CORS 凭证请求。
- 调用方凭证应存放在操作系统安全存储或同等保护的私有区域。
- 二维码、验证码和密码只用于当前登录事务，不应进入 Uni Playlist 或业务日志。


### 咪咕账户歌单目录

以下入口使用选中账户的音乐会话，支持服务器别名或调用方凭据：

- `GET /v1/account/playlists?platform=migu`：按自建、收藏顺序合并两个目录。
- `GET /v1/users/migu:{uid}/playlists/created`：本人自建歌单。
- `GET /v1/users/migu:{uid}/favorites/playlists`：本人收藏歌单。

用户入口只允许选中会话的 UID，不能用其他用户 ID 查询私有目录；缺少凭据不会回退到匿名请求。`limit` 为 1–100，`offset` 为统一列表偏移。每次调用先完整读取所需目录，再应用窗口；每类最多 64 个物理页、每页 20 条。官方分页以短页结束，有明确总数或继续标记时还需相互一致；重复歌单、页间总数变化、提前结束及超过请求预算均明确失败。两类目录中出现同一 ID 时保留两条记录及各自的 `extensions.library_section`。`source_user_id` 是来源账户，`owner_id` 仅在上游提供时输出；收藏歌单的作者可以属于其他用户，未知可见性与创建时间不推测。

每个目录请求前沿用上一已核验的 PACM，返回的 token 需再次核验同一 UID 后才保存或交付。后页失败仍保留前面已核验的更新；认证失效或并发重登／注销冲突不交付旧凭据。三个入口均支持更新凭据响应头和 `meta.caller_credential`，账户响应为 `no-store`。

完整读取需要每个物理页后再做一次身份核验，大型目录有相应网络成本。当前官方目录调用没有提供可确认的不可变版本，完整分页不代表事务快照；目录本身不提供不可变版本；普通歌单与喜欢集合的内容导入见下一节。单曲已购记录见下文；歌单写操作见下文。代码与离线协议测试不替代真实账户验收。


### 咪咕账户歌单内容与喜欢列表

普通 `GET /v1/playlists/migu:{id}` 与 `/tracks` 现支持服务器 `account` 或调用方凭据；未指定账户来源时仍使用公开链。喜欢集合可通过 `/v1/account/favorites/playlist?platform=migu`、`/tracks?platform=migu` 读取，也支持 `/v1/users/migu:{uid}/favorites/playlist` 和 `/tracks`。用户入口只允许选中账户本人。喜欢集合采用平台个人导航给出的实际歌单 ID，并核验所有者，不从自建歌单的同名标题推断。

账户详情和曲目入口均完整读取所选歌单再返回详情或应用 `limit`（1–100）与 `offset`。每个物理页请求 50 条，最多 200 页、10,000 首。明确曲目总数、页间发布时间和所有者必须一致，读取前后再次核对歌单详情；喜欢集合还会再次核对导航 ID。缺少完整性字段、提前空页／短页、整页重复或元数据变化会返回错误，不返回截断结果。普通重复歌曲保留原位置；无法区分异常重放的整页重复会明确拒绝。收藏歌单可属于其他作者，喜欢集合必须属于当前用户。

响应的 `source_snapshot_id` 是所读元数据与有序歌曲身份的内容标识。它不包含凭据，不证明上游提供了事务快照。普通歌单和 `favorite_tracks` 来源已接入服务端 Uni 导入与客户端 materialize；不同完整读取的标识不一致时，不拼接创建集合。导入保留顺序和重复条目，不把来源凭据写入导出文档。账户播放／下载权益尚未接入，导入成功不表示取得歌曲播放权。

每个上游业务请求后均核验同一账户 UID，再接受其凭据轮换。六个普通／喜欢读取入口在成功或后续普通错误时返回已核验的新凭据，并遵循 `no-store`；认证失效、重登或注销竞争抑制旧凭据。真实普通／付费账户及私有可见性仍待最终验收。

咪咕喜欢歌曲写入使用 `PUT /v1/account/favorites/tracks/migu:{contentId}`，取消喜欢使用同一路径的 `DELETE`；通过 `account` 选择服务器账户，或提供调用方凭据。每次调用只发送一次写入，随后完整读取实际喜欢歌单，并核对平台返回的单曲喜欢状态。集合身份变化、读取不完整、状态缺失或两处结果不一致均不能确认成功；列表中缺少歌曲本身不能证明已经取消喜欢。

写入可能已经发生、但结果未能确认时，错误携带 `details.write_outcome: "unconfirmed"` 和 `retryable: false`，不要自动重发。整个操作沿用同一次登录，写后读取不能切换账户；已核验的凭据更新仍可随成功或普通失败返回，认证失效或代际冲突时抑制更新。操作响应遵循 `no-store`。真实账户写后状态仍须最终验收；歌单增删改、歌单收藏和其他音乐库写入不由此接口代替。


咪咕歌单收藏使用 `PUT /v1/account/favorites/playlists/migu:{playlistId}`，取消收藏使用同一路径的 `DELETE`。支持服务器 `account` 或调用方凭据。收藏时读取真实歌单标题，取消时不依赖歌单详情仍可访问。每次只写入一次，再完整遍历当前账户的收藏目录，并核对独立收藏状态；即使目标在第一页已经出现，也要读完剩余页。缺失、未知或矛盾的状态不能确认成功，普通专辑及数字专辑不会当作歌单处理。

上述歌单写后未确认错误同样带 `write_outcome: "unconfirmed"`、`retryable: false`；已核验的调用方凭据更新按既有规则交付，响应为 `no-store`。平台的收藏和取消虽然采用 GET 传输，仍是写入，TuneWeave 不自动重放；真实账户写后结果仍待最终验收。歌单创建、修改、删除和曲目增删为独立能力。


### 咪咕本人歌单管理

已接入以下账户写操作，支持服务器 `account` 或调用方凭据：

| 方法 | 端点 | 行为 |
| --- | --- | --- |
| POST | `/v1/playlists` | 创建普通歌单，指定 `platform=migu`、`name`、`visibility=platform_default` |
| PATCH | `/v1/playlists/migu:{id}` | 修改名称、非空简介和／或标签顺序；见下文 |
| DELETE | `/v1/playlists/migu:{id}` | 删除本人普通歌单 |
| DELETE | `/v1/playlists` | 按 `refs` 顺序删除 1–100 个不同的咪咕歌单 |
| POST / DELETE | `/v1/playlists/migu:{id}/tracks` | 按 `refs` 增加／移除 1–100 个不同的咪咕歌曲引用 |
| POST / DELETE | `/v1/playlists/migu:{id}/items` | 同上，仅支持 `kind=track` |

`platform_default` 表示使用平台自身的可见性设置，不承诺公开或私密。现有通用请求省略可见性时仍为 `public`；咪咕创建必须显式选择 `platform_default`，`kind` 仅支持默认的 `normal`。当前官方创建协议没有已确认的公开／私密控制字段，因此其他可见性和歌单类型在发出请求前拒绝。创建成功还需完整读取前后自建目录，识别唯一新 ID，核验本人所有权、名称和空歌单；无法唯一确认时返回未确认错误。

改名、简介、标签、删除和曲目操作只接受选中账户自建目录内的普通歌单，并核验详情中的所有者。平台实际喜欢歌单受保护，不能通过这些入口编辑、删除或修改曲目；喜欢操作使用专门的歌曲收藏入口。仅提交 `name` 时使用原有 PC 改名接口，`variant` 为默认或 `individual`。含 `description` 或 `tags` 的请求使用原生资料接口；默认或 `batch` 可组合字段，`individual` 必须恰好提供一个资料字段。

提交 `description` 时使用原生简介更新接口，支持默认或 `batch` 下同时修改名称，`individual` 只接受简介单字段。简介最多 4000 UTF-8 字节，允许换行、回车和制表符，拒绝空白简介及其他控制字符；清空简介尚未支持。更新先通过 H5 交换获得临时原生凭据并核验同 UID，临时凭据不保存或导出；整个操作限 90 秒。

`tags` 可提交完整目标顺序。实现保留目标与现有标签的最长有序前缀，再按原顺序逐个删除其余标签，最后按目标顺序逐个添加后缀；重加标签必须仍存在于官方目录且 ID 与原身份相符。包含添加时最多六个标签，每次请求最多执行 16 个已逐步确认的变更。每一步都完整回读确认标签、简介、名称、曲目顺序和重复项，并核对 UID 与登录代际。该过程不是原子写入；中途失败可能留下部分调整，错误会报告已确认的移除和添加，不自动重试或回滚。收到平台成功码后仍无法确认的写入返回 `write_outcome=unconfirmed`。歌单隐私、曲目完整重排、封面和其他未列出的写方法尚未接入，真实账户写入待最终验收。

曲目操作先后完整读取歌单，保留原有条目的相对顺序及重复位置。增加已存在歌曲不要求再插入一个副本；每个新 ID 必须恰好出现一次，新增歌曲的位置以平台实际结果为准。移除作用于选中 ID 的所有重复位置；请求中的重复引用会明确拒绝。仅有平台成功码不足以确认完成，部分添加、其他曲目意外变化、分页不完整或身份变化均返回错误。

批量删除先验证整批资源和所有者，再逐项写入并完整回读目录。它不是原子操作；中途失败时，`error.details` 提供 `atomic=false`、`completed_refs`、`failed_ref`、`remaining_refs`。已经发送写入后的错误均带 `write_outcome=unconfirmed`、`retryable=false`，不会自动重发。成功及失败均按统一契约回传已验证的调用方凭据更新并设置 `no-store`；认证失效或并发换号不会交付旧凭据。

完整目录和歌曲回读有分页预算及多次身份核验成本，不表示平台提供了事务快照。上述协议仍需最终真实账户验收。

### 咪咕普通与数字专辑收藏

普通专辑列表使用 `GET /v1/account/library/albums?platform=migu`，数字专辑列表使用 `GET /v1/account/library/digital-albums?platform=migu`。本人用户入口分别是 `GET /v1/users/migu:{uid}/favorites/albums` 与 `/favorites/digital-albums`，其中 UID 必须属于所选账户。使用 `account` 选择服务器别名，或省略别名并提交调用方凭据，两种来源不能混合。

普通专辑使用 `albumId` 和 `resource_type=2003`，数字专辑使用 `contentId` 和 `resource_type=5`，返回的 `Album` / `DigitalAlbum` 不能互换。即使两个类型的数字 ID 恰好相同，也仍是两个独立收藏。目录保留实际标题、歌手与可用封面；曲目数、价格、已购买状态等没有可靠字段时保持未知，收藏不构成购买或播放授权。

两类读取均先完整遍历同一份混合收藏目录，再筛选类型并应用 `offset` / `limit`（1–100）。最多读取 64 个上游页，每页 10 个原始项目；短页或一致的明确结束信息才结束，重复身份、总数矛盾或预算耗尽返回错误。分页 `total` 是当前类型的数量，`extensions.upstream_raw_collection_count` 是混合目录的总数。即使当前类型暂时没有项目，也继续读取其余原始页；大集合的每次窗口读取均有完整遍历成本。

单项收藏／取消分别使用 `PUT` / `DELETE /v1/account/library/albums/migu:{albumId}` 或 `/v1/account/library/digital-albums/migu:{contentId}`。批量使用相同方法但省略末尾引用，JSON 例如 `{"refs":["migu:77","migu:78"],"account":"personal"}`，要求 1–100 个不同 ID。请求路径确定整批资源类型；数字专辑的销售条目 ID 或关联普通专辑 ID 不能代替其 `contentId`。

收藏前通过所选账户读取对应类型的实际专辑标题；取消不依赖专辑详情仍可访问。每项只发送一次写入，之后完整回读混合目录，并核对该资源类型的独立收藏状态，两者一致才返回成功。列表中缺席本身不能证明取消成功。写入后的网络或确认错误带 `write_outcome=unconfirmed` 和 `retryable=false`，不自动重发。

批量按输入顺序逐项确认，中途失败提供 `atomic=false`、`resource_type`、`completed_refs`、`failed_ref`、`failed_write_dispatched` 和 `remaining_refs`。`failed_write_dispatched=false` 表示失败项尚未发出写请求，前面已确认的结果仍保留；整批不具备原子回滚。每一步返回的新凭据都先核验同一 UID，并防止迟到响应覆盖重新登录或注销的结果。成功及普通失败通过统一 JSON 元数据和更新头返回已接受的调用方凭据；认证失效、会话冲突不返回旧凭据，所有账户响应均为 `no-store`。真实账户目录与写后可见性仍待最终验收。


### 酷狗底层二维码 SDK

`KugouClient::create_login_qr(KugouLoginClient::{Standard, Concept, Web})` 创建内存中的 `KugouQrSession`。`url()` 返回可编码为二维码的官方 URL；`poll()` 返回等待扫码、等待确认、过期或 `AuthorizationReceived`。每个事务使用独立设备快照，所有克隆共享轮询节流和一次性交付状态，`cancel()` 会使克隆及尚未完成的请求失效。创建和轮询不携带账户凭据，不修改持久化匿名设备，也不访问上游提供的图片或重定向地址。

轮询至少间隔两秒，HTTP 429 的 `Retry-After` 秒值会延长共享等待时间（最多到本地事务预算）；事务使用本地五分钟预算，`expires_in()` 返回本地剩余时间，不代表平台承诺的有效期。平台过期、取消和已交付授权均为终态。请求失败不会自动重发；调用方应遵守节流后显式决定是否再次轮询。

`KugouQrAuthorization` 是未核验的授权材料，只交付一次，包含明确的客户端类别、UID、Token 和本次设备快照；它不能直接作为 `ProviderCredential`、会员信息或播放权限。`token()` 及设备字段属于敏感材料，不应记录或公开。

普通版和概念版可以把授权交给 `KugouClient::complete_qr_login(authorization)`：SDK 先通过固定 HTTPS 网关交换令牌，要求交换响应实际返回的 UID 与二维码一致，再以更新后的令牌读取本人资料。个人资料可能省略 UID；仅在此前交换已核验身份时采用该 UID，资料中出现的身份仍须一致。两步都成功后返回 `ProviderAuthResult`，其中账户别名为 `default`，凭证由调用方保管，不写入服务器账户或匿名设备。普通 Token、VIP Token 与设备状态分别保存，不据此推断会员或单曲播放权限。

`refresh_native_login(&credential)` 显式交换原生凭证并回读资料，保留本次登录代际。调用方必须保存返回的新凭证；如果交换成功而后续资料读取发生普通错误，使用错误的 `take_caller_credential_update()` 取出并保存已接受的更新。认证失效或身份冲突不返回更新。首次登录后续失败也不交付半成品凭证。网络失败不自动重试，额外验证要求会明确失败。

Web 授权由 `complete_qr_login` 或 `complete_web_qr_login` 分派到官网 Cookie 交换：使用独立 Web 公钥和 16 字符随机种子，通过固定 HTTPS 接口提交加密令牌；业务成功后必须收到新的 `KuGoo` Cookie，其中 UID 与二维码相同，且包含有效 Web Token。资料来自此次服务器签发的 Cookie，不读取任意公开用户资料作为认证证据。`refresh_web_login` 使用所选 Web 会话再次交换，要求新的同 UID Cookie，保留登录代际。缺少 Cookie、重复身份或域／路径不符均失败，实际 Cookie 到期后不再发送请求；请求沿用官网默认的一天登录偏好，实际有效期仍以响应 Cookie 为准；没有平台到期信息时不推算有效期。Web 使用独立的 `kugou_web_v1` 凭证，原生 `kugou_native_v1` 格式保持兼容。

Web 二维码、交换及后续会话沿用同一十六进制 MID；它与原生十进制 MID 分开。只保存当前登录 Cookie，不使用其他账户的 Cookie，不执行上游脚本或跳转。真实扫码、Cookie 成功响应形态及账户协议仍待最终验收；概念版桌面标识的实际接受情况也尚待验收。

### 酷狗账户与统一 HTTP 登录

`POST /v1/auth/qr` 的 `platform=kugou` 支持 `login_type=standard`（默认）、`concept` 和 `web`。使用已有二维码轮询入口完成登录，支持 `server`、`client`、`both` 三种归属。`server` 和 `both` 需要配置 `KugouConfig.credential_store`，服务器默认启动已接入共享账户仓库。`client` 使用 `default`，凭证仅交给调用方；`both` 返回与服务器保存内容完全相同的凭证。其他尚未实现的登录方式明确拒绝。

Provider 事务在创建时固定客户端和凭证归属，首次轮询时固定账号及旧凭证快照；之后不能换账号或归属。完成时要求原事务仍有效，并对旧快照执行原子替换，首次登录则仅在账号仍不存在时保存。事务容量为 128，创建中的请求也占容量；并发轮询限流。`KugouProvider::cancel_qr_login` 接受 provider 事务 ID，可取消尚未完成的登录。同一 provider 及其克隆的账号注销、新登录会使该账号已绑定的其他事务失效；取消、到期或任务中断后不交付迟到授权。

`GET /v1/auth/session`、`GET /v1/account` 和本人资料入口支持精确服务器账号或调用方凭证；只实现 `modern` 资料视图。原生会话每次读取先执行令牌交换核验 UID，再读取本人资料；Web 会话通过重新交换后签发的同 UID Cookie 核验身份与资料。读取产生的新凭证遵循统一返回契约；服务器模式只更新对应账号，调用方模式不查找或写入服务器账号。原生账户已接个人歌单、已购库、音频播放与独立下载、视频播放及歌词；Web 账户已接音频播放和普通歌词，但不提供独立下载授权。其他目录或媒体操作按其实际支持范围明确拒绝，不会静默切换匿名来源。

`POST /v1/auth/session/refresh` 支持三种归属，`both` 可以用同次登录的调用方凭证同步最新服务器令牌，其他登录代际、UID、客户端或设备不能混用。已核验的轮换先按原子快照保存；后续普通资料错误仍保留已接受的更新，认证失效和并发冲突不交付凭证。所有成功与迟到错误均复查当前代际，防止恢复已注销或覆盖已重新登录的账号。

`DELETE /v1/auth/session` 清理所选的本地凭证归属。调用方及 `both` 注销需要提供对应凭证，并返回 `caller_credential_discard_required`；它不表示上游令牌已被撤销。真实登录、账户身份、概念版设备兼容性、会员、个人音乐库读写及权益播放仍需真实账户验收；尚未实现的能力继续保留明确边界。


### 酷狗 Web 密码登录

`POST /v1/auth/password` 的 `platform=kugou` 默认使用 Web 登录，也可显式设置 `backend=web`。支持 `principal_type=username|email|phone`、`password_format=plain` 及 `server|client|both`。数字酷狗 ID 可使用 `username`；手机号码使用规范的 11 位国内号码，国家码可省略或填 `86`／`+86`。SDK 同时提供 `KugouClient::login_web_password`，只返回 `default` 调用方凭据；服务器别名由 Provider 层管理。

该路径按当前官方网页把原文密码放入 AES/RSA 加密参数，不做 MD5。MD5 输入、额外 `secure_captcha`、密码或账号首尾空白及控制字符会在联网前拒绝，避免静默改写输入。手机号在查询参数中遮盖，完整号码只在加密参数内；普通用户名和邮箱按官方协议进入 HTTPS 查询参数，但日志不记录查询或请求体。

响应中的 UID 必须与新签发的 Web Cookie 一致，才创建新的登录代际并按归属保存。密码不写入凭据或事务；密码请求与二维码共用 128 个事务容量，取消、到期、新登录及本地账号注销后不接受迟到结果。其他实例的首次登录或已有账号更新仍使用原子条件写入，失败不交付未保存的凭据。

普通 Web 密码登录及二次手机验证可使用统一 `POST /v1/auth/password`（`backend=web` 或省略）接续。平台明确要求验证绑定手机号且提供完整号码时，发送一次短信并返回 `state=verification_required`、`transaction_id` 和 `verification.method=sms`。号码由平台决定，响应只展示脱敏的 `masked_destination`；不能指定其他手机号。此流程不会自动创建账号或修改绑定。单次 SDK `login_web_password`／Provider `password_login_with_mode` 仍只处理普通成功，需要续接时使用 `begin_password_login`／`advance_password_login`。

向同一 `POST /v1/auth/password/challenges/{transaction_id}/verify` 提交 `{"action":"submit_sms","code":"<短信验证码>"}`。需要重发时使用 `{"action":"resend_sms"}`，冷却 60 秒，含首次发送最多 5 次；与独立 Web 短信入口共享同手机号冷却，新发码会使该号码之前的待验证事务失效。所有回答共用最初登录起的 5 分钟期限和 5 次预算，重发不重置。

若短信验证要求浏览器交互，返回 `verification.method=sms_browser`，其 `verification` 对象包含 `verification_id`、`url`、`message_origin`、`message_type`、`response_field` 和 `remaining_attempts`。按[Web 短信浏览器说明](#酷狗短信登录与账户选择)校验官方 iframe 的 origin、窗口和消息类型，再向原密码续接入口提交：

```json
{"action":"submit_sms_browser","verification_id":"<当前标识>","code":"<重新提供的短信验证码>","response":"<官方 dataJson 原始字符串>"}
```

该动作不接受密码；它与原生密码流程的 `submit_browser` 使用不同回执协议。平台要求选择账号时，返回 `verification.method=account_selection`；只接受用户明确选择的候选 UID，提交 `{"action":"select_account","user_id":"<候选UID>","code":"<重新提供的短信验证码>"}`。浏览器验证也可能出现在账号选择之后，始终按最新状态接续同一事务。密码、短信验证码和浏览器回执不保存在待验证上下文；账号、后端、设备与凭据归属保持最初绑定，独立身份核验和条件保存成功后才交付凭据。

Web 初始密码请求的图形／交互验证、绑定及换绑仍返回明确的验证错误，尚未提供对应续接；二次短信浏览器支持不代表这些入口已经完成。取消、过期、注销、换号或不确定网络失败后需重新登录。当前验证来自本地协议及 HTTP 契约测试，真实账户、短信与调用方浏览器集成仍待最终验收。


### 酷狗原生密码登录

`KugouClient::login_native_password` 提供标准版原生密码登录，接受与 Web SDK 相同的明文密码、用户名、邮箱或国内手机号输入，仅返回 `default` 调用方的原生凭据，不保存服务器账号。请求按 Standard Android 20.8.0（20809）生成设备参数及关联时间戳，解密本次请求对应的登录回执后，还需通过认证资料读取才返回凭据。设备环境明确标识为 TuneWeave，不读取本机 Android ID 或 IMEI。

统一入口 `POST /v1/auth/password` 设置 `platform=kugou`、`backend=native` 即使用该原生流程，支持 `server|client|both`。`client` 请求省略 `account`，使用 `default` 且不读写服务器账号；`server` 和 `both` 需要配置账户存储，使用精确账号别名。认证资料核验通过后才保存新凭据，`both` 返回与保存内容完全一致的凭据。可以显式将同一服务器别名从 Web 登录切换到原生登录；失败保留原账户，注销或新登录后不接受迟到结果。

```json
{"platform":"kugou","backend":"native","principal_type":"phone","principal":"<phone>","password":"<password>","credential_mode":"client"}
```

平台要求原生图形验证时，统一入口返回 `state=verification_required`、`transaction_id` 及 `verification.method=image`；`verification.image.answer_kind` 为 `alphanumeric`。此时不保存或交付凭据，原账户保持不变。展示 `verification.image.image_data_url` 后，在同一事务中提交用户读出的验证码和重新输入的密码：

```http
POST /v1/auth/password/challenges/{transaction_id}/verify
Content-Type: application/json

{"action":"submit_image","answer":"<图中字符>","password":"<原账户密码>"}
```

需要换图时提交 `{"action":"refresh_image"}`，遵守 `refresh_after_secs`。回答最多 5 次、主动换图最多 5 次，主动换图间隔至少 2 秒；换图不重置回答次数。平台明确要求重新图形验证时返回更新后的图片，保留同一事务。答案为 1–64 个 ASCII 字母或数字，不自动识别或改写。请求中的账户、原生后端、凭据归属和设备身份在最初登录时固定；续接不能替换这些字段，事务不保存密码。事务从最初登录起最多 5 分钟；过期、注销、新登录、取消请求或不确定的网络失败后需要重新登录。验证成功后仍须独立资料核验和账户条件保存，才返回登录结果。

需要浏览器验证时，返回同一事务及 `verification.method=browser`、`protocol=kugou_native_bridge`、`verification_id`、`url`、`device_id`、`client_version` 和 `remaining_attempts`。`url` 是酷狗官方验证页，包含本轮验证参数；页面可能显示旧版腾讯、新版腾讯或极验交互。不要记录或缓存该地址。浏览器验证优先于同时返回的图片。

调用方在独立 WebView 打开该页，并为当前验证页实现 `external.superCall` 的三个命令。密码、账户凭据和应用的其他原生能力不得暴露给页面；只接受当前事务对应的页面窗口及官方 origin/path，导航离开后撤销桥接：

| 命令 | 桥接行为 |
| --- | --- |
| `122` | 返回 JSON 字符串 `{"version": <client_version>}`。 |
| `124` | 返回 JSON 字符串 `{"mid": "<device_id>"}`，使用本次响应中的设备身份，必须保留字符串，不能转为 JavaScript 数字。 |
| `252` | 接收第二参数中的原始 JSON 字符串；`close=0` 且有非空 `ticket` 才能提交，`close=1` 表示取消。 |

该页使用原生桥接，并不提供 Web 短信验证页的 `postMessage` 接口。普通浏览器直接打开链接不足以完成回执交付。人工完成后，调用方将 `252` 的 JSON 原样作为字符串 `response`，与当前 `verification_id` 和重新输入的密码一起提交；不要 URL 解码或重新拼接 `ticket`，也不要单独提取新版腾讯或极验的内部字段：

```json
{"action":"submit_browser","verification_id":"<本轮验证标识>","response":"{\"close\":0,\"ticket\":\"<官方页面回执>\"}","password":"<原账户密码>"}
```

提交至同一 `POST /v1/auth/password/challenges/{transaction_id}/verify`。若平台再次要求浏览器验证，以新的 `verification_id` 和页面重新交互，旧标识无效；也可能切换为图形验证，应按最新 `method` 展示。图形和浏览器回答共用 5 次限额和起始 5 分钟期限，不因切换或换图重置。浏览器阶段不接受 `refresh_image`；取消时关闭页面，不提交回执，需要继续时重新发起登录。

Rust 使用 `begin_password_login` 和 `advance_password_login` 处理上述续接；单次 `password_login_with_mode` 和底层 `KugouClient::login_native_password` 仍在需要附加验证时返回错误。用户名或密码错误不会自动取图；二次手机验证续接见下文，绑定或换绑手机仍未接通。错误不回显平台原文或临时验证令牌。接口及回执传输使用自动化测试核验；调用方 WebView 集成、真实账户登录与验证码成功流程仍待最终验收。

原生密码登录要求二次手机验证时，同一事务返回 `verification.method=sms`。完整手机号只保留在原事务中；`masked_destination` 返回脱敏号码，平台只提供账号目标时显示“账号绑定手机”。使用同一密码续接入口提交 `{"action":"submit_sms","code":"<短信验证码>"}`，需要重发时提交 `{"action":"resend_sms"}`。此阶段不再提交密码，不允许指定新手机号。原生短信重发冷却 31 秒，同一事务最多发送 5 次，所有验证回答共享 5 次限额和起始 5 分钟期限。

短信验证后若平台要求选账号，返回 `verification.method=account_selection` 及 `accounts`，候选仅含 `user_id`、可选昵称和头像。客户端让用户明确选择，再提交 `{"action":"select_account","user_id":"<候选UID>","code":"<重新提交的短信验证码>"}`；验证码不会保存在服务端事务中，不能自动选择第一项。直到所选 UID、登录回执与独立账户资料一致并完成原归属模式的凭据提交，才返回登录成功。绑定/换绑手机号、恢复账号与实名确认不属于这条续接流程；真实账号及短信验收仍待完成。

### 酷狗短信登录与账户选择

使用既有的 `POST /v1/auth/challenges` 创建事务，设置 `platform: "kugou"`、`method: "sms"`、`backend: "standard"` 和中国大陆 11 位 `principal`；国家码可省略或为 `86`／`+86`。支持 `server`／`client`／`both`；服务器模式需要凭据仓库，客户端模式仅使用 `default`，不读取服务器账户。

`allow_account_creation` 默认为 `false`，允许现有账号登录。只有创建事务时明确传入 `true`，才会在平台要求确认新账号时继续一次注册确认；不会因其他错误自动重试。该许可固定在原事务中，验证时不能修改，也不代表已获得登录或播放权限。

发送成功返回 `state: "waiting"` 和 `transaction_id`。向 `POST /v1/auth/challenges/{transaction_id}/verify` 提交 `{"code":"<4至8位数字验证码>"}`，也支持 `{"action":"submit_code","code":"<验证码>"}`。同一手机号绑定多个账号时，返回：

```json
{
  "state": "account_selection_required",
  "accounts": [
    {"user_id": "222", "nickname": "示例用户", "avatar_url": null}
  ]
}
```

以上为响应中的 `data`。候选账号仅供选择，不包含登录凭据或已认证资料。调用方必须让用户明确选择，再向原验证入口提交 `{"action":"select_account","user_id":"222","code":"<原短信验证码>"}`；两个字段均为字符串。服务端不保存验证码，因此选择时必须再次提供。只能选择原事务返回的 UID，不自动选择第一项。错误动作或未提供的 UID 在联网前拒绝，并保留有效的原事务。

短信登录接口明确返回业务码 `20028` 时，原事务返回 `state: "browser_verification_required"`，`verification` 包含：

- `verification_id`：本次人工验证的标识，只能提交到原短信事务。
- `url`：绑定原事件、App ID 和设备 MID 的酷狗官方人工验证页；客户端以独立 iframe 打开，让用户完成验证。
- `message_origin: "https://h5.kugou.com"`、`message_type: "kgVerifyCallbackData"`、`response_field: "dataJson"`：官方页面回调的来源、类型和载荷字段。
- `remaining_attempts`：原短信事务剩余的验证码动作次数。

接收 `message` 时，必须同时核对 `event.origin === verification.message_origin`、`event.source === iframe.contentWindow`，以及 `event.data.type === verification.message_type`；只接收当前事务对应 iframe 的回调。官方页面可能读取浏览器现有酷狗会话，因此应使用与本次登录相符的浏览器环境。不要把回调状态视为登录成功，也不要保存回执、页面 URL 或短信验证码到日志、Cookie 或持久化存储。

将 `event.data.dataJson` 的原始 JSON **字符串**作为 `response`，不预先解码其中的 `verify_data`，向原验证入口提交：

```json
{
  "action": "submit_browser",
  "verification_id": "<本次 verification_id>",
  "code": "<再次提供原短信验证码>",
  "response": "<官方 dataJson 字符串>"
}
```

SDK 严格校验成功回调格式，将 `verify_data` 按 `decodeURIComponent` 语义解码一次，并只向原短信登录接口提交 `VerifyData` 请求头；不加入签名正文，不发送到候选查询、Cookie 交换或其他服务。所选 UID 和创建事务时的注册许可不能通过此动作修改。再次要求人工验证会生成新标识，旧标识失效；错误验证码回到原等待／账户选择状态，丢弃人工回执，需按新状态继续。格式错误、错误标识或失败回调在联网前拒绝，保留有效事务。回调本身不创建登录凭据。

成功的短信响应仍需独立交换并核验新的同 UID Web Cookie；只有完成核验及服务器条件保存后才返回 `state: "confirmed"`。`server` 保存所选别名，`client` 返回调用方凭据，`both` 返回与保存内容一致的凭据。候选 Cookie、手机号、验证码和平台原始附加字段不交付调用方。

事务从创建起最多保留 5 分钟，发送和每次验证各有 45 秒总等待上限，验证不超过原事务期限。一个事务最多执行 5 次验证码动作，选择账号与提交浏览器回执也各计一次；明确错误验证码可在剩余次数和期限内重试，其他不确定错误、身份不符或尚不支持的附加验证会消费事务。人工验证不会延长原期限或重置次数。取消、超时、注销或重新登录后不能交付迟到结果。二维码、密码和短信共用 Provider 的 128 个待处理名额。同一 Provider 及其克隆对相同手机号从发送开始实施 60 秒冷却，发送失败或取消仍保留冷却，发送接口返回更长的 `Retry-After` 时按有界等待时间延长；最多保留 1024 个未到期号码，验证进行中不能重发，冷却结束后的显式新发送使该号码旧事务失效。独立 Provider 实例不共享此内存节流。

Rust 调用 `begin_auth_challenge`、`auth_challenge_status` 和 `advance_auth_challenge`，在 `Pending(AccountSelectionRequired)` 或 `Pending(BrowserVerificationRequired)` 时保留原收据。旧 `complete_auth_challenge` 遇到这些状态返回可接续错误，不自动完成验证或选择。`cancel_sms_login` 可取消精确的 Provider 收据。底层 `send_web_login_sms`／`verify_web_login_sms`／`advance_web_login_sms` 使用不可序列化的 `KugouWebSmsChallenge`，直接 SDK 调用方自行管理跨请求节流；底层不读写服务器账号。

HTTP 创建、人工验证、选择和完成的成功与失败响应均为 `no-store`。本接续仅适用于短信登录接口的业务码 `20028`；发送短信、查询候选时的附加验证、`SSA-CODE` 响应头挑战及密码图形验证仍未接入，不自动套用该回执。真实短信发送、人工验证、注册、选择账号和登录均留待最终真实账户验收。

### 酷狗会员资料

`GET /v1/account/membership?platform=kugou` 和 `GET /v1/users/kugou:{id}/membership` 支持默认账户、具名 `account` 或调用方凭证。指定用户 ID 必须属于选中的账户，否则在联网前拒绝。Rust 使用 `user_membership` 或 `user_membership_client_info`；HTTP 的 `backend=front`／`client` 均按凭证所属客户端选择协议，不会把 Web Cookie 转换为原生登录。

普通版读取 VIP detail V3，概念版读取联合会员，Web 读取官网角色资料。原生请求先交换会话并核验本人资料，再核对会员响应 UID；概念版每个并行会员产品也核对 UID。Web 会员响应没有已核实的 UID，因此前后均独立验证同一账户；响应携带的新 Cookie 只有在作用域正确、未过期且后续身份核验通过后才保存。

`extensions.backend` 分别为 `standard_vip_detail_v3`、`concept_union_vip`、`web_roleinfo`。`summary_scope=main_membership` 表示通用 `active` 和 `expires_at` 对应主会员，不合并音乐包或概念版其他产品；主会员未激活时，某个并行产品仍可能有效。未知状态码返回 `active: null`。普通版的 `level` 只取明确的 `svip_level`，并用 `level_scope=super_membership` 标明所属类型，不能把会员类型码当作等级；其他客户端等级保持未知。

`membership_details` 保留经过类型校验的独立字段：普通版主会员、SVIP、音乐包及联合产品，概念版完整 `busi_vip` 产品列表，Web 的角色、主会员与音乐包日期。不会挑选最长日期替代其他产品，也不会导出上游任意 JSON、手机或令牌字段。日期按上游原文返回，`date_format=upstream_text`、`date_timezone=null`；不能把这些值当作已确定时区的 RFC 3339 时间，或据此推断单曲播放／下载权限。

整个读取过程限时 45 秒，会员正文最多 1 MiB。每次网络响应都核对原登录状态；注销、重登或并发换号后的迟到结果不交付。普通业务错误可通过既有更新头返回此前独立核验的调用方凭据，调用方应先处理更新再处理错误；认证失效、会话冲突、存储失败、取消和总超时不交付凭据更新。HTTP 成功及失败均为 `no-store`。会员查询不修改订阅或付款设置，三类真实账户及媒体权益仍待最终验收。

### 酷我登录验证码 SDK

`KuwoClient::create_login_challenge()` 可创建官网图形验证码；返回的 `KuwoLoginChallenge::image()` 仅包含内嵌 PNG、`answer_kind: "alphanumeric"` 和本地交互限制。调用方展示图片后可用 `validate_answer()` 检查 1–6 位 ASCII 字母或数字的输入格式；该方法不提交答案。`refresh_login_challenge()` 在同一独立登录会话内刷新，失败后旧图片与令牌作废。每个对象本地保留最多五分钟、刷新间隔至少两秒、最多刷新五次，这些限制不代表官方会话有效期。

此 SDK 提供官网验证码准备，不建立账户、不发送短信；该验证码尚未接入原生密码登录流程。它使用独立于公开音乐缓存的会话，平台验证码令牌和 Cookie 不进入公共结果或日志。

### 酷我原生会话 SDK

已有原生客户端会话可用 `KuwoNativeSessionInput::new(user_id, session_id, device_id, device_user)` 表示。后两项分别对应原客户端的应用设备 ID 和原生 `user` 设备标识，必须分别保留，不能用账户 UID 或彼此代替。该类型只检查输入格式，不能使用网页 Cookie 代替，也不代表已经认证。

`KuwoClient::exchange_native_session(&input)` 读取原生会话交换结果，核对返回的用户 ID，并保留可能更新的会话 ID。返回的 `KuwoNativeSessionExchange` 提供 `session()` 和可选的 `nickname()`；必须继续调用 `validate_native_session(exchange.session())`，独立核验这组用户与会话 ID。只有明确的业务校验成功才返回 `Ok(())`，HTTP 200 或其他数值字段不代表认证成功。

两步均使用固定的 HTTPS 入口，不发送密码、短信、网页 Cookie 或公开音乐凭据，不自动重试，不读写账户存储。交换结果不生成通用登录凭证；账户管理方还需在每次网络操作后核对原账户代际，并妥善处理交换后的会话更新。原生输入与交换结果不自动序列化，调试输出隐藏全部账户字段。统一账号入口在下文说明；底层会话交换不直接操作账号存储，个人库和真实账户成功流程仍待各自完成。

`KuwoNativeDeviceStore::default()` 在内存中创建独立安装身份；如需重启后复用，可使用 `KuwoNativeDeviceStore::new(Some(path))` 指定本地文件。`initialize(&client).await` 返回匿名的 `KuwoNativeDevice`，已有注册结果时直接复用。首次联网前保存两个独立随机安装标识，不采集硬件 ID；平台返回的应用设备 ID 与账户 UID 分开。该操作不登录账户，也不授予会员或播放权益。

持久化文件仅保存设备状态，同一路径的实例和协作进程通过文件锁串行初始化与刷新，等待锁最多 30 秒。失败或取消后保留原安装身份；损坏文件、未知格式以及文件路径上的符号链接会报错。文件更新采用临时文件和原子替换，并检查原状态，避免覆盖请求期间已被修改的设备数据。Unix 下文件权限为 `0600`。请保留相邻的 `.lock` 文件，不在运行期间移除或替换它。

`refresh(&client).await` 显式用原安装身份刷新应用设备 ID，失败保留旧状态，不自动重试。`registered_at_ms()` 是本地记录时间，不是平台承诺的过期时间。设备和存储的调试输出隐藏标识及路径。

在这个 SDK 安装身份下取得的会话，可以通过 `device.session_input(user_id, session_id)` 构造输入，后续交换与独立核验会保留同一设备上下文。来自其他安装的已有会话仍应使用其原设备字段；不能随意换成新注册设备来证明会话有效。底层设备注册已可匿名验证，真实会话成功仍待最终验收。

### 酷我原生密码登录 SDK

`KuwoClient::login_native_password(&request, &device)` 接收 `PasswordLoginRequest` 和已初始化的 `KuwoNativeDevice`。支持 `account="default"`、`principal_type="username"|"phone"|"email"`、`password_format="plain"`，后端为默认或 `native`；Web、MD5 或额外验证码参数在联网前拒绝。手机号限中国大陆 11 位 ASCII 数字，国家码可省略或为 `86`／`+86`，号码本身不带国家码；用户名和邮箱不接受国家码。邮箱须有一个 `@`、两端非空且无空白，保留大小写和 `+` 等原字符。

登录标识不接受首尾空白，最多 256 UTF-8 字节；密码最多 1024 字节，保留原有 UTF-8 字节和空格，不静默修剪。两者均拒绝控制字符。三种标识均使用原生密码登录，不切换为短信登录，不发送短信或自动注册账户。

登录请求按原生协议提交后，SDK 继续独立核验响应中的用户 ID 和会话 ID。两步都成功才返回 `ProviderAuthResult`、已认证的用户 ID、可用昵称及 `kuwo_native_v1` 调用方凭据。第二步失败或任务取消不交付半成品凭据。昵称、头像和会员并不由本地设备证明，未知字段保持未知；不推断单曲播放或下载权益。

SDK 不保存密码，不写服务器账户或匿名设备状态，不自动重试，也不发送短信。原生 `Cookies` 设备信息头与 Web `Cookie` 分开，初次登录的头中账户字段固定为零，不混入其他登录状态。敏感查询和返回字段不写日志。

`validate_native_login(&credential)` 对该凭据中的 UID/SID 再做独立网络核验，返回仅含已核验身份的 `AccountProfile`，不交换会话 ID，也不从调用方材料相信昵称或会员声明。凭据包含登录代际和对应安装上下文，格式检查本身不代表认证；没有平台有效期证据时不自行添加到期时间。

这两项提供直接 SDK 调用；统一 HTTP 账号入口和凭据归属见下文。图形或其他附加验证不自动续接，平台未知业务失败明确报错。真实用户名／手机号／邮箱密码登录、激活或绑定状态和成功响应仍待最终账户验收。


### 酷我账号与统一 HTTP 登录

`POST /v1/auth/password` 的 `platform=kuwo` 支持原生用户名／手机号／邮箱的 plain 密码流程和 Web 用户名图形验证码流程，以及 `server`、`client`、`both` 三种归属。`server` 和 `both` 使用精确服务器账号别名，需要 `KuwoConfig.credential_store`；`client` 仅允许 `default`，不读取或写入服务器账号。`both` 只返回与保存内容完全一致的凭据。服务器默认接入共享账号仓库，设备文件使用数据目录下的 `kuwo-device.json`；Provider 可通过 `KuwoConfig.device_path` 设置设备文件，未设置则使用内存设备状态。Web 密码接续见[酷我 Web 密码登录](#酷我-web-密码登录)。

登录事务固定账号、归属、旧凭据快照和一份安装身份。设备初始化、密码请求、独立会话校验每个网络步骤前后都会检查事务与账号来源；事务本地预算五分钟，创建中和进行中的登录/刷新合计最多 128 个。取消请求会释放事务，不保留密码或用户名。首次保存使用条件插入，替换使用旧快照的原子比较；存储冲突或失败不交付登录成功。相同 Provider 及其克隆的账号新登录或本地注销会取消该账号的未完成事务；不同 Provider 实例通过共享存储的条件写入防止覆盖其他提交。

`GET /v1/auth/session` 和 `GET /v1/account` 独立核验所选账户的 UID/SID，返回身份视图。`GET /v1/users/kuwo:{uid}?backend=modern` 要求路径 UID 属于该账户，再读取下文的本人资料；会员通过独立会员入口读取。调用方凭据不选择服务器别名，不回退到服务器默认账号。明确失效的凭据只从仍匹配原快照的本地来源清理，迟到结果不能清理新登录。

`POST /v1/auth/session/refresh` 使用所选原生会话交换，再独立校验新会话。两步都通过才按原快照保存；保留原登录代际、UID 和设备上下文，更新 SID。`both` 可用同一登录代际的旧调用方凭据取得服务器当前会话的更新，其他代际、账号或安装不接受。交换之后若独立核验失败，不发布未经核验的新 SID，原本地凭据保持不变；这时不保证平台仍接受旧 SID，需要根据后续校验结果处理或重新登录。刷新不自动重试，不代表官方令牌撤销。

`DELETE /v1/auth/session` 清理所选本地凭据。`client` 和 `both` 的来源及归属必须匹配，返回 `caller_credential_discard_required`，由调用方丢弃旧凭据；它不保证上游会话失效。`server` 不接受调用方凭据，`client` 必须提供凭据。所有账号入口沿用统一 `no-store` 和调用方凭据交付契约。

酷我已接入下文的本人资料、音乐会员及个人歌单目录。普通自建、收藏及喜欢歌曲与 Uni 导入见下文；普通完整账户音频、下载和可用性见下文；其余尚未接入账户来源的搜索、目录、歌词和视频入口仍明确拒绝账户请求。短信入口见下文，附加验证和真实账户业务仍按后续范围完成。

### 酷我上游会话撤销

`POST /v1/auth/session/revoke` 设置 `platform=kuwo`，支持 `server|client|both`，撤销选定的原生 SID。它与仅清理本地凭据的 `DELETE /v1/auth/session` 分开。请求前独立核验 UID/SID，已明确失效返回 `already_invalid`；否则发送一次官方退出请求，再独立核验同一 SID，只有明确失效才返回 `invalidated`。HTTP 401、未知业务错误和退出成功回执本身均不作为失效证明。

总预算为 45 秒，不自动重试或交换 SID。撤销请求开始后，即使超时、响应丢失、取消或无法确认结果，会条件清理仍与被撤销 UID/SID 完全一致的服务器凭据；并发重登或同代际 SID 轮换后的凭据会保留。调用方应检查 `caller_credential_discard_required`；错误还包含 `revocation_request_started`、`upstream_outcome`、`local_cleanup` 和 `removed`，不返回更新凭据。未发起撤销且验证结果不明时保留原凭据。真实账户的上游失效仍待最终验收。

### 酷我本人资料

`KuwoClient::native_self_profile(&credential)` 先独立验证原生 UID/SID，再从固定官方资料接口读取同一 UID。Provider 的 `user_profile`（`modern`）及对应本人资料 HTTP 入口接入相同协议；不接受其他用户 ID，每个网络响应完成后复核所选凭据，读取期间退出或重新登录会使旧请求失败。读取不交换 SID，也不接受资料响应的 Cookie。

资料包含上游实际提供的昵称、头像、签名、成长等级、生日、注册时间文字和直接背景图片；缺失字段保持未知。日期保留上游文字，不推断时区或时间戳单位；头像和背景仅接受已支持的酷我图片域名，保留原始 HTTP/HTTPS 协议，不主动获取图片。预设背景 ID 不合成 URL，关注数等尚未核实的字段不映射。成长等级不代表会员身份或单曲权益。账户登录名、恢复邮箱/手机号、QQ 和密保字段不会进入资料或扩展字段。

`session_profile` 与 `/v1/auth/session` 保持独立身份检查；它们不通过公开资料查询代替认证。真实账户资料的成功形态仍需最终验收。

### 酷我原生会员

`KuwoClient::native_membership(&credential)`、Provider 的 `user_membership` / `user_membership_client_info` 及 `GET /v1/account/membership?platform=kuwo`、`GET /v1/users/kuwo:{uid}/membership` 支持所选原生账户。`backend=front|client` 都读取同一官方原生会员接口，返回相同的当前会员明细；不能查询其他用户或使用匿名数据代替会员状态。

读取先独立验证 UID/SID，再使用同一安装和会话查询会员；两个网络边界都复核原账户来源，退出、换号和迟到错误遵循资料读取的保护规则。请求不轮换 SID，不采纳会员响应的 Cookie，不请求支付、领取或续费。会员数据为空时返回上游错误，不能把平台的空“成功”响应解释为非会员。响应不提供 UID 时，`user_ref` 绑定已独立核验的请求身份；若返回 UID，则还必须匹配。

`extensions.memberships` 分别提供 `legacy_vip`、`music`、`luxury`、`super`、`car`、`experience`、`ad`、`given` 八类状态、到期毫秒值及 UTC 时间、已知自动续订标志。状态按本次服务器 `ctime` 判断，缺失保持未知，明确零值表示没有该产品。统一 `active` 按官方音乐账户分类汇总 `music/luxury/super/given`；其他产品仍保留在明细。多个有效产品到期时间不一致、或这些音乐分类有未知项时，统一 `expires_at` 保持未知，调用方应读取各项明细。

平台年费代码保存在 `annual_user_code`，不将它当作年费数量；会员等级 `level` 与 `annual_count` 保持未知。`vip_tag`、`user_vip_type` 只保留有界的类型文字，图标只接受官方酷我域名的图片元数据，不主动下载。会员身份不等于单曲实时播放或下载授权；独立听书会员尚未包含，真实账户及会员样本留待最终验收。

### 酷我本人歌单目录

原生账户支持 `GET /v1/account/playlists?platform=kuwo`、`GET /v1/users/kuwo:{uid}/playlists/created` 和 `GET /v1/users/kuwo:{uid}/favorites/playlists`。UID 必须属于所选账户；支持服务器账户别名或调用方凭据。SDK 对应 `native_account_playlists`、`native_created_playlists`、`native_collected_playlists`，传入凭据和 `PageRequest`，SDK 不选择服务器别名。

读取先独立核验 UID/SID，然后取完整目录再按 `limit=1..100`、`offset` 返回窗口。自建只包含普通 `GENERAL` 歌单；喜欢、默认列表、电台和顺序记录不混入普通自建歌单。已知 `turn` 完整时稳定升序，否则保留上游顺序并标明。收藏按每页 20 项读取到短页或空页，最多 100 页；不凭匿名空列表判断账户有效。

合并目录先自建、后收藏，同一歌单同时出现在两组时保留两项，`extensions.library_section` 区分 `created` / `collected`。`library_owner_id` 表示本次目录所属账户；收藏者不被当作歌单创建者。可见性、创建者、数量和日期缺失时不补造；完整读取不等于上游原子快照，`consistency=single_complete_traversal` 明示边界。

每次响应最多 2 MiB，自建响应最多 4096 条，合并元数据最多 8 MiB，验证后的目录读取总预算 60 秒。越限、重复页、未知类型、后续分页失败都返回错误，不交付部分结果。退出、换号、认证失效和调用方来源检查覆盖每个网络边界；不刷新 SID、不接受业务响应的 Set-Cookie，并保留 HTTP `no-store`。

此能力提供目录元数据。普通自建、收藏及喜欢内容，以及已支持的歌单写操作见下文；账户媒体仍未接入。真实账户的非空、私密歌单及收藏样本仍需最终验收。

### 酷我本人自建歌单曲目

本节同时适用于所选账户收藏的普通音乐歌单。

`GET /v1/playlists/kuwo:{pid}` 及其 `/tracks` 入口，显式指定服务器 `account` 或携带调用方凭据时，先查本人普通自建及喜欢云歌单，未命中再查完整收藏目录。同一 ID 同时属于本人云歌单和收藏时优先本人云歌单；来源确定后不会因后续失败切换来源，不自动匿名回退。SDK 自建方法为 `native_created_playlist(&credential, pid)`、`native_created_playlist_tracks(&credential, pid, &PageRequest)`，收藏方法为 `native_collected_playlist`、`native_collected_playlist_tracks`，参数相同。SDK 方法只选择指定类别；喜欢曲目使用下方专用方法。

先独立验证 UID/SID，再核对所选类别的完整目录、连续完整读取两遍曲目并比较、最后重查同类目录。重复歌曲和顺序原样保留；同数量但内容变化、页数变化、缺页、后页错误均不交付部分结果。`songchange=false` 不是空歌单：没有本地缓存时，仅目录前后均明确零曲目且响应没有曲目时接受空结果。

自建每次遍历最多 15 个物理页；收藏以每页 100 首读取，最多 100 页，并检查返回的页码、页宽、总数和实际数量。两种来源均最多 10,000 首、16 MiB 类型化数据，每响应最多 2 MiB，验证后的业务总预算 120 秒；`limit=1..100` 仅在完整读取后切片。`source_snapshot_id` 是绑定 UID、来源类别、目录元数据及完整有序曲目的本地内容摘要，供独立 metadata/tracks 调用及 Uni 导入检测变化；两遍结果相等不代表上游原子快照，`consistency` 明示读取方式。每个网络边界检查原账户，取消或退出后不复活会话。

普通自建及收藏歌单可作为 Server/Client Uni 导入来源，沿用来源账户与播放账户分离、重复项保留、版本不一致拒绝及文档无凭据规则。该导入不提供酷我账户播放授权。曲目仅映射已知元数据，不从 `payInfo`、媒体 token 或目录格式声明音质和权益；节目等非歌曲内容明确报不支持，不丢弃后冒称完整。读取不轮换 SID、不接受 Set-Cookie，HTTP 保留 `no-store`。收藏者不被视为创建者；收藏目录没有明确零数量时，不凭内容接口的空响应认定有效空歌单。协议和账户边界已纳入离线测试，真实账户成功样本仍待最终验收。

本人普通自建歌单的详情与曲目读取现在同时读取完整可编辑资料：在两遍完整曲目前后各读取一次详情，再复核原类别目录。标签及已知标签 ID、封面尺寸、发布状态和类型等任一已知字段变化时拒绝返回快照；返回的 `Playlist.tags` 为已核实值，`extensions.editable_metadata_verified=true`，显式空标签是已知为空。目录数量未知时由完整详情与两遍曲目共同确认，目录隐私未知仍保留未知。已有私密、公开及已投稿歌单均可只读，不触发修改或下架。

普通自建歌单的 `source_snapshot_id` 同时包含完整已知详情和有序曲目，Uni 可识别仅标签变化造成的跨调用不一致。Server/Client 导入结果的每个已确认来源在 `extensions.source_tags` 返回标签，`source_metadata_verified=true`；服务器保存的 `import_sources` 同样保留这些值。不会把未知标签伪装为空，也不透传整个 Provider 扩展。收藏和喜欢列表仍使用各自已实现的读取链，此次不把本人普通歌单的创建者约束套到其他来源。路由和 120 秒业务预算不变，两个新增详情 GET 纳入 `upstream_pages_fetched`；这些检查仍不表示上游提供原子快照。

### 酷我喜欢歌曲与 Uni

原生账户支持 `/v1/account/favorites/playlist`、`/v1/account/favorites/tracks`，以及 `/v1/users/kuwo:{uid}/favorites/playlist`、`/tracks`。支持默认服务器账户、指定 `account` 和调用方凭据；指定 UID 必须等于所选账户。四个入口的成功和业务错误均返回 `no-store`，无显式别名的本人读取也保持私有响应。

SDK 方法为 `native_favorite_playlist(&credential)`、`native_favorite_tracks(&credential, &PageRequest)`。根据完整本人目录中的唯一 `MYFAVORITE` 标记选择真实云歌单 ID；同名普通歌单、手机默认列表和 PC 默认列表不能代替喜欢列表。名称沿用官方系统名“我喜欢听”，返回真实云歌单引用，并标记 `extensions.is_favorite=true`。喜欢列表缺失、ID 无效、标记重复或冲突时明确失败，不虚构空列表或创建列表。

喜欢读取使用上述自建云歌单的完整内容协议与预算：独立身份验证、两遍有序内容一致及目录复核，保留重复歌曲；云 ID、类型、数量或内容变化均失败。已返回的喜欢云引用也可通过带账户的普通歌单详情和曲目入口读取。普通自建目录及 `native_created_*` 方法仍排除喜欢列表。

Uni 的 `favorite_tracks` 来源使用本人 UID，例如 `{"ref":"kuwo:42","type":"favorite_tracks","account":"personal"}`；源元数据中的歌单引用是实际云 ID，两者用途不同。Server/Client 导入检查完整内容摘要，跨调用变化或认证失效时不发布部分结果；文档不包含凭据。喜欢写操作和普通账户媒体见下文；真实账户成功读取仍留待最终验收。

### 酷我投稿记录与审核状态

`GET /v1/account/playlist-submissions?platform=kuwo&account=personal&limit=30&offset=0` 读取所选账户的投稿记录，也支持默认账户和调用方凭据。SDK 使用 `native_playlist_submissions(&credential, &PageRequest)`，Provider 使用 `account_playlist_submissions`；能力为 `account_playlist_submissions`。其他未实现的平台明确拒绝，不回退到普通歌单目录。参数、请求 ID、方法和业务错误也返回 `no-store`。

每条 `PlaylistSubmission` 包含 `playlist_ref`、`owner_id`、可选名称／封面／歌曲数／收听数、`review_status` 和可选 `published`。审核状态为 `pending`、`approved`、`rejected` 或 `unknown`；缺失和未知状态不补成“审核中”，已知原始状态码保留在 `extensions.upstream_review_status`。此记录接口不能证明当前是否上线，`published` 保持 `null`；审核通过也不授予播放下载权限。记录缺失不能证明从未投稿、已撤稿或原歌单已删除。未知元数据保留 `null`，不返回原始响应或凭据。

独立核验 UID/SID 后，以官方每页 6 条、从第 1 页开始的接口完整读取两遍；短页（可为空）确认末尾，两遍已知字段与顺序一致后才截取请求窗口。保留重复记录，重复完整物理页因无法排除分页循环而拒绝，不去重或返回部分。`total` 表示读取到的记录数，不使用单条记录的歌曲数计算分页。`source_snapshot_id` 是本地内容摘要，`consistency=two_complete_reads` 是观察一致性，不代表上游提供原子快照。

每遍最多 256 页，最后必须短页，完整可验证上限为 1535 条；每响应最多 1 MiB，两遍累计最多 16 MiB，包含初始身份核验的总期限为 120 秒。`limit` 为 1–100，`offset` 非负且不能溢出。每个网络成功和错误都检查原账户代际；退出、重登、换号、坏数据、超限或内容变化时拒绝交付，明确认证失效仅清除仍未变化的原来源。该读取不轮换 SID，不接纳业务 Cookie，不提交或删除投稿。成功协议和账户边界通过本地模拟验证，真实账户记录仍需最终验收。

### 酷我显式投稿与编辑后重新投稿

`POST /v1/playlists/kuwo:{pid}/submission` 提交所选账户的普通公开歌单。JSON `{}` 提交已经保存的状态；可选 `account` 选择服务器账户，也可使用调用方凭据。可选 `name`、`description`、`tags` 会先保存资料并完整回读确认，再提交审核，例如 `{"name":"夜晚安静听的歌单","description":"夜间聆听","tags":["安静"],"account":"personal"}`。SDK 为 `native_submit_playlist(&credential, pid, &PlaylistSubmissionRequest)`，Provider 为 `submit_playlist`，能力名为 `playlist_submission_write`。

提交前要求本人普通公开歌单、已有封面、非空简介及标签、至少 10 个云端歌曲条目（保留重复歌曲），标题加权长度为 7–20：中文 U+4E00–U+9FA5 每个字算 1，其余每个 UTF-16 单元算 0.5。私密歌单需要先显式修改可见性。本入口不接受可见性或封面字段；这些字段不会被隐式修改。已上线歌单可通过此显式入口编辑后重新投稿；普通 PATCH 对已上线歌单的保护仍保留。

可选 `recommendation` 提交推荐词，例如 `{"recommendation":"夜间好歌","account":"personal"}`。提供时必须包含 1–5 个 Java UTF-16 单元，不允许控制字符或首尾 ASCII 空白；不会替调用方截短或去空白。此分支要求标题加权长度为 7–16，且标题无首尾 ASCII 空白。不提供或传 `null` 仍采用上述 7–20 的普通投稿规则。

推荐词分支分别校验推荐词与歌单名称/简介，只有完整回包与输入逐项对应且明确通过才继续；校验请求不计为资料保存或投稿。随后使用该分支的资料保存接口，并完整回读确认，再提交推荐词。即使未提供资料修改字段，也会执行一次资料保存与一次投稿，`metadata_updated=true` 表示保存步骤已确认，不表示内容必然发生变化。保存所需的本人登录名仅在内部读取和使用，不加入公开资料、凭据返回或结果扩展。`extensions.recommendation_submitted` 表示是否使用推荐词分支；文本拒绝时返回 `permission_denied`，不执行后续业务写入。

成功的 `accepted=true` 仅表示上游明确接收了这次投稿。`metadata_updated` 表示这次请求是否已经完成并确认资料保存；`records` 是随后完整读取到的该歌单投稿记录，`published` 是随后独立读取到的上线状态。审核通过、等待、驳回、空记录及未知状态均不能据此关联到这次请求；`extensions.records_correlated_to_request=false` 明示没有可核对的上游投稿事务 ID。重复调用每次都可能再次投稿，服务不依据旧记录假装幂等成功。

保存与投稿是两个独立业务写入。失败时通过 `details.playlist_write_outcome`（`not_dispatched`／`unconfirmed`／`confirmed`）和 `submission_outcome`（`not_dispatched`／`unconfirmed`／`accepted`）分别说明已知结果，同时提供写入次数及 `automatic_retry=false`。例如资料已确认保存后投稿失败，不会将保存回滚；投稿已接收但回读失败，也不会声称投稿从未发生。请求取消、网络失败或超时不能保证平台取消已经收到的写入，应先重新查询状态再决定后续操作。

总期限为 120 秒（包括身份核验），投稿确认响应上限 64 KiB；完整投稿记录、歌单资料和曲目复用各自的分页与响应预算。所有网络成功和错误均检查原账户来源，认证失效仅清理尚未替换的原会话；不自动重试、轮换 SID 或采用业务 Cookie。响应以及参数、请求 ID 和方法错误均为 `no-store`。当前验证来自本地协议与并发测试，真实账户投稿和审核结果仍待最终验收。

### 酷我删除投稿记录

`DELETE /v1/account/playlist-submissions/kuwo:{pid}?account=personal` 删除所选账户中该歌单的投稿历史记录；也支持默认账户和调用方凭据。仅接受 `account` 查询参数和空请求体。SDK 使用 `native_delete_playlist_submission_records`，Provider 使用 `delete_playlist_submission_records`，独立能力为 `playlist_submission_record_delete`；其他平台未实现时明确返回不支持。

删除前后分别完整读取两遍投稿记录，保留重复历史记录；一次写入后要求目标 PID 的所有记录消失，其他记录的已知字段、数量及顺序保持。成功返回 `confirmed=true`、`changed` 和 `removed_records`。若前后均没有目标记录，返回 `changed=false`、`removed_records=0`，不发送删除。上游成功回执本身不等于回读确认，不能把部分移除当作全部删除成功。

同时独立观察本人普通自建目录中的原歌单。`owned_playlist_present=false` 仅表示两遍完整目录中没有该歌单，`playlist` 和 `published` 为 `null`；它不证明全平台资源不存在或已下线。历史记录对应的歌单已不在本人目录时，仍可删除本人记录。原歌单存在时核验完整资料、曲目顺序和重复条目；响应的 `published` 与扩展中的 `published_before`、`publication_changed` 只描述观察到的上线状态，不建立与删除操作的因果关系。记录删除不是普通歌单删除，也不承诺撤稿或下线，不会调用普通删除接口替代。

已发出写入后的失败可能包含 `record_delete_outcome=unconfirmed/acknowledged/confirmed`。其中 `confirmed` 表示已经回读确认目标记录消失，但后续歌单或其他记录检查仍可能失败；这时 `write_outcome=partial`，不能当成整个操作成功或无副作用。原歌单在前后观察中出现或消失会返回冲突，并附 `owned_playlist_present_before/after`。不会自动重试或回滚。

总期限为 120 秒（含身份核验），回执上限 64 KiB；记录和歌单复用完整分页、响应体与曲目预算。所有成功和错误边界检查原账户代际，调用方模式不访问服务器账户仓库，不采用业务 Cookie 或轮换 SID。所有响应，包括早期参数、请求体、请求 ID 和方法错误，均为 `no-store`。当前验证为本地协议及并发测试，真实删除范围与上线状态变化留待最终账户验收。

### 酷我普通歌单创建与删除

`POST /v1/playlists` 支持 `{"platform":"kuwo","name":"新歌单","visibility":"public","kind":"normal","account":"personal"}`，创建空的公开普通歌单。歌单名遵循上游原生客户端的宽度校验：UTF-16 单元中落在 U+4E00–U+9FA5 的字符计 1，其余计 0.5，合计最多 20。当前只支持 `visibility=public`（HTTP 未指定时默认公开）；私密、`platform_default` 可见性、视频和共享类型尚未支持。`DELETE /v1/playlists/kuwo:{pid}?account=personal` 删除单个歌单；批量使用 `DELETE /v1/playlists`，例如 `{"refs":["kuwo:101","kuwo:102"],"account":"personal"}`。一次接受 1–100 个不同的正规酷我歌单引用，只能删除所选账户的普通自建歌单；喜欢列表、其他系统列表、收藏和缺失目标在写入前拒绝。

以上入口支持默认/指定服务器账户和调用方凭据，响应为 `no-store`。SDK 对应 `native_create_playlist(&credential, &PlaylistCreateRequest)` 和 `native_delete_playlists(&credential, &PlaylistDeleteRequest)`；SDK 凭据本身确定账户，不能指定服务器别名。每次先独立验证身份、完整读取本人目录，再发送一次写入并完整回读目录。创建确认新的真实云 ID、名称、公开状态和零歌曲；删除确认所有目标 ID 消失。成功返回 `extensions.confirmed=true`，不代表上游提供原子事务。

发送后遇到响应错误、回读不一致、退出换号或超时，错误包含 `details.write_outcome="unconfirmed"`、`write_requests_dispatched=1` 和 `retryable=false`。写入可能已经发生，应先重新读取目录再决定下一步；服务不会自动重发、回滚或清理新建歌单。取消客户端请求也不保证平台撤销已收到的操作。每个网络边界都检查原凭据，明确认证失效仅清理仍未变化的原来源，不轮换 SID，不接受业务响应的 Cookie。目录与写入确认响应分别限制 2 MiB 和 64 KiB，身份验证后的业务总预算为 120 秒。

名称/简介/标签修改见下文；隐私变更、曲目与收藏写操作继续扩展。当前验证覆盖离线协议、账户边界和 HTTP 契约；真实账户创建、删除及写后状态仍待最终验收。

### 酷我普通歌单资料修改

`PATCH /v1/playlists/kuwo:{pid}` 支持本人未投稿上线的普通自建歌单的 `name`、`description` 和 `tags`，例如 `{"name":"新名字","account":"personal"}`。SDK 方法为 `native_update_playlist(&credential, pid, &PlaylistUpdateRequest)`；Provider 使用通用 `update_playlist`。支持默认／指定服务器账户和调用方凭据，仅支持 `variant=default`；空请求、喜欢或其他系统列表、收藏歌单不执行修改。

已投稿上线的歌单在写入前返回 `capability_not_supported`，因为官方编辑流程包含再次投稿审核；普通资料修改不会隐式执行该流程。

未提供的字段保留原值；`description=""`、`tags=[]` 分别明确清空简介和标签。名字最多 1024 UTF-8 字节且非空、不能含控制字符；简介最多 16 KiB，允许换行、回车和制表符；标签最多 128 个，每项最多 256 字节，逗号连接后最多 4096 字节，不接受空标签、重复、逗号或控制字符。平台仍可能按业务规则拒绝名称或标签。

修改前独立核验账户，交叉检查完整本人目录和官方详情，并再次检查原普通歌单。已有封面值及公开／私密状态原样保留，完整详情返回的大图地址不会替换原封面；未知字段不当成空值。现有可见性会被保留，此请求不接受可见性变更。普通详情接口没有返回歌单 ID，因此内容绑定本次请求的 ID、所选本人目录和创建者 UID，不把创建者 UID 当成歌单 ID。

只发送一次修改，随后回读详情及目录，确认名称、简介、标签、封面、可见性与歌曲数量。成功响应的 `playlist` 包含已核实的标签，并标记 `extensions.editable_metadata_verified=true`；普通目录读取尚不返回这些详情标签。发送后的回包错误、变化、认证失效或超时沿用 `write_outcome=unconfirmed` 与 `retryable=false`，不自动重发或回滚。HTTP 响应为 `no-store`，元数据响应最多 2 MiB，写入确认最多 64 KiB，验证后的业务总预算 120 秒；上游没有条件写事务保证。真实账户和真实写后状态仍待最终验收。

### 酷我普通歌单隐私修改

`PUT /v1/playlists/kuwo:{pid}/visibility` 接收 `{"visibility":"private","account":"personal"}`，也支持 `public`。`visibility` 必填，不接受 `platform_default`、数字别名或额外字段。支持默认／指定服务器账户和调用方凭据，成功及失败均为 `no-store`。通用能力为 `playlist_visibility_write`；尚未实现的平台明确返回 `capability_not_supported`，不会转成普通资料修改。

SDK 使用 `native_update_playlist_visibility(&credential, id, &PlaylistVisibilityUpdateRequest)`；Provider 使用 `update_playlist_visibility`。只允许所选账户本人、未投稿上线的普通自建歌单；喜欢、收藏及其他系统列表不支持。官方客户端编辑已投稿上线歌单时还会提交重新审核，当前普通资料和隐私修改均在写入前明确返回“不支持”，不把可见性修改当作投稿或下线已确认。先独立核验身份，读取完整本人目录及可编辑详情，再复核目录后发出一次修改。名称、简介、标签、原封面、数量及已知发布／类型字段均保持；上游确认后再次读取详情和目录，只有可见性与目标一致且其余字段未变才返回 `PlaylistMutationResult`（`action=Update`、`confirmed=true`）。

不自动取消投稿、重试或回滚。上游没有条件更新事务；并发编辑或任何写后未确认结果为 `write_outcome=unconfirmed`、`retryable=false`，取消请求也不能保证撤销已被上游接收的写入。已有歌单的隐私修改不代表支持私密创建，创建接口仍仅支持显式公开。自动测试使用本地模拟服务，真实公开／私密切换留待最终账户验收。

### 酷我 Web 密码登录

统一 `POST /v1/auth/password` 支持 `platform=kuwo`、`backend=web`、`principal_type=username` 和明文密码。酷我网页密码登录必须先显示图形验证码，因此该入口返回 `state=verification_required`、`transaction_id` 和 `verification.method=image`；它不支持手机／邮箱 Web 标识、密码哈希或 `secure_captcha`。`client` 模式使用 `default` 且不读取服务器账号；`server`／`both` 使用账户仓库中的指定账号。

收到图形验证码后，调用 `POST /v1/auth/password/challenges/{transaction_id}/verify` 并发送 `{"action":"submit_image","answer":"<验证码>","password":"<重新输入的密码>"}`。密码不会保存在待验证事务中，故每次提交都须由调用方重新提供。需要换图时使用 `{"action":"refresh_image"}`；验证码、图片答案和临时网页 Cookie 不进入返回的凭据。账号只有在官方登录回执、Web UID/SID 会话检查，以及独立酷我原生 UID/SID 检查都通过后才确认并签发 `kuwo_native_v1` 凭据。登录后续附加验证或账户激活分支尚未实现时会明确失败，不自动切换到原生密码入口。

SDK／Provider 的单次 `password_login_with_mode` 不处理这个交互式 Web 挑战；请使用 `begin_password_login`／`advance_password_login`。自动化覆盖本地协议夹具，真实账户、验证码成功和实际 Web／原生会话兼容性仍留到最终验收。

### 酷我原生短信登录 SDK

`KuwoClient::send_native_login_sms(&request, &device)` 接收 `KuwoNativeSmsRequest` 和已初始化的安装身份。`phone` 为 11 位中国大陆手机号，不含国家码或空格。官方手机号登录页面提示，未注册的手机号会自动生成新账号；因此请求必须明确设置 `allow_account_creation=true`，默认值 `false` 会在发送前拒绝。此许可不表示短信已验证，也不授予任何音乐权益。

发送成功返回内存中的 `KuwoNativeSmsChallenge`，绑定原手机号、安装身份和服务器返回的短信标识。`login_native_sms(challenge, code)` 消耗该回执并提交 5 位 ASCII 数字验证码；只有原生登录响应和独立 UID/SID 核验均通过，才交付 `ProviderAuthResult` 及原生调用方凭据。回执不能克隆、序列化或用另一个手机号、设备续接，不包含验证码；它不是登录凭据。验证码、手机号、短信标识和响应中的额外资料不会进入返回的原生凭据或日志。

回执从开始发送起使用本地五分钟预算，包含后续登录和独立校验；单次发送最多等待 20 秒。`expires_in_secs()` 表示本地剩余预算，不能据此推断平台验证码有效期。`resend_after_secs()` 提供官方客户端 60 秒重发策略的剩余等待时间；直接 SDK 调用方仍需在同一手机号的多次请求之间实施节流。发送失败、超时、取消或收到不确定响应不会自动重试；提交失败不交付未核验的凭据，调用方应按重发限制开始新的流程。

SDK 可直接使用；需要服务器别名、三种凭据归属及跨请求节流时，使用下方 Provider 或 HTTP 事务。附加图形或其他验证不在未知业务错误时自动猜测。协议与状态测试使用本地模拟服务，未发送真实短信，真实短信登录留待最终账户验收。

### 酷我短信登录与统一 HTTP

创建短信事务使用 `POST /v1/auth/challenges`：

```json
{
  "platform": "kuwo",
  "account": "personal",
  "credential_mode": "both",
  "method": "sms",
  "backend": "standard",
  "principal": "<11位中国大陆手机号>",
  "country_code": "86",
  "allow_account_creation": true
}
```

`allow_account_creation` 必须是 JSON 布尔值，默认 `false`。酷我两种短信后端都要求明确的 `true` 才发送，因为未注册号码完成登录时会创建账号。酷狗允许默认不创建账号的登录，酷我没有对应保证。`country_code` 支持 `86`、`+86` 或省略，手机号不含国家码。酷我 `standard` 使用原生登录协议；`middle` 使用官方 PC Web 短信页，并且只有在调用方已展示并由用户勾选同意用户协议、隐私政策及儿童隐私政策后，才能额外传 JSON 布尔值 `accept_platform_policies: true`。该字段默认 `false`，只支持酷我 `middle`；其他后端传 `true` 会在网络请求前被拒绝。两种酷我后端共用同一号码的 60 秒冷却。`server/both` 需要已配置的服务器凭据仓库，`client` 只使用 `default`，不读取服务器账号。

创建成功后保存返回的 `transaction_id`，向 `POST /v1/auth/challenges/{transaction_id}/verify` 提交 `{"code":"<5位数字验证码>"}`。验证码提交只使用原事务里的手机号、账号别名、归属和账号创建许可，不能在验证请求中改写。通过原生登录和独立身份核验后，`server` 条件保存，`client` 返回调用方凭据，`both` 返回与条件保存完全相同的凭据。底层 Provider 使用 `begin_auth_challenge`、`complete_auth_challenge` / `advance_auth_challenge(SubmitCode)` 和 `auth_challenge_status`；需保留原始 stateful 回执，不使用无回执的旧式 start/verify 方法。

密码、刷新和短信事务共享每个 Provider 的 128 个名额，短信包含等待验证码的阶段。五分钟总预算从事务创建起计算，包括设备初始化、发送、用户输入和身份核验；所有网络步骤前后检查原服务器别名快照。相同 Provider 及其克隆中，对该服务器别名重新登录或本地退出会取消其未完成的 `server/both` 事务。独立 Provider 实例仍通过共享存储的条件写入防止覆盖新凭据，手机号节流和待验证回执不跨独立实例共享。

同一 Provider 及其克隆对同一手机号实施 60 秒发送冷却，不能通过更换别名或归属绕过；发送中和验证中的事务也不接受同手机号重入。冷却记录最多 1024 个，不删除尚未到期记录来放行新号码。短信请求失败或取消后仍保留已经开始发送的冷却，成功响应后至少再等待 60 秒。冷却结束后显式创建新短信事务会使该号码旧回执失效。不同手机号分别绑定各自回执。

同一回执只允许一个验证码提交进入网络，重复、篡改、过期或已取消的回执不能建立登录态；迟到成功或错误都不能替换新登录。错误验证码输入格式在联网前拒绝；验证码一旦进入上游验证流程，无论成功或失败都不重用该事务。所有 HTTP 成功与失败响应沿用认证 `no-store` 规则，不返回手机号、上游短信标识或未经独立核验的凭据。

### 酷我普通歌单曲目增删

`POST`／`DELETE /v1/playlists/kuwo:{pid}/tracks` 支持 `{"refs":["kuwo:11","kuwo:22"],"account":"personal"}`，通用 `/items` 可显式指定 `kind=track`。SDK 为 `native_mutate_playlist_items(&credential, id, action, &PlaylistItemMutationRequest)`，Provider 沿用 `mutate_playlist_items`。支持默认／指定服务器账户和调用方凭据，成功和业务错误响应为 `no-store`。一次接受 1–100 个不同的正规酷我歌曲引用；视频、收藏、喜欢及其他系统歌单不通过此普通歌单接口写入。

添加表示确保歌曲存在：已存在的歌曲保留原位置及重复次数，只发送缺失项。新增歌曲的位置及彼此顺序以平台实际结果为准，接口不承诺追加，也不额外排序。删除按歌曲引用移除全部出现次数，重复云 RID 按原列表展开提交；非目标曲目的顺序和重复次数必须保持。全部目标已满足时，完整读取确认后返回 `changed=false`、`write_requests_dispatched=0`，不会发送空修改。新增后超出 10000 首的本地完整读取上限会在写前拒绝；删除可处理上限内的全部目标出现次数。

独立认证后读取写前完整快照，单次写入取得严格确认后再次完整读取：曲目双遍有序一致、完整详情及目录均核验。名称、简介、已知标签和标签 ID、隐私、发布及类型等非目标资料必须保持。平台可能随曲目变化更新封面，返回 `extensions.cover_changed` 表明观察到的变化；不会主动写封面。成功返回真实 `snapshot_id`、`cloud_track_count` 和实际发送的 `sent_occurrences`。这些检查不提供上游原子事务保证。

官方客户端向已投稿上线歌单新增曲目时还会提交重新审核，当前实际新增在写前明确返回“不支持”；如果删除全部匹配条目后将不足 10 首云端歌曲，也在写前拒绝，避免隐式下线。重复歌曲按实际删除的每个条目计数；不产生变化的增删仍可作为只读操作确认。保留至少 10 首的删除仍须完整回读并确认已知发布状态未变，不能把数量检查当作状态不变的保证。任何写后不能确认的结果均为 `write_outcome=unconfirmed`、`retryable=false`，不自动重试或回滚；取消请求不能撤销上游可能已收到的写入。成功协议测试使用本地模拟服务，真实账户及真实增删验收统一最后进行。


### 酷我喜欢与取消喜欢

`PUT /v1/account/favorites/tracks/kuwo:{rid}?account=personal` 喜欢歌曲，`DELETE` 同路径取消喜欢；省略 `account` 使用默认账户，也可提交调用方凭据。Provider 为 `set_track_subscription`，SDK 为 `native_set_track_subscription(&credential, id, subscribed)`，能力名为 `track_subscription_write`。只接受正规正整数歌曲 ID；该能力不授予播放或下载权益。

操作按所选账户目录中的唯一 `MYFAVORITE` 找到真实喜欢歌单 ID，先完整读取并确认有序内容，再执行一次歌曲增删，最后对同一系统类别和同一 ID 完整回读。缺失或多个喜欢列表明确失败，不自动创建。已有歌曲的喜欢、未在列表中的取消喜欢，完整读取确认后返回 `changed=false`、`write_requests_dispatched=0`。取消喜欢会移除该歌曲的全部重复项，其余歌曲顺序与重复次数保留；新增歌曲的位置由平台决定。

返回 `SubscriptionResult` 的 `subscribed` 为已确认状态；扩展包含 `favorite_playlist_ref`、`source_snapshot_id`、`cloud_track_count`、`confirmed`、`changed` 及发送次数。平台自动更新封面时返回 `cover_changed=true`；其他已知列表身份、简介、隐私和顺序标记必须保持。来源摘要是本地内容一致性标识，不承诺原子写。单次响应及完整读取预算沿用喜欢读取限制，业务操作总预算为 120 秒。

每个网络边界检查原账户代际，退出、重新登录或调用方凭据被移除后拒绝迟到结果。发送后无法确认的结果为 `write_outcome=unconfirmed` 且不可自动重试；取消请求不保证撤销已经到达上游的写入，不自动回滚。HTTP 成功与错误响应均为 `no-store`。普通歌单曲目接口仍不写喜欢列表。当前验证为本地协议与 HTTP 契约测试，真实账户验收在最终阶段进行。


### 酷我歌单歌曲排序

`PUT /v1/playlists/kuwo:{pid}/tracks/order` 接收完整目标顺序，例如 `{"refs":["kuwo:22","kuwo:33","kuwo:11","kuwo:22"],"account":"personal"}`，也可使用 `ids`。Provider 为 `reorder_playlist_tracks`，SDK 为 `native_reorder_playlist_tracks(&credential, id, &PlaylistTrackOrderRequest)`。支持普通本人自建歌单和所选账户真实喜欢列表的云 ID；收藏歌单及其他系统类别不支持。默认／指定服务器账户和调用方凭据沿用现有隔离规则。

请求需包含 1–10000 个正规酷我歌曲引用，并与完整歌单中的每个歌曲 ID 及其出现次数一致。重复歌曲要重复列出；缺项、多项或重复次数变化均在写入前拒绝。SDK 和 HTTP 都拒绝空排列。已与当前顺序一致时，完整读取确认后返回 `changed=false`、`write_requests_dispatched=0`。

实际排序只发送一次独立请求，随后完整读取并确认顺序等于请求，全部已知歌曲资料和重复次数保持，歌单名称、简介、标签、隐私及投稿状态等已知资料未变。平台可能随新顺序重新生成歌单封面，此时返回 `cover_changed=true`。已投稿普通歌单可排序，但不会自动撤稿。返回实际 `snapshot_id`、`cloud_track_count` 和 `confirmed`；该摘要是本地内容一致性标识，不代表上游原子事务，也不标识相同歌曲的独立实例。

读取预算沿用账户完整歌单规则，整个写操作预算 120 秒。身份或类别变化、非预期资料变化、错误回包或无法确认实际顺序时明确失败；已发送的写入标为 `write_outcome=unconfirmed`，不可自动重试，不自动回滚。每个网络成功／错误边界检查原凭据代际，HTTP 成功与错误均为 `no-store`。当前验证为本地协议和 HTTP 契约测试，真实账户验收保留到最终阶段。

### 酷我自建歌单目录排序

`PUT /v1/account/playlists/order` 接收完整的普通自建歌单顺序，例如 `{"refs":["kuwo:102","kuwo:101"],"account":"personal"}`。Provider 为 `reorder_account_playlists`，SDK 为 `native_reorder_account_playlists(&credential, &PlaylistOrderRequest)`。支持默认／指定服务器账户和调用方凭据；SDK 本身不选择服务器账户别名。

请求必须包含所选账户全部普通自建歌单的正规酷我引用，数量为 1–4096，且不得重复。喜欢、默认列表、电台和其他系统列表，以及收藏歌单不在此排序范围内；缺项、多项和非本人目标均在写入前拒绝。该接口仅改变歌单在目录中的顺序，不改变歌单内的歌曲顺序。

完整读取后，只有各歌单的明确序号已经等于请求的 1 起始连续位置时，才返回 `changed=false`、`write_requests_dispatched=0`。旧序号缺失、重复、为零或为负数时，不能仅凭响应数组位置认定顺序已满足；会提交一次规范排序，再完整读取核对所有明确序号。

成功还要求目录中的全部已知歌单资料、系统列表及其序号保持不变；系统列表即使没有云 ID 也参与核对。返回请求顺序的 `playlist_refs`、`confirmed=true`、`ordering=explicit_turn_one_based` 和实际写入次数；完整回读不代表上游原子事务。写入发出后的异常或回读不一致标为 `write_outcome=unconfirmed`，不可自动重试，不执行补偿排序。退出、换号、认证失效和取消超时沿用原账户来源边界，HTTP 成功及业务错误响应均为 `no-store`。真实账户排序仍待最终验收。

### 酷我收藏与取消收藏歌单

`PUT`／`DELETE /v1/account/favorites/playlists/kuwo:{pid}` 设置所选账户的歌单收藏状态，可使用 `account` 查询参数或调用方凭据。Provider 为 `set_playlist_subscription`，SDK 为 `native_set_playlist_subscription(&credential, id, subscribed)`，能力名为 `playlist_subscription_write`。支持单个正规正整数歌单 ID，最大为有符号 64 位整数；默认／指定服务器账户和调用方凭据沿用现有隔离规则。

操作先独立验证所选会话，再完整读取收藏目录。当前状态已满足时返回 `confirmed=true`、`changed=false`、`write_requests_dispatched=0`；否则发送一次修改，核对平台确认及完整目录回读，确认目标成员状态、其余收藏歌单的已知资料和相对顺序保持。新增收藏的位置由平台决定，不承诺置顶或追加。返回 `resource_ref`、`subscribed`、`library_section=collected` 及已确认的修改状态；收藏者不会被视为源歌单创建者，此操作不修改源歌单内容或授予播放权益。

目录沿用每页 20 项、最多 100 页和短页终止规则，当前可完整核验最多 1999 项。已有 1999 项时，新增收藏会在写入前拒绝；已满足状态的重复调用及取消收藏仍可执行。跨页重复、分页中途失败、超限和资料异常均不会作为完整目录交付。

该上游修改虽然采用 GET，请求仍按写操作处理，禁止自动重试和跟随重定向。发送后出现异常、平台确认不明确或回读不一致，返回 `write_outcome=unconfirmed` 且不可自动重试，不执行补偿收藏。退出、换号、认证失效和取消超时检查覆盖每次分页及修改请求；HTTP 成功和业务错误响应均为 `no-store`。不轮换 SID、不接收业务 Set-Cookie；真实账户收藏与取消收藏仍待最终验收。

### 酷我收藏歌单目录排序

`PUT /v1/account/favorites/playlists/order` 接收收藏歌单的完整目标顺序，例如 `{"refs":["kuwo:102","kuwo:101"],"account":"personal"}`，也支持既有 `ids` 及歌单引用别名。Provider 为 `reorder_collected_playlists`，SDK 为 `native_reorder_collected_playlists(&credential, &PlaylistOrderRequest)`，能力名为 `playlist_collection_order_write`。默认／指定服务器账户和调用方凭据均受支持，SDK 本身不选择服务器别名。

此入口明确操作收藏目录；`PUT /v1/account/playlists/order` 对酷我仍用于普通自建目录。相同歌单 ID 可以同时出现在自建和收藏目录中，不通过 ID 猜测目标类别。其他平台未实现此独立能力时明确返回不支持，不回退到普通自建排序或收藏／取消收藏操作。

请求必须列出全部收藏歌单的唯一正规酷我引用，数量为 1–1999。缺项、多项、重复、混合平台和超限均在写入前拒绝。完整读取确认当前顺序已经符合请求时，返回 `changed=false`、`write_requests_dispatched=0`；否则只发送一次排序，再完整读取核对每个歌单的精确位置及全部已映射资料保持不变。此操作不修改歌单内容、创建者或播放权益。

返回请求顺序的 `playlist_refs`、`library_section=collected`、`confirmed`、`changed` 和写入次数。上游确认必须明确成功，缺少确认字段不视为成功；回读顺序或资料不一致、分页失败或写入后其他异常都返回 `write_outcome=unconfirmed`，不可自动重试，不执行补偿排序。完整回读不代表上游原子事务。退出、换号、认证失效和取消超时检查覆盖每个请求，HTTP 成功及业务错误响应为 `no-store`，不接收业务 Set-Cookie。真实账户排序及成功响应仍待最终验收。


### 酷我普通歌单封面修改

`PUT /v1/playlists/kuwo:{pid}/cover` 接受原始图片请求体和对应的 `Content-Type`，查询参数可提供 `account`、`filename`、`image_size`、`crop_x`、`crop_y`（也支持通用 `imgSize`、`imgX`、`imgY` 别名）。SDK 为 `native_update_playlist_cover(&credential, id, &ImageUploadRequest)`；Provider 沿用 `update_playlist_cover`。默认／指定服务器账户和调用方凭据分别选择原会话，响应及参数错误均为 `no-store`。

输入支持 JPEG、PNG、GIF 首帧和 BMP，字节格式必须与 MIME 一致；上限 20 MiB、单边 8192 像素、总计 16 Mi 像素。先应用图片方向信息；未指定裁剪时居中取正方形，指定时 `image_size` 为正方形边长、坐标默认为零，全部必须在图片内。透明像素合成白底，再输出 700×700、质量 100 的 JPEG。设备屏幕宽度与输出图片尺寸分开处理。图片解码和转换有资源限制，并发最多两个；不会读取服务器上的任意文件路径。

只允许本人未投稿上线的普通自建歌单，不借此修改系统／收藏列表或隐式提交重新审核。完整读取目标后先上传图片，再次核对歌单资料和本人目录，然后保存封面并完整读取歌曲和资料。只有新图片被确认使用、歌曲顺序与重复次数以及其他已知资料保持一致才返回 `confirmed=true`。结果 `image` 包含上传 URL、可选缩略图、输出尺寸及实际裁剪信息，不包含原始图片、原文件名或凭据；`source_snapshot_id` 是读回内容的本地摘要。

上传与歌单保存是两次独立写入，成功返回 `write_requests_dispatched=2`，并分别返回 `upload_requests_dispatched`、`playlist_write_requests_dispatched` 和各自结果。上传成功后保存未发出或未确认时，错误仍为 `write_outcome=unconfirmed`，用 `upload_outcome=confirmed` 与 `playlist_write_outcome=not_dispatched/unconfirmed` 区分；不能据此认为封面已更新。请求不自动重试、不回滚，退出、换号或迟到响应都不会继续操作新会话。真实上传、实际账户权限和线上图片处理仍需最终账户验收。


### 酷我普通账户音频与下载

既有 `/v1/tracks/kuwo:{rid}/stream`、`/download`、`/availability` 及播放／下载重定向入口支持原生账户。使用 `account=default`、指定服务器别名，或调用方凭据；省略账户且没有调用方凭据时保留公开接口行为。SDK 为 `native_stream`、`native_download`、`native_track_availability`，使用明确提供的原生凭据，不读取服务器账户。

账户链先核验 UID/SID，再独立读取当前歌曲权益与原生媒体结果。播放与下载分别查询授权；下载被拒绝时不改用播放 URL，即使设置 `fallback=true`。歌曲资料先核验所选会话，再读取不携带原生凭据的公开目录资料，标记 `catalogue_scope=public`；这一步不声明账户是否可播或可用音质。来源歌单中的媒体 token、会员标签及公开目录权限均不能代替本次媒体授权。

默认 `variant` 支持普通、明确完整的 AAC 48 kbps、MP3 128／320 kbps 和 FLAC：`low`、`standard`、`higher`／`high`、`lossless`、`hires`，以及独立的 `master` 母带、`spatial` 至臻全景声、`vinyl` 黑胶和 `dtsx` DTS:X 档位。`auto` 按已授权的母带、Hi-Res FLAC、普通 FLAC、MP3 320、MP3 128、AAC 48 顺序选择，不含 spatial、vinyl 或 dtsx；显式音质不静默降级。可选 `bitrate` 只接受与所选有损音质一致的 48000、128000 或 320000；FLAC 和 dtsx 实际码率保持未知。Hi-Res 的权益标记为 `HR / 4000 / HIRFLAC`，媒体响应须匹配 `HR / 4000 / flac`；4000 是档位选择器，不代表 4 Mbps 或固定采样率／位深。伴唱使用下述独立的 `variant=sing_along`；其他音质、变体和沉浸参数明确报不支持。

`master` 精确对应 `ZPLY / 20900 / mflac`，与 `ZP / 20000` 不同。20900 同样只是选择器，不是实际码率或固定采样率。必须取得原歌曲本次操作的完整授权和设备绑定密钥，内容接口解密后验证 FLAC 并输出 `audio/flac`、`.flac` 文件；不把密文 `.mflac` URL 当作播放或下载成功。显式母带不使用客户端省流降为无损的行为；Auto 只按已授权档位选择，选到加密档位时须使用内容接口。`mgg` 仅支持下述已核验的伴唱布局。

`dtsx` 独立选择 `DTSX / 25000 / mmp4`，播放和下载分别核验授权，精确匹配歌曲身份、媒体档位和受信 CDN 地址；解密后通过内容接口返回原始压缩字节，类型为 `audio/mp4`、扩展名 `.mp4`，单文件上限 128 MiB。25000 是上游档位选择器，不是实测码率；此档不参与 `auto`，也不映射为 `surround` 或 `spatial`。当前检查 MP4 样本表及 DTSX 帧的 FTOC 边界和校验值，不做 DTS 解码，也不宣称完整 DTS-UHD Profile 2 合规或可播放。真实权益、合法编码样本和客户端播放仍待最终验收。

`spatial` 精确对应 `ZPGA201 / 20201 / mflac`；20201 是档位选择器，不是实际码率，因此不接受显式 `bitrate`。播放与下载各自独立授权，取得设备绑定密钥后通过 `stream/content` 或 `download/content` 解密并验证 FLAC，输出 `audio/flac` 和 `.flac`；加密内容上限仍为 128 MiB。Auto 的选择顺序不包含此档，`immersive_type` 仍不接受。此功能不等同于 BCMS 伴唱、DTS:X 或其他 ZP 档，真实权益、最终播放及空间听感尚待验收。

`vinyl` 精确选择官方独立的 `VINYL / 23000 / flac` 黑胶资源，Auto 不包含此档；23000 是选择器，不是实际码率，不接受显式 `bitrate` 或 `immersive_type`。播放与下载分别核验 VINYL 权益和对应令牌，不以普通无损、母带或其他档位代替。合法明文 FLAC 沿用直链与内容接口；带设备绑定密钥的加密 FLAC 通过内容接口解密、完整校验后交付，内容上限为 512 MiB。未知采样率、位深、声道或实际码率不推断；真实黑胶资源和权益仍待最终验收。

显式 `variant=sing_along` 选择 BCMS 伴唱，SDK 使用 `StreamVariant::SingAlong`。例如 `/v1/tracks/kuwo:{rid}/stream/content?account=personal&variant=sing_along&quality=standard`；下载改用 `/download/content`，每次独立取得下载授权。伴唱保留引导人声，使用固定的人声 0.25、伴奏 1 权重及限幅处理；不提供纯去人声或自定义混音参数。

伴唱的 `quality` 选择普通主资源，接受 `auto`、`low`、`standard`、`higher`、`high`、`lossless` 和 `hires`；此变体下 Auto 的顺序为 Hi-Res、普通无损、MP3 320、MP3 128、AAC 48，不包含母带、全景声或黑胶。不能指定 `bitrate` 或 `immersive_type`。同次操作须同时获得主资源与独立 `BCMS / 22000` 权益和各自令牌；22000 不是实际码率。主资源与 `bc_data` 必须都覆盖完整歌曲，任何试听或截取标记均拒绝，不回退到普通音轨。

伴唱仅通过两个内容端点交付：验证并解密四声道、44.1 kHz 的 Ogg/Vorbis `.mgg`，按人声／伴奏声道混音后返回双声道、44.1 kHz、16 位 PCM WAV（`audio/wav`、`.wav`）。不返回原四声道媒体 URL 或密钥；普通 `/stream` 提示使用内容端点，`/download` 返回不可用及 `content_delivery=download_content`，不把主音轨的音质、码率或大小冒充最终 WAV 参数。容器须为单一完整音频流，解码帧数须与终止页一致，转换后的 WAV 上限为 512 MiB。默认变体不会自动选择伴唱。合成向量与离线协议验证不代替真实账户伴唱权益、下载及听感验收。

完整响应必须绑定原 RID、格式和质量，且明确没有试听或截取区间。只返回实际提供并通过校验的 HTTPS 媒体 URL，不把 HTTP 地址改写为 HTTPS。权限拒绝和部分音频不会报告可用全曲；`availability` 同样经过媒体核验，但不返回 URL。

`auto` 或 `standard`（兼容 128000 码率）在没有所请求的完整资源、且标准音质明确仅受付费限制时，可以向独立官方试听接口申请 30 秒以内的 MP3 试听；禁播、版权限制、未知权益、登录失效和其他显式音质不会触发此分支。试听必须再次校验 RID、格式、128 kbps、起止秒数及独立提供的 HTTPS 地址。`MediaStream.trial` 给出原曲时间轴上的毫秒范围，`duration_ms` 为该范围长度；`availability.playable=false`，通过 `preview_available`、`preview_start_ms`、`preview_duration_ms`、`preview_actual_bitrate` 描述试听。下载从不申请或接受试听，其他原生特殊预览类型仍明确拒绝。

账户读取不轮换 SID、不采纳业务 Cookie。请求期间退出或重新登录时，旧结果和迟到认证错误不覆盖新凭据；媒体请求总预算为 60 秒，不自动重试。HTTP 账户响应使用 `no-store`。协议计算、三种账户来源及 HTTP 契约由本地测试覆盖，真实账户登录、收费权益、最终媒体传输和播放仍待最终验收。

酷我加密媒体使用 `/v1/tracks/kuwo:{rid}/stream/content` 交付播放内容，使用 `/download/content` 交付经过独立下载授权的内容。均需明确的 `account`（包括 `default`）或调用方凭据；SDK 对应 `native_audio_content`、`native_download_content`，Provider 对应 `audio_content`、`audio_download_content`。`AudioContent.trial` 保留试听窗口；试听内容文件名带 `-preview`，MP3 帧时长须与授权范围相符（允许整秒边界和帧填充共 1.5 秒误差），不通过裁剪完整歌曲生成试听。这两个内容端点拒绝跨平台路由参数。下载内容使用 `Content-Disposition: attachment`，播放内容使用 `inline`，均返回实际音频格式及 `private, no-store`。

原生密钥绑定当前安装；支持已核验的短密钥变换和分段变换。不能覆盖完整文件的 301–511 字节密钥、超过 4096 字节的密钥及损坏封装均拒绝。普通 URL 端点不导出密钥或把密文 URL 当作可直接播放内容：加密 `/stream` 返回不支持并提示内容接口，`/download` 返回无 URL 和 `content_delivery=download_content`；已授权的加密 `/availability` 可返回可播并标记 `content_delivery=audio_content`。

内容传输只使用校验后的原生 HTTPS CDN 地址，不发送账户凭据，不跟随重定向或自动重试。普通档位完整缓冲上限为 128 MiB，母带和黑胶为 512 MiB；同时最多两项传输／转换，总预算 180 秒（其中媒体授权仍限 60 秒）。解密后检查 MP3、ADTS AAC、M4A 或 FLAC 的容器与帧结构；这不等于 PCM 解码或真实播放器验收。账户变化、取消、超时、异常 CDN 响应或损坏内容均不交付部分结果；CDN 的 401 不作为会话失效清除登录。合成音频验证不替代最终真实账户、权益和播放验收。


### 咪咕账户歌曲与播放授权

以下入口支持显式服务器账户 `account=default`／指定别名，或调用方 `X-TuneWeave-Credential`：歌曲详情、歌曲搜索、歌曲播放、可用性和播放重定向。调用方凭据只绑定自身账户，不能同时选择服务器别名；未指定账户的普通调用仍走公开路径，不暗读默认存储。

详情与候选搜索先核验所选 UID，再请求不携带 PACM 的公共目录，返回 `extensions.catalogue_scope=public` 和 `source_user_id`。该资料可参与 Uni 与跨平台严格匹配，但不声明账户可播、可下载或已购买。Uni 来源账户用于取来源资料，播放账户由本次请求独立选择；凭据不写入条目快照。

播放重新读取当前资源身份，再请求所选 PACM 的 PC 实时权益与播放授权；媒体响应中的候选 PACM 必须再次通过同一 UID 的资料核验。每个请求边界检查原登录代际，退出、同 token 重登录或换号后到达的成功及失败结果均返回 `conflict`。已验证的轮换遵循咪咕既有会话合同：普通下游错误保留更新，认证失败和冲突不交付更新；取消的调用不遗留待交付的调用方更新。每次账户读取／播放总期限 45 秒，PC 权益／播放响应限 1 MiB。

| 请求 | 账户 PC 音频语义 |
| --- | --- |
| `quality=low`／`standard` | PQ，实际 MP3 128 kbps |
| `quality=higher`／`high` | HQ，实际 MP3 320 kbps |
| `quality=lossless` | SQ，FLAC，未知实际码率保持未知 |
| `quality=hires` | ZQ24，FLAC，未知实际码率保持未知 |
| `quality=auto` | 按当前目录的普通音质顺序逐档独立请求，仅在明确播放格式拒绝后继续 |
| `bitrate=1..320000` | 覆盖质量选择，按 128／320 kbps 两档映射，返回实际码率 |

账户内容下载中的 ZQ24 FLAC 只接受单声道或双声道、24-bit 的 44.1、48、88.2、96、176.4 或 192 kHz 内容；逐帧采样率必须与 FLAC 元数据一致。此解析范围不表示每首获授权歌曲都会提供所有采样率，输出保持原始 FLAC 字节，不做转码。

仅支持默认 `variant`，暂不支持沉浸音效参数和其他质量家族。明确请求的格式未获得授权时返回 `permission_denied`；认证、网络、身份不匹配和未知业务错误不触发音质降级。只接收已核实域名及路径的 HTTPS 签名 URL，不向媒体地址附加 PACM。URL 的平台时间参数尚不能证明过期时间，因此 `expires_at` 保持未知，调用方应按需重新请求授权；不把目录缓存当作可重用的授权。

试听必须同时有实时试听权益和完整、有效且不超过原曲时长的窗口。`trial.start_ms`／`end_ms` 使用原曲时间轴，`duration_ms` 保留原曲时长。明确的 `cannotCode` 即使伴有试听字段也按拒绝处理。可用性入口 `br=999000`（默认）采用自动音质，`br=1..320000` 按对应档位实际授权；试听或明确拒绝返回 `playable=false`，未知状态仍返回错误。

账户下载使用独立的原生授权链，支持 `/v1/tracks/migu:{contentId}/download` 及 `/download/redirect` 的显式服务器账户或调用方凭据。实现先核验所选 PACM 账户，经 H5 交换得到临时 token，再要求原生 token 核验返回相同 UID；重新读取歌曲身份后请求独立下载授权，最后再次核验原账户。临时原生 token 不存入凭据仓库，也不返回给调用方或媒体服务器。整个操作限 45 秒，单次授权响应限 1 MiB；账户代际和已验证 PACM 轮换沿用上述合同。

当前下载只处理 PQ／HQ 的 MP3 和 SQ／ZQ24 的 FLAC，要求授权明确对应本次歌曲、格式、完整文件大小且没有加密或试听标记。明确音质不降级；`auto` 仅在某档明确返回 `permission_denied` 后继续。URL 暂限定为已核实的 `https://dlsdownfree.nf.migu.cn/wlansst` 路径及其子路径，且恰有一个非空 `pars` 授权参数；未知下载域名和路径拒绝。媒体请求不携带账户凭据，`expires_at` 保持未知。该 URL 范围是保守子集，不表示已覆盖所有账户 CDN。

`requires_download_authorization` 对账户来源返回 `true`，统一下载／重定向即使允许回退也不能把完整播放链接转为下载授权。上述下载 URL／302 路径仍拒绝加密内容；`/v1/tracks/migu:{contentId}/download/content` 则支持所选账户经独立下载授权的受限 MG3D 内容，仅限 PQ／HQ MP3 与 SQ／ZQ24 FLAC。该接口校验有效 fileKey、资源与格式身份、完整文件及原账户代际，解密并通过媒体完整性校验后才交付音频；不返回密钥或加密 URL。MGM、云盘／云端已购的专属下载及上述范围之外的未证实格式仍不支持。H5 token 与原生账户的真实兼容性、实际下载权益和 CDN 仍待最终真实账户验收；本地协议、Provider、Resolver 和 HTTP／Uni 测试不替代该验收。

### 咪咕单曲已购记录

`GET /v1/account/purchases/tracks?platform=migu` 使用所选服务器账户（可传 `account`）或调用方凭据；SDK 为 `account_purchased_tracks`。初始资料核验后，通过 PACM Cookie 读取单曲已购 ID，候选 PACM 再经同一 UID 核验后才接受。公开歌曲资料请求不携带账户凭据，随后再次完整读取购买 ID 并核验身份；登录代际、取消和普通错误时的更新规则与上述账户播放一致。

该接口保留购买记录及可解析的公开目录资料，不包含专辑订阅附带歌曲，也不代表实时媒体授权。空库必须经过同样的身份和完整列表复核；明确下架的歌曲保留引用和 `track=null`，网络或格式错误不会变成空记录。限制、分页字段和未支持范围见[API 文档](api-v1.md)。成功协议已通过离线 fixture 验证，真实非空账户库仍待最终验收。

### 咪咕已购专辑记录

`GET /v1/account/purchases/albums?platform=migu` 使用官方 PC 的 PACM 请求头读取专辑订阅列表；不把 PACM 当作 Android 原生全局 token。初始身份核验后，每页响应中的候选 PACM 都须重新核验同一 UID，已购响应累计预算在两遍完整读取间共享。成功、失败、取消、超时和登录代际保护沿用咪咕账户合同。

普通 `album` 与数字 `digital_album` 分开返回，未知标题保留未解析记录；专辑购买记录不作为收藏状态、当前会员或媒体权限。代码与离线协议测试覆盖默认／指定／调用方来源，真实非空专辑订阅库仍待最后账户验收。详细分页和类型合同见[API 文档](api-v1.md)。
