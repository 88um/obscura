// Run with OBSCURA_BIN and playwright-core available through NODE_PATH.
// One browser process, two pages, loopback fixtures, and a hard 60-second deadline.
const assert = require('node:assert/strict');
const http = require('node:http');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { chromium } = require('playwright-core');

async function main() {
  const fixture = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    if (req.url === '/graphql') {
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({ body: Buffer.concat(chunks).toString(), cursor: 'next' }));
    } else {
      res.setHeader('content-type', 'text/html');
      res.end('<!doctype html><title>fork compatibility</title>');
    }
  });
  fixture.listen(0, '127.0.0.1');
  await once(fixture, 'listening');
  const reservation = http.createServer();
  reservation.listen(0, '127.0.0.1');
  await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const token = 'offline-fork-compatibility-token-123456789';
  const child = spawn(process.env.OBSCURA_BIN, [
    'serve', '--host', '127.0.0.1', '--port', String(port), '--workers', '1',
    '--allow-private-network', ...(process.env.SMOKE_STEALTH === '1' ? ['--stealth'] : []),
  ], { env: { ...process.env, OBSCURA_CDP_TOKEN: token }, stdio: ['ignore', 'pipe', 'pipe'] });
  process.once('exit', () => child.kill('SIGKILL'));
  let logs = '';
  child.stdout.on('data', chunk => { logs = (logs + chunk).slice(-16000); });
  child.stderr.on('data', chunk => { logs = (logs + chunk).slice(-16000); });
  let browser;
  const deadline = setTimeout(() => {
    child.kill('SIGKILL');
    fixture.closeAllConnections();
    fixture.close();
    console.error('fork smoke exceeded 60 seconds', logs);
    process.exit(1);
  }, 60000);
  try {
    const endpoint = `http://127.0.0.1:${port}`;
    const auth = { Authorization: `Bearer ${token}` };
    // Bounded startup retry; no further processes or browser launches.
    for (let attempt = 0; attempt < 100; attempt++) {
      if (child.exitCode !== null) throw new Error(`server exited: ${logs}`);
      try {
        if ((await fetch(`${endpoint}/json/version`, { headers: auth })).ok) break;
      } catch {}
      if (attempt === 99) throw new Error(`server did not become ready: ${logs}`);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    assert.equal((await fetch(`${endpoint}/json/version`)).status, 401);
    const identity = Buffer.from(JSON.stringify({ version: 1, fingerprintSeed: 'offline-account',
      browserProfile: 'chrome_145', operatingSystem: 'windows' })).toString('base64url');
    browser = await chromium.connectOverCDP(endpoint, {
      headers: { ...auth, 'x-obscura-context': identity }, timeout: 10000,
    });
    const context = await browser.newContext();
    const pages = [await context.newPage(), await context.newPage()];
    const raw = await context.newCDPSession(pages[0]);
    await raw.send('Network.enable');
    for (let index = 0; index < pages.length; index++) {
      const page = pages[index];
      await page.goto(`http://127.0.0.1:${fixture.address().port}/`, { timeout: 10000 });
      const responsePromise = page.waitForResponse(r => r.url().endsWith('/graphql'), { timeout: 10000 });
      await page.evaluate(async index => {
        await fetch('/graphql', { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' },
          body: `cursor=page-${index}&count=12` });
      }, index);
      const response = await responsePromise;
      assert.equal(response.request().postData(), `cursor=page-${index}&count=12`);
      assert.deepEqual(await response.json(), { body: `cursor=page-${index}&count=12`, cursor: 'next' });
      assert.equal(await page.evaluate(() => document.createElement('div').dataset instanceof DOMStringMap), true);
      assert.equal(await page.evaluate(() => typeof ServiceWorkerRegistration), 'function');
    }
    await raw.send('Fetch.enable', { patterns: [{ urlPattern: '*graphql*' }] });
    const paused = once(raw, 'Fetch.requestPaused', { signal: AbortSignal.timeout(10000) });
    const completed = pages[0].waitForResponse(r => r.url().endsWith('/graphql'), { timeout: 10000 });
    // Reposter's mutation executor schedules the fetch and observes completion
    // separately, allowing the CDP processor to service Fetch.continueRequest.
    paused.catch(() => {});
    completed.catch(() => {});
    await raw.send('Runtime.evaluate', {
      expression: "void fetch('/graphql', {method: 'POST', body: 'cursor=intercepted'})",
      awaitPromise: false, returnByValue: true,
    });
    const [request] = await paused;
    assert.equal(Buffer.from(request.request.postDataEntries[0].bytes, 'base64').toString(), 'cursor=intercepted');
    await raw.send('Fetch.continueRequest', { requestId: request.requestId });
    await completed;
    const interceptedBody = await raw.send('Fetch.getResponseBody', { requestId: request.requestId });
    assert.equal(interceptedBody.base64Encoded, false);
    assert.deepEqual(JSON.parse(interceptedBody.body), { body: 'cursor=intercepted', cursor: 'next' });
    await raw.send('Fetch.disable');
    await context.close();
    console.log(`PASS: Playwright cursor replay, response bodies, two live pages, flattened session, intercepted response IDs, identity, bearer auth (${process.env.SMOKE_STEALTH === '1' ? 'stealth' : 'plain'})`);
  } catch (error) {
    console.error(logs);
    throw error;
  } finally {
    if (browser) await browser.close();
    child.kill('SIGTERM');
    const forceKill = setTimeout(() => child.kill('SIGKILL'), 3000);
    if (child.exitCode === null) await once(child, 'exit');
    clearTimeout(forceKill);
    fixture.closeAllConnections();
    await new Promise(resolve => fixture.close(resolve));
    clearTimeout(deadline);
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
