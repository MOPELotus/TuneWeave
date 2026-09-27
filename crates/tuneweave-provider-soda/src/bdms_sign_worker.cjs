'use strict';

const net = require('node:net');
const fs = require('node:fs');
const addonPath = process.env.TUNEWEAVE_SODA_BDMS_ADDON;
const sdkPath = process.env.TUNEWEAVE_SODA_BDMS_PASSPORT_SDK;
const jsdomPath = process.env.TUNEWEAVE_SODA_BDMS_JSDOM;
const endpoint = String(process.env.TUNEWEAVE_SODA_BDMS_SOCKET || '');
const token = String(process.env.TUNEWEAVE_SODA_BDMS_TOKEN || '');

if (!addonPath || !/^127\.0\.0\.1:\d+$/.test(endpoint) || !/^[0-9a-f]{64}$/.test(token)) {
  process.exit(64);
}

let addon;
try {
  addon = require(addonPath);
} catch {
  process.exit(65);
}
if (typeof addon.init !== 'function' || typeof addon.generateHttpSignatureHeaders !== 'function') {
  process.exit(66);
}

const passportConfigured = Boolean(sdkPath && jsdomPath);
if (Boolean(sdkPath) !== Boolean(jsdomPath)) process.exit(67);
let window = null;
let xhrState = null;
let activePassportRequest = null;
let cookieNames = new Set();

if (passportConfigured) {
  try {
    const { JSDOM, VirtualConsole } = require(jsdomPath);
    const sdk = fs.readFileSync(sdkPath, 'utf8');
    const dom = new JSDOM('<!doctype html><html><head></head><body></body></html>', {
      url: 'https://api.qishui.com/',
      runScripts: 'outside-only',
      pretendToBeVisual: true,
      virtualConsole: new VirtualConsole(),
    });
    window = dom.window;
    Object.defineProperty(window.navigator, 'userAgent', {
      value: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) SodaMusic/3.7.0 Chrome/136.0.7103.59 Electron/36.4.0-rs.29.release.main.0 TTElectron/36.4.0-rs.29.release.main.0 Safari/537.36',
      configurable: true,
    });
    xhrState = new WeakMap();
    const prototype = window.XMLHttpRequest.prototype;
    const originalOpen = prototype.open;
    const originalSetRequestHeader = prototype.setRequestHeader;
    prototype.open = function (method, url, ...rest) {
      xhrState.set(this, { method: String(method), url: String(url), headers: {} });
      return originalOpen.call(this, method, url, ...rest);
    };
    prototype.setRequestHeader = function (name, value) {
      const request = xhrState.get(this);
      if (request) request.headers[String(name).toLowerCase()] = String(value);
      return originalSetRequestHeader.call(this, name, value);
    };
    prototype.send = function () {
      const request = xhrState.get(this);
      const active = activePassportRequest;
      if (!request || !active || active.finished) return;
      try {
        const url = new URL(request.url, active.target.origin);
        if (url.hostname !== active.target.hostname || url.pathname !== active.target.pathname) return;
        active.finish({ ok: true, url: url.toString() });
      } catch {
        active.finish({ ok: false, error: 'signing_failed' });
      }
    };
    window.eval(sdk);
    if (!window.bdms || typeof window.bdms.init !== 'function') throw new Error();
    window.bdms.init({ aid: 386088, paths: ['/passport'], boe: false });
  } catch {
    process.exit(68);
  }
}

function nativeSign(request) {
  const deviceId = String(request.device_id || '');
  const url = String(request.url || '');
  const headers = request.headers;
  const parsedUrl = new URL(url);
  const host = parsedUrl.hostname.toLowerCase();
  const qishuiHost = host === 'qishui.com' || host.endsWith('.qishui.com');
  if (!/^\d{19}$/.test(deviceId) || !Array.isArray(headers) || headers.length > 128 || parsedUrl.protocol !== 'https:' || !qishuiHost) {
    throw new Error();
  }
  if (nativeSign.activeDeviceId !== deviceId) {
    addon.init({ deviceId });
    nativeSign.activeDeviceId = deviceId;
  }
  const lines = [];
  for (const pair of headers) {
    if (!Array.isArray(pair) || pair.length !== 2) throw new Error();
    const name = String(pair[0]);
    const value = String(pair[1]);
    if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name) || /[\r\n]/.test(value)) throw new Error();
    lines.push(name, value);
  }
  const signed = String(addon.generateHttpSignatureHeaders(url, lines.join('\r\n')))
    .split('\r\n')
    .filter(Boolean);
  if (signed.length < 2 || signed.length % 2 !== 0) throw new Error();
  const output = {};
  for (let index = 0; index < signed.length; index += 2) {
    const name = signed[index].toLowerCase();
    if (!['x-helios', 'x-medusa'].includes(name) || /[\r\n]/.test(signed[index + 1])) throw new Error();
    output[name] = signed[index + 1];
  }
  if (!output['x-helios'] || !output['x-medusa']) throw new Error();
  return { ok: true, headers: output };
}

