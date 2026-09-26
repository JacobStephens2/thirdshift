// Renders site/share-card.png, the 1200×630 Open Graph and Twitter card
// image: the home page's headline over its press line, taken from the page
// itself so the card follows the site. Re-run it after changing either.
//
// Needs playwright-core and its Chromium (npx playwright install chromium):
//   NODE_PATH=<dir with node_modules> node docs/share-card/render.mjs
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

// Through require, so NODE_PATH finds a playwright-core installed anywhere.
const { chromium } = createRequire(import.meta.url)('playwright-core');

const site = fileURLToPath(new URL('../../site/', import.meta.url));
const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml' };

// The page links its stylesheet and script from the root, so serve site/.
const server = createServer(async (req, res) => {
  const path = new URL(req.url, 'http://x').pathname;
  try {
    const body = await readFile(join(site, path.endsWith('/') ? path + 'index.html' : path));
    res.writeHead(200, { 'content-type': types[extname(path) || '.html'] ?? 'application/octet-stream' });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
await page.goto(`http://127.0.0.1:${server.address().port}/`, { waitUntil: 'networkidle' });
await page.evaluate(() => {
  const press = document.querySelector('.press-svg').cloneNode(true);
  const colorbar = document.querySelector('.hero .colorbar').cloneNode(true);
  document.body.innerHTML = `
    <div class="card grid-bg">
      <div class="card-top">
        <p class="eyebrow">thirdshift.app</p>
        <p class="eyebrow">How a Run works</p>
      </div>
      <h1 class="misreg">Your issues go to press overnight.</h1>
    </div>`;
  const card = document.querySelector('.card');
  card.append(press, colorbar);
  const style = document.createElement('style');
  style.textContent = `
    body { padding: 0; }
    .card { width: 1200px; height: 630px; padding: 30px 36px 28px; display: flex; flex-direction: column; gap: 14px; overflow: hidden; }
    .card-top { display: flex; justify-content: space-between; }
    .card-top .eyebrow:first-child { color: var(--paper); }
    .card h1 { font-size: 76px; }
    .card .press-svg { flex: 1; min-height: 0; }
  `;
  document.head.append(style);
});
await page.evaluate(() => document.fonts.ready);
await page.screenshot({ path: join(site, 'share-card.png') });
await browser.close();
server.close();
