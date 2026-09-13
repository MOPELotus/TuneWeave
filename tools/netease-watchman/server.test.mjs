import { test } from 'node:test';
import assert from 'node:assert/strict';
import { acquireToken, tokenServer } from './server.mjs';

test('fresh acquisition, credential rejection, bounded concurrency and failure do not leak tokens', { timeout: 10_000 }, async () => {
  let calls = 0;
  let fail = false;
  let blocked;
  const server = tokenServer(async () => {
    calls++;
    if (blocked) await blocked;
    if (fail) throw new Error('private SDK error');
    return `token-${calls}`;
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}/token`;
  try {
    const first = await fetch(url, { method: 'POST' });
    assert.equal(first.headers.get('cache-control'), 'no-store');
    assert.equal((await first.json()).token, 'token-1');
    assert.equal((await (await fetch(url, { method: 'POST' })).json()).token, 'token-2');
    for (const headers of [
      { cookie: 'MUSIC_U=private' }, { origin: 'https://example.test' },
      { authorization: 'Bearer private' },
    ]) {
      assert.equal((await fetch(url, { method: 'POST', headers })).status, 400);
    }
    assert.equal((await fetch(url, { method: 'POST', body: 'private' })).status, 400);
    assert.equal((await fetch(url)).status, 404);
    assert.equal((await fetch(url + '?token=private', { method: 'POST' })).status, 404);
    assert.equal((await fetch(url.replace('/token', '/other'), { method: 'POST' })).status, 404);
    assert.equal(calls, 2);
    fail = true;
    assert.deepEqual(await (await fetch(url, { method: 'POST' })).json(), { code: 502, registered: false });
    fail = false;
    let release;
    blocked = new Promise(resolve => { release = resolve; });
    const requests = [fetch(url, { method: 'POST' }), fetch(url, { method: 'POST' })];
    while (calls < 5) await new Promise(resolve => setImmediate(resolve));
    assert.equal((await fetch(url, { method: 'POST' })).status, 429);
    release();
    for (const response of await Promise.all(requests)) assert.equal(response.status, 200);
  } finally {
    await new Promise(resolve => server.close(resolve));
  }
});

test('SDK acquisition creates and closes an isolated context for each token', async () => {
  let contexts = 0;
  let closed = 0;
  const browser = {
    async newContext(options) {
      assert.deepEqual(options, { serviceWorkers: 'block' });
      const sequence = ++contexts;
      return {
        async route(url, handler) {
          assert.equal(url, 'https://music.163.com/');
          await handler({ async fulfill(response) {
            assert.equal(response.contentType, 'text/html');
            assert.ok(!response.body.includes('<script'));
          } });
        },
        async newPage() {
          return {
            setDefaultTimeout(timeout) { assert.equal(timeout, 12_000); },
            async goto(url) { assert.equal(url, 'https://music.163.com/'); },
            async addScriptTag({ url }) { assert.equal(url, 'https://acstatic-dun.126.net/tool.min.js'); },
            async evaluate(_callback, args) {
              assert.deepEqual(Object.keys(args).sort(), ['business', 'product']);
              return `sdk-token-${sequence}`;
            },
          };
        },
        async close() { closed++; },
      };
    },
  };
  assert.equal(await acquireToken(browser), 'sdk-token-1');
  assert.equal(await acquireToken(browser), 'sdk-token-2');
  assert.equal(contexts, 2);
  assert.equal(closed, 2);
});

for (const result of ['', 'bad\r\nvalue', 'x'.repeat(8193), null, new Error('private SDK failure')]) {
  test(`SDK invalid result or failure closes its context (${result instanceof Error ? 'failure' : typeof result}/${result?.length ?? 0})`, async () => {
    let closed = false;
    const browser = { async newContext() {
      return {
        async route() {},
        async newPage() {
          return {
            setDefaultTimeout() {}, async goto() {}, async addScriptTag() {},
            async evaluate() { if (result instanceof Error) throw result; return result; },
          };
        },
        async close() { closed = true; },
      };
    } };
    await assert.rejects(acquireToken(browser));
    assert.equal(closed, true);
  });
}
