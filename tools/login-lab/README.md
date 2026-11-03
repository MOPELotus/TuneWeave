# 内网登录测试页

用于逐个平台手动测试 QQ、网易云、酷狗和汽水的登录及会话。只连接独立的 TuneWeave 后端，账户别名必须以 `lab-` 开头，不读取正式服务账户。密码、Cookie 和回执不写入页面存储或访问日志，诊断文件仅记录阶段、状态和请求编号。

```sh
npm ci --prefix tools/login-lab
# 先用单独的数据目录在 127.0.0.1:17841 启动待测 TuneWeave。
python3 tools/login-lab/server.py --bind 10.10.10.18 --port 17840 --events /私有目录/events.jsonl
```

汽水需要按 `docs/authentication.md` 配置签名服务。页面提供扫码、密码、短信和 Cookie 导入；仅开放平台声明支持的方式。额外验证支持图片、短信、账户选择及官方 Web 短信 iframe 回执；酷狗原生浏览器桥接需要独立 WebView，页面会明确显示该限制，不能把普通 iframe 当作完成原生桥接。

打开页面后选平台、生成二维码或主动提交登录，再检查会话、资料和刷新。切换平台会停止扫码轮询。图形验证阶段需要重新输入密码。短信不会自动发送或重发；请遵守平台冷却时间。

代理边界测试：`python3 -B -m unittest discover -s tools/login-lab -p test_server.py`。测试覆盖来源检查、隔离账户限制、导入路由及日志脱敏，不会联系音乐平台。

本工具仅作内网人工测试，不向公网部署。诊断日志不包含请求正文和原始上游响应。测试结束后自行停止对应服务。
