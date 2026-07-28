# B 站验证码 Provider 契约

本链路用于 B 站 GeeTest 人机验证、密码登录和短信验证码登录，实施阶段位于酷狗、咪咕、酷我公开音源补充层之后。实现时直接审查 `MOPELotus/Lotus-ReFactor` 已实际使用的完整流程，并按 TuneWeave 的强类型 provider、多账户、调用方凭证和短期事务模型重新设计；不得机械翻译参考代码。

## Provider 与顺序

统一 provider 为 `test_nine`、`ttocr`、`gtmanual`、`vision_ai`。默认顺序固定为 `test_nine → ttocr → gtmanual`；`vision_ai` 只能由部署方或调用方显式启用，不能默认进入处理链。顺序切换必须保留失败分类、耗时、尝试次数和事件通知。

`test_nine` 初期固定调用：

```text
GET https://cloud.lotusshared.cn/pass_uni?gt={gt}&challenge={challenge}
```

只读取响应中的 `validate`。必须区分 challenge 已使用、验证耗时过短、无有效 validate、请求失败和超时，并按统一规则决定重新获取 challenge、切换 provider 或终止。不得无限重试、高频探测或并发调用多个 provider。

`gtmanual` 使用外部 GT-Manual：

```text
POST https://gt.lotusshared.cn/GTest/register?key=114514
Content-Type: application/json

{"gt":"...","challenge":"..."}
```

注册响应中的 `link` 返回调用方用于人工验证，`result` 按严格受限频率轮询，成功后取得 `challenge`、`validate` 和可选 `seccode`。适配行为以 `MOPELotus/GT-Manual` 和 `Lotus-ReFactor` 的实际链路为协议依据。

注册返回的 `link` 与 `result` 均是不可信数据：只允许 HTTPS、主机精确为预先配置的 `gt.lotusshared.cn`、无用户信息、无非标准端口，且路径必须命中明确允许列表。禁止访问任意外部域名、IP 地址、其他协议或跟随越界重定向。

本阶段不内置 GT-Manual 页面或服务，也不内置 test_nine 模型和推理服务。两者只作为项目开发者临时提供的外部 provider；本地或原生 Rust 实现仅是未排期的远期方向。

## 外部后端告知与事务同意

每个登录事务首次准备调用 `test_nine` 或 `gtmanual` 前必须暂停，并向用户显示不可静默跳过的提示：

> 当前准备使用由项目开发者临时提供的外部验证码处理后端。验证码的 `gt`、`challenge` 及完成验证所必需的信息将发送至本次提示中列明的服务地址。该服务不是目标平台的官方服务，请确认已经阅读并同意后再继续。

提示必须列出本次可能使用的 provider、每个实际服务域名、调用顺序、发送的数据类型，并说明同意只对当前登录事务有效。调用方必须由用户显式提交“已阅读并同意”；客户端、SDK、机器人和脚本不得代替用户自动确认。

确认记录只存在于有期限的登录事务中，并绑定事务 ID、提示内容版本、列出的 provider、域名、顺序和到期时间。未确认、已过期、事务不一致、提示版本变化、实际 provider/域名未列出或确认令牌用于其他事务时必须拒绝。确认不能保存为账户级或调用方级永久同意；同一提示已完整列出的 provider 可在同一事务内切换，新增 provider 必须重新告知和确认。

## TTOCR 凭证模式

`ttocr` 支持调用方提供与服务器提供两种模式。

- 调用方模式：当前请求或短期事务携带提交接口、结果接口、`itemid`、服务商密钥及必要参数。端点必须为 HTTPS，并通过部署方允许的主机/路径策略和 DNS/IP 内网阻断；数据不得持久化或进入日志、Debug、错误和指标标签；调用方不能覆盖通用代理、任意请求头、重定向策略或内部网络范围。
- 服务器模式：使用部署方预配置的服务商凭证。策略 `public` 允许调用方在服务器并发、频率和额度限制内使用；策略 `access_key_required` 要求调用方提交 TuneWeave 部署方设置的访问密钥。

TuneWeave 访问密钥与 TTOCR 服务商密钥必须严格分离，服务器服务商密钥不得出现在任何调用方可见内容中。调用方自带 TTOCR 凭证不执行开发者临时后端的强制同意；共享 TTOCR 是否需要额外告知由服务器访问策略单独配置。

## 状态机与安全限制

所有 provider 共用强类型状态机和统一结果模型，覆盖 challenge 获取、等待同意、排队、处理中、等待人工操作、成功、可切换失败、需刷新 challenge、终止和超时。一个 challenge 可能被消费后不得交给下一个 provider 重用，必须按 B 站协议重新获取；challenge 刷新、单 provider 尝试和整链尝试都有硬上限。

部署配置必须明确设置全局、来源 IP、调用方、账户和登录事务并发数，以及调用频率、每日上限、最大队列、请求超时、结果轮询频率、单 provider 最大尝试、整链最大尝试、challenge 最大刷新、连续失败熔断和冷却时间。禁止无限重试、无上限刷新、并发 provider 或持续真实验证。

日志、事件和错误不得记录完整 `gt`、`challenge`、`validate`、`seccode`、TTOCR 密钥、TuneWeave 访问密钥、Cookie、手机号、密码或其他登录凭证。诊断只允许不可逆摘要、部分掩码、provider 名称、失败分类、耗时和重试次数。
