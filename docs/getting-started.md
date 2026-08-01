# 安装与配置

## 运行预编译包

从 GitHub Release 下载与系统匹配的 `tuneweave-<target>.tar.gz`，解压后运行 `tuneweave`；Windows 可执行文件名为 `tuneweave.exe`。每个压缩包都有同名 `.sha256` 文件，下载程序也可以从仓库根目录的 [`release-manifest.json`](../release-manifest.json) 获取版本、文件名、下载地址和校验地址。

默认监听地址是 `127.0.0.1:7832`：

```console
curl http://127.0.0.1:7832/healthz
```

## 从源码构建

需要 Rust 1.85 或更高版本：

```console
cargo build --release --locked -p tuneweave-server --bin tuneweave
```

产物位于 `target/release/tuneweave`，Windows 位于 `target/release/tuneweave.exe`。

## 基础配置

TuneWeave 使用环境变量配置。未设置时可直接启动。

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `TUNEWEAVE_BIND` | `127.0.0.1:7832` | HTTP 监听地址 |
| `TUNEWEAVE_DATA_DIR` | `.local/data` | 账户、设备身份和 Uni Playlist 的私有数据目录 |
| `TUNEWEAVE_LOG_LEVEL` | `info` | `trace`、`debug`、`info`、`warn`、`error` 或 `off`；优先于 `RUST_LOG` |
| `TUNEWEAVE_LOG_FORMAT` | `human` | `human` 或 `json` |
| `TUNEWEAVE_LOG_DIR` | `<data-dir>/logs` | 日志目录 |
| `TUNEWEAVE_LOG_FILE` | `tuneweave.log` | 日志文件名前缀 |
| `TUNEWEAVE_LOG_TO_STDERR` | `true` | 是否输出到标准错误 |
| `TUNEWEAVE_LOG_TO_FILE` | `true` | 是否输出到文件 |
| `TUNEWEAVE_LOG_RETENTION_DAYS` | `14` | 日志保留天数 |
| `TUNEWEAVE_LOG_MAX_FILES` | `30` | 最大日志文件数 |
| `TUNEWEAVE_LOG_MAX_FILE_BYTES` | `16777216` | 单个日志文件上限 |
| `TUNEWEAVE_LOG_MAX_TOTAL_BYTES` | `268435456` | 日志总空间上限 |

平台代理只能由部署方配置，调用方不能在 API 请求中覆盖：

| 平台 | 代理变量 |
| --- | --- |
| 网易云音乐 | `TUNEWEAVE_NETEASE_PROXY` |
| QQ 音乐 | `TUNEWEAVE_QQ_PROXY` |
| B 站 | `TUNEWEAVE_BILIBILI_PROXY` |
| 酷狗音乐 | `TUNEWEAVE_KUGOU_PROXY` |
| 咪咕音乐 | `TUNEWEAVE_MIGU_PROXY` |
| 酷我音乐 | `TUNEWEAVE_KUWO_PROXY` |
| 汽水音乐 | `TUNEWEAVE_SODA_PROXY` |

网易云音乐还支持以下部署配置：

- `TUNEWEAVE_NETEASE_COOKIE`：为 `default` 账户提供启动 Cookie。
- `TUNEWEAVE_NETEASE_REAL_IP`：为 provider 固定一个 IPv4 请求身份。
- `TUNEWEAVE_NETEASE_RANDOM_CN_IP=true`：启动时生成并复用一个中国 IPv4 请求身份，不能与固定地址同时使用。

## 数据与安全

`TUNEWEAVE_DATA_DIR` 包含服务器托管账户凭证、平台设备身份和 Server 模式 Uni Playlist。生产环境应把它放在仅服务账户可读写的本地目录中，不要放入公开目录、同步盘或镜像。

TuneWeave 不在静态数据文件中保存登录密码或短信验证码，但账户会话仍属于敏感信息。对外提供服务时应在可信反向代理后启用 HTTPS，并禁止代理记录 `X-TuneWeave-Credential`、Cookie、请求体和媒体签名 URL。

## 运行检查

```console
curl http://127.0.0.1:7832/v1/platforms
curl http://127.0.0.1:7832/v1/capabilities
curl "http://127.0.0.1:7832/v1/search?q=海阔天空&type=track&platform=all"
```

平台支持的能力可能受地区、账户权益和上游服务状态影响。调用方应先读取 `/v1/capabilities`，并按 [HTTP API v1](api-v1.md) 处理统一错误。