function signPassport(request) {
  if (!window) return Promise.resolve({ ok: false, error: 'passport_signer_unavailable' });
  const method = String(request.method || '');
  const rawUrl = String(request.url || '');
  const target = new URL(rawUrl);
  const host = target.hostname.toLowerCase();
  const qishuiHost = host === 'qishui.com' || host.endsWith('.qishui.com');
  if (!['GET', 'POST'].includes(method.toUpperCase()) || target.protocol !== 'https:' || !qishuiHost || !target.pathname.startsWith('/passport/')) {
    return Promise.resolve({ ok: false, error: 'invalid_request' });
  }
  return new Promise(resolve => {
    let finished = false;
    const finish = result => {
      if (finished) return;
      finished = true;
      clearTimeout(timer);
      activePassportRequest = null;
      if (result.ok) {
        const signed = new URL(result.url);
        if (signed.origin !== target.origin || signed.pathname !== target.pathname || !signed.searchParams.has('a_bogus')) {
          resolve({ ok: false, error: 'signature_missing' });
          return;
        }
        resolve({ ok: true, url: signed.toString() });
      } else {
        resolve(result);
      }
    };
    const timer = setTimeout(() => finish({ ok: false, error: 'signature_timeout' }), 750);
    activePassportRequest = { target, finish, finished: false };

    for (const name of cookieNames) {
      try { window.document.cookie = `${name}=; expires=Thu, 01 Jan 1970 00:00:00 GMT; domain=.qishui.com; path=/`; } catch {}
    }
    cookieNames = new Set();
    for (const [name, value] of Object.entries(request.cookies || {})) {
      if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name) || /[;\r\n]/.test(String(value))) continue;
      try {
        window.document.cookie = `${name}=${value}; domain=.qishui.com; path=/; secure`;
        cookieNames.add(name);
      } catch {}
    }

    try {
      const xhr = new window.XMLHttpRequest();
      xhr.open(method, rawUrl, true);
      for (const pair of request.headers || []) {
        if (!Array.isArray(pair) || pair.length !== 2) continue;
        try { xhr.setRequestHeader(String(pair[0]), String(pair[1])); } catch {}
      }
      xhr.send(request.body == null ? null : String(request.body));
    } catch {
      finish({ ok: false, error: 'signing_failed' });
    }
  });
}

const [host, port] = String(process.env.TUNEWEAVE_SODA_BDMS_SOCKET).split(':');
const socket = net.connect({ host, port: Number(port) });
let pending = '';
let chain = Promise.resolve();
socket.on('connect', () => socket.write(JSON.stringify({ type: 'ready', token, passport_available: passportConfigured }) + '\n'));
socket.on('data', chunk => {
  pending += chunk.toString('utf8');
  if (pending.length > 262144) { socket.destroy(); return; }
  let end;
  while ((end = pending.indexOf('\n')) >= 0) {
    const line = pending.slice(0, end);
    pending = pending.slice(end + 1);
    chain = chain.then(async () => {
      try {
        const request = JSON.parse(line);
        const result = request.mode === 'native' ? nativeSign(request)
          : request.mode === 'passport' ? await signPassport(request)
            : { ok: false, error: 'invalid_request' };
        socket.write(JSON.stringify(result) + '\n');
      } catch {
        socket.write('{"ok":false,"error":"signing_failed"}\n');
      }
    });
  }
});
socket.on('error', () => process.exit(1));
socket.on('end', () => {
  if (window) window.close();
  process.exit(0);
});
