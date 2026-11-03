#!/usr/bin/env python3
"""LAN-only login harness; runs against a separately started TuneWeave instance."""
import argparse
import http.server
import json
import pathlib
import re
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent
ALLOWED = re.compile(r"^/v1/(?:capabilities|auth/(?:qr(?:/[A-Za-z0-9_-]+(?:/verification)?)?|password(?:/challenges/[A-Za-z0-9_-]+/verify)?|challenges(?:/[A-Za-z0-9_-]+/verify)?|import|session(?:/refresh)?)|account/(?:profile|membership)|recommendations/styles/selection|extensions/qq/sound-power/detail|account/history/tracks)$")
SECRET = re.compile(r"password|cookie|musickey|(?:access|refresh|bearer|session).*token|caller.?credential|sessionid|authorization", re.I)


def scrub(value):
    if isinstance(value, dict):
        return {k: "[redacted]" if SECRET.search(k) else scrub(v) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v) for v in value]
    if isinstance(value, str):
        return re.sub(r"twc1_[A-Za-z0-9_-]+", "[redacted]", value)
    return value


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass  # Never log query strings, request bodies, QR contents or credentials.

    def reply(self, status, body, content_type="application/json; charset=utf-8"):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Frame-Options", "DENY")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.handle_request()

    def do_POST(self):
        self.handle_request()

    def do_DELETE(self):
        self.handle_request()

    def handle_request(self):
        path = urllib.parse.urlsplit(self.path)
        if self.headers.get("Host") not in self.server.allowed_hosts:
            self.reply(403, b'{"error":"invalid_host"}')
            return
        if self.headers.get("Sec-Fetch-Site") == "cross-site":
            self.reply(403, b'{"error":"cross_site_request"}')
            return
        origin = self.headers.get("Origin")
        if origin and origin != "http://" + self.headers.get("Host", ""):
            self.reply(403, b'{"error":"foreign_origin"}')
            return
        if self.command == "GET" and path.path in ["/", "/app.js", "/style.css", "/qrcode.js", "/info"]:
            if path.path == "/info":
                self.reply(200, json.dumps({"version": "1.0.0-beta.2", "isolated": True}).encode())
                return
            name = {"/": "index.html", "/app.js": "app.js", "/style.css": "style.css", "/qrcode.js": "node_modules/qrcode-generator/dist/qrcode.js"}[path.path]
            p = ROOT / name
            kind = "text/html" if p.suffix == ".html" else "text/css" if p.suffix == ".css" else "application/javascript"
            self.reply(200, p.read_bytes(), kind + "; charset=utf-8")
            return
        target = path.path.removeprefix("/api")
        if not path.path.startswith("/api/") or not ALLOWED.fullmatch(target):
            self.reply(404, b'{"error":"not_found"}')
            return
        started = time.monotonic()
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 <= size <= 1024 * 1024:
                raise ValueError("body_size")
            body = self.rfile.read(size) if self.command == "POST" else None
            if body:
                data = json.loads(body)
                if not isinstance(data, dict) or data.get("credential_mode", "server") != "server":
                    raise ValueError("server_credentials_only")
                if target in ["/v1/auth/qr", "/v1/auth/password", "/v1/auth/challenges", "/v1/auth/import"]:
                    if data.get("credential_mode", "server") != "server":
                        raise ValueError("server_credentials_only")
                    if not re.fullmatch(r"lab-[A-Za-z0-9_-]{1,40}", data.get("account", "")):
                        raise ValueError("isolated_account_required")
            query = urllib.parse.parse_qs(path.query)
            if "account" in query and (len(query["account"]) != 1 or not re.fullmatch(r"lab-[A-Za-z0-9_-]{1,40}", query["account"][0])):
                raise ValueError("isolated_account_required")
            if target == "/v1/auth/session/refresh" and body:
                if not re.fullmatch(r"lab-[A-Za-z0-9_-]{1,40}", data.get("account", "")):
                    raise ValueError("isolated_account_required")
            url = self.server.backend + target + ("?" + path.query if path.query else "")
            request = urllib.request.Request(url, data=body, method=self.command, headers={"Content-Type": "application/json"})
            try:
                response = self.server.opener.open(request, timeout=70)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                raw = response.read(8 * 1024 * 1024 + 1)
                if len(raw) > 8 * 1024 * 1024:
                    raise ValueError("response_size")
                result = scrub(json.loads(raw))
                status = response.status
            safe_path = re.sub(r"/(qr|challenges)/[A-Za-z0-9_-]+", r"/\1/{transaction}", target)
            record = {"at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "method": self.command, "path": safe_path, "status": status, "elapsed_ms": round((time.monotonic() - started) * 1000), "code": result.get("error", {}).get("code"), "request_id": result.get("meta", {}).get("request_id")}
            with self.server.events.open("a", encoding="utf-8") as log:
                log.write(json.dumps(record) + "\n")
            self.reply(status, json.dumps(result, ensure_ascii=False).encode())
        except (ValueError, json.JSONDecodeError):
            self.reply(400, b'{"ok":false,"error":{"code":"invalid_lab_request","message":"Invalid request or account alias; use lab- prefix"}}')
        except (OSError, urllib.error.URLError):
            self.reply(502, b'{"ok":false,"error":{"code":"backend_unavailable","message":"Login backend unavailable or timed out"}}')


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=17840)
    parser.add_argument("--backend", default="http://127.0.0.1:17841")
    parser.add_argument("--events", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if urllib.parse.urlsplit(args.backend).hostname not in ["127.0.0.1", "localhost"]:
        parser.error("backend must be loopback")
    args.events.parent.mkdir(parents=True, exist_ok=True)
    args.events.touch(mode=0o600, exist_ok=True)
    server = http.server.ThreadingHTTPServer((args.bind, args.port), Handler)
    server.backend = args.backend.rstrip("/")
    server.events = args.events
    server.allowed_hosts = {f"{host}:{args.port}" for host in [args.bind, "127.0.0.1", "localhost"]}
    server.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    print(f"Login lab: http://{args.bind}:{args.port}", flush=True)
    server.serve_forever()
