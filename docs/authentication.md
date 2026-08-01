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

响应包含 TuneWeave 事务 ID、二维码内容和过期时间。使用事务 ID 轮询：

```http
GET /v1/auth/qr/{transaction_id}
```

状态为 `waiting`、`scanned`、`confirmed`、`expired` 或 `failed`。凭证归属模式在创建事务时固定，确认阶段不能更改。

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

## 验证码登录

创建并发送挑战：

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

`POST /v1/auth/challenges/validate` 只校验验证码，不创建登录态。`POST /v1/auth/security-challenges` 用于已登录账户的安全操作验证码。国家和地区号可从 `GET /v1/auth/country-codes?platform=<platform>` 获取。

## 会话管理

| 方法 | 端点 | 说明 |
| --- | --- | --- |
| `GET` | `/v1/auth/session` | 返回脱敏会话状态 |
| `POST` | `/v1/auth/session/refresh` | 刷新凭证；调用方模式返回新凭证 |
| `DELETE` | `/v1/auth/session` | 退出平台会话；调用方模式会提示丢弃本地凭证 |

服务器账户通过 `platform + account` 选择。调用方凭证通过 `X-TuneWeave-Credential` 提交。刷新失败不会替换服务器保存的旧凭证。

## 安全要求

- 对外部署时使用 HTTPS。
- 不记录 `X-TuneWeave-Credential`、Cookie、密码、手机号、验证码、二维码事务数据或平台 token。
- 不向浏览器开放通配 CORS 凭证请求。
- 调用方凭证应存放在操作系统安全存储或同等保护的私有区域。
- 二维码、验证码和密码只用于当前登录事务，不应进入 Uni Playlist 或业务日志。
