import { createServer } from 'node:http';
import { pathToFileURL } from 'node:url';
import { chromium } from 'playwright';

// Protocol constants only. The vendor SDK is loaded from its official origin at runtime.
const sdk = 'https://acstatic-dun.126.net/tool.min.js';
const product = 'YD00000558929251';
const business = 'bd5d2f973ef74cd2a61325a412ae54d9';

export async function acquireToken(browser) {
  const context = await browser.newContext({ serviceWorkers: 'block' });
  const timer = setTimeout(() => { void context.close().catch(() => {}); }, 40_000);
  try {
    const page = await context.newPage();
    page.setDefaultTimeout(12_000);
    // An empty first-party document avoids loading the music application or any account state.
    await context.route('https://music.163.com/', route => route.fulfill({
      contentType: 'text/html', body: '<!doctype html><meta charset="utf-8"><title>Watchman</title>',
    }));
    await page.goto('https://music.163.com/');
    await page.addScriptTag({ url: sdk });
    const token = await page.evaluate(async ({ product, business }) => {
      function callbackResult(start) {
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error('SDK timeout')), 12_000);
          const finish = value => { clearTimeout(timer); resolve(value); };
          const fail = () => { clearTimeout(timer); reject(new Error('SDK failure')); };
          try { start(finish, fail); } catch { fail(); }
        });
      }
      const client = await callbackResult((onload, onerror) =>
        window.initWatchman({ productNumber: product, auto: true, onload, onerror }));
      await callbackResult(done => client.getInstance().I(done));
      return callbackResult(done => client.getToken(business, done));
    }, { product, business });
    if (typeof token !== 'string' || !/^[\x21-\x7e]{1,8192}$/.test(token)) {
      throw new Error('SDK returned no valid token');
    }
    return token;
  } finally {
    clearTimeout(timer);
    await context.close();
  }
}

export function tokenServer(acquire) {
  let active = 0;
  return createServer(async (request, response) => {
    const reply = (status, body) => {
      response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      response.end(JSON.stringify(body));
    };
    // No CORS, browser-origin requests, credentials, parameters, or request bodies accepted.
    if (request.method !== 'POST' || request.url !== '/token') return reply(404, { code: 404 });
    if (request.headers.origin || request.headers.cookie || request.headers.authorization
      || request.headers['transfer-encoding'] || Number(request.headers['content-length'] ?? 0) !== 0) {
      return reply(400, { code: 400 });
    }
    if (active >= 2) return reply(429, { code: 429 });
    active++;
    try { reply(200, { code: 200, registered: true, token: await acquire() }); }
    catch { reply(502, { code: 502, registered: false }); }
    finally { active--; }
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const port = Number(process.env.TUNEWEAVE_WATCHMAN_PORT ?? 17863);
  if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('Invalid port');
  const browser = await chromium.launch({ headless: true });
  const server = tokenServer(() => acquireToken(browser));
  server.requestTimeout = 45_000;
  server.headersTimeout = 5_000;
  server.listen(port, '127.0.0.1', () => console.log(`Watchman adapter listening on 127.0.0.1:${port}`));
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => {
    server.close();
    void browser.close();
  });
}
