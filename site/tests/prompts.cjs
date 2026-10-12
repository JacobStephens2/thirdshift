// Optional browser regression: install puppeteer-core outside the repo, then run
// NODE_PATH=<install>/node_modules CHROME_BIN=<chromium executable> node site/tests/prompts.cjs
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const http = require('node:http');
const path = require('node:path');
const puppeteer = require('puppeteer-core');
const root = path.resolve(__dirname, '..');
const server = http.createServer(async (req, res) => {
  try {
    const pathname = new URL(req.url, 'http://localhost').pathname;
    const file = path.join(root, pathname.endsWith('/') ? pathname + 'index.html' : pathname);
    const content = await fs.readFile(file);
    res.setHeader('Content-Type', file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
    res.end(content);
  } catch { res.writeHead(404); res.end(); }
});
(async () => {
  let browser;
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const url = `http://127.0.0.1:${server.address().port}/prompts/`;
    browser = await puppeteer.launch({executablePath:process.env.CHROME_BIN, headless:true, args:['--no-sandbox']});
    for (const width of [1440, 390]) {
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.setViewport({width, height:900});
      await page.goto(url);
      assert.equal(await page.$$eval('.section-disclosure', nodes => nodes.length), 4, 'four main sections have disclosures');
      assert.equal(await page.$$eval('details[open]', nodes => nodes.length), 0, 'all sections and sheets start closed');
      for (const title of ['command-title', 'prompts-title', 'skills-title', 'licence-title']) {
        const selector = `section[aria-labelledby="${title}"] > .wrap > details > summary`;
        await page.click(selector);
        assert.equal(await page.$eval(selector, node => node.parentElement.open), true, `${title} opens`);
        await page.click(selector);
        assert.equal(await page.$eval(selector, node => node.parentElement.open), false, `${title} closes`);
      }
      await page.click('section[aria-labelledby="prompts-title"] summary');
      await page.focus('#prompt-fresh > summary');
      await page.keyboard.press('Enter');
      assert.equal(await page.$eval('#prompt-fresh', node => node.open), true, 'Enter opens a prompt');
      assert.equal(await page.$eval('#prompt-resume', node => node.open), false, 'other prompt stays closed');
      await page.keyboard.press('Space');
      assert.equal(await page.$eval('#prompt-fresh', node => node.open), false, 'Space closes a prompt');
      await page.goto(url + '#file-thirdshift-tdd-skill-md');
      assert.equal(await page.$eval('#skill-thirdshift-tdd', node => node.open), true, 'file fragment opens skill');
      assert.equal(await page.$eval('section[aria-labelledby="skills-title"] details', node => node.open), true, 'file fragment opens section');
      assert.ok(await page.$eval('#file-thirdshift-tdd-skill-md', node => node.getBoundingClientRect().height > 0), 'linked file is visible');
      await page.goto(url);
      await page.click('section[aria-labelledby="command-title"] summary');
      await page.click('a[href="#prompt-resume"]');
      await page.waitForFunction(() => document.querySelector('#prompt-resume').open, {timeout:3000});
      assert.equal(await page.$eval('#prompt-resume', node => node.open), true, 'in-page link opens prompt');
      await page.click('#prompt-resume > summary');
      await page.click('a[href="#prompt-resume"]');
      await page.waitForFunction(() => document.querySelector('#prompt-resume').open, {timeout:3000});
      assert.equal(await page.$eval('#prompt-resume', node => node.open), true, 'same fragment link reopens closed prompt');
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'no horizontal overflow');
      assert.deepEqual(errors, [], 'no script exceptions');
      console.log(`PASS ${width}px: default closed; pointer/keyboard toggles; independent sheets; direct/in-page/repeated fragments; layout`);
      await page.close();
    }
    const noJs = await browser.newPage();
    await noJs.setJavaScriptEnabled(false);
    await noJs.goto(url);
    await noJs.click('section[aria-labelledby="prompts-title"] summary');
    await noJs.click('#prompt-fresh > summary');
    assert.equal(await noJs.$eval('#prompt-fresh', node => node.open), true, 'disclosures work without JavaScript');
    console.log('PASS no JavaScript: native toggles');
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
