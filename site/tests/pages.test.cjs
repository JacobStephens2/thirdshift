const { test, before, after } = require("node:test");
const assert = require("node:assert/strict");
const { createServer } = require("node:http");
const { readFile } = require("node:fs/promises");
const { join } = require("node:path");
const { chromium } = require("playwright");

const publicPaths = ["/", "/docs/", "/how-it-works/", "/prompts/", "/videos/"];

let server, browser, origin;
before(async () => {
  server = createServer(async (request, response) => {
    const path = new URL(request.url, "http://localhost").pathname;
    const file = path.endsWith("/") ? `${path.slice(1)}index.html` : path.slice(1);
    try {
      const content = await readFile(join(__dirname, "..", file));
      const types = { js: "text/javascript", css: "text/css", jpg: "image/jpeg", woff2: "font/woff2" };
      const type = types[file.split(".").pop()] ?? "text/html";
      response.writeHead(200, { "Content-Type": type }).end(content);
    } catch {
      response.writeHead(404).end();
    }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise(resolve => server.close(resolve));
});

async function pageForTest(t, options) {
  const context = await browser.newContext(options);
  t.after(() => context.close());
  const page = await context.newPage();
  await page.route("https://**", route => route.abort());
  return page;
}

// Nothing in the selected containers, or the page, scrolls sideways.
async function assertFits(page, selector, message) {
  const overflow = await page.locator(selector).evaluateAll(nodes =>
    nodes.filter(node => node.scrollWidth > node.clientWidth + 1).map(node => node.textContent.trim().slice(0, 80)));
  assert.deepEqual(overflow, [], message);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, "no horizontal page scrolling");
}

async function checkSectionToggles(page, contentSelector, closeKey) {
  for (const heading of await page.locator("main h2").all()) {
    assert.equal(await heading.isVisible(), true);
    const fold = heading.locator("xpath=ancestor::details[1]");
    const content = fold.locator(contentSelector);
    assert.equal(await content.isVisible(), false);
    await heading.click();
    assert.equal(await content.isVisible(), true);
    assert.equal(await page.locator("main details[open]").count(), 1);
    await fold.locator(":scope > summary").focus();
    await page.keyboard.press(closeKey);
    assert.equal(await content.isVisible(), false);
  }
}

test("the guide starts with a folded overview and its sections toggle independently without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  await page.goto(`${origin}/how-it-works/`);
  assert.equal(await page.locator("main h2").count(), 7);
  assert.equal(await page.locator("main details[open]").count(), 0);
  await checkSectionToggles(page, ":scope > .guide-content", "Space");
});

test("the manual starts folded and readers can unfold nested sections without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false, viewport: { width: 320, height: 900 } });
  await page.goto(`${origin}/docs/`);
  const headings = page.locator("main h2");
  assert.ok(await headings.count() > 10);
  assert.equal(await page.locator("main details[open]").count(), 0);
  assert.equal(await page.locator("main details").count(), await page.locator("main :is(h2,h3,h4)").count());
  await checkSectionToggles(page, ":scope > .manual-content", "Enter");
  await page.getByRole("heading", { name: "What a Run does", exact: true }).click();
  const claim = page.getByRole("heading", { name: "The Claim", exact: true });
  assert.equal(await claim.isVisible(), true);
  const ends = page.getByRole("heading", { name: "When the Claim ends", exact: true });
  assert.equal(await ends.isVisible(), false);
  await claim.click();
  assert.equal(await ends.isVisible(), true);
  await ends.click();
  assert.equal(await page.getByRole("link", { name: "Sweep", exact: true }).first().isVisible(), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
});

test("every public page identifies its canonical URL, purpose and share metadata without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  const titles = new Set();
  const descriptions = new Set();
  for (const path of publicPaths) {
    assert.equal((await page.goto(`${origin}${path}`)).status(), 200);
    assert.equal(await page.locator('link[rel="canonical"]').count(), 1, path);
    assert.equal(await page.locator('link[rel="canonical"]').getAttribute("href"), `https://thirdshift.app${path}`);
    const title = await page.title();
    const description = await page.locator('meta[name="description"]').getAttribute("content");
    assert.match(title, /thirdshift/);
    assert.ok(description.length > 50, path);
    titles.add(title);
    descriptions.add(description);
    for (const [selector, expected] of [
      ['meta[property="og:type"]', "website"],
      ['meta[property="og:site_name"]', "thirdshift"],
      ['meta[property="og:url"]', `https://thirdshift.app${path}`],
      ['meta[property="og:title"]', title],
      ['meta[property="og:description"]', description],
      ['meta[property="og:image"]', "https://thirdshift.app/share-card.png"],
      ['meta[name="twitter:card"]', "summary_large_image"],
      ['meta[name="twitter:title"]', title],
      ['meta[name="twitter:description"]', description],
      ['meta[name="twitter:image"]', "https://thirdshift.app/share-card.png"],
    ]) {
      const tag = page.locator(`head ${selector}`);
      assert.equal(await tag.count(), 1, `${path}: ${selector}`);
      assert.equal(await tag.getAttribute("content"), expected, `${path}: ${selector}`);
    }
    assert.equal(await page.locator('meta[name="robots"][content*="noindex"]').count(), 0);
    assert.equal(await page.locator("h1").count(), 1, path);
    assert.ok((await page.locator("body").textContent()).includes("GitHub"), path);
  }
  assert.equal(titles.size, publicPaths.length, "page titles distinguish the search results");
  assert.equal(descriptions.size, publicPaths.length, "each page has its own search description");
  await page.goto(`${origin}/`);
  assert.match(await page.title(), /GitHub issues.*pull requests/);
});

test("robots advertises a valid sitemap of reachable canonical pages", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  const robots = await page.request.get(`${origin}/robots.txt`);
  assert.equal(robots.status(), 200);
  assert.match(await robots.text(), /^Allow: \/$/m);
  assert.match(await robots.text(), /^Sitemap: https:\/\/thirdshift\.app\/sitemap\.xml$/m);
  const sitemap = await page.request.get(`${origin}/sitemap.xml`);
  assert.equal(sitemap.status(), 200);
  const urls = await page.evaluate(xml => {
    const document = new DOMParser().parseFromString(xml, "application/xml");
    if (document.querySelector("parsererror")) throw new Error("Invalid sitemap XML");
    if (document.documentElement.namespaceURI !== "http://www.sitemaps.org/schemas/sitemap/0.9") {
      throw new Error("Missing sitemap namespace");
    }
    return [...document.querySelectorAll("urlset > url > loc")].map(node => node.textContent);
  }, await sitemap.text());
  // The metadata test checks reachability and canonical URLs for this inventory.
  assert.deepEqual(urls.sort(), publicPaths.map(path => `https://thirdshift.app${path}`));
});

test("the homepage declares the site name as static structured data", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  await page.goto(`${origin}/`);
  assert.equal(await page.locator('script[type="application/ld+json"]').count(), 1);
  const site = JSON.parse(await page.locator('script[type="application/ld+json"]').textContent());
  assert.equal(site["@context"], "https://schema.org");
  assert.equal(site["@type"], "WebSite");
  assert.equal(site.name, "thirdshift");
  assert.equal(site.url, "https://thirdshift.app/");
  assert.equal(site.url, await page.locator('link[rel="canonical"]').getAttribute("href"));
  assert.equal(site.name, await page.locator('meta[property="og:site_name"]').getAttribute("content"));
});

test("the homepage links to the shared Markdown manual without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false, viewport: { width: 320, height: 900 } });
  await page.goto(`${origin}/`);
  await page.getByRole("link", { name: "Documentation", exact: true }).first().click({ timeout: 2000 });
  assert.equal(new URL(page.url()).pathname, "/docs/");
  assert.equal(await page.getByRole("heading", { name: "thirdshift", exact: true, level: 1 }).isVisible(), true);
  await page.getByRole("navigation", { name: "Manual sections" }).getByRole("link", { name: "Install", exact: true }).click();
  assert.equal(new URL(page.url()).hash, "#install");
  assert.equal(await page.locator("#install > pre code").first().textContent(),
    "curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh\n");
  assert.equal(await page.getByRole("link", { name: "Markdown source", exact: true }).getAttribute("href"),
    "https://github.com/JacobStephens2/thirdshift/blob/main/docs/guide.md");
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  const fragments = await page.locator('main a[href^="#"]').evaluateAll(nodes => nodes.map(node => node.hash.slice(1)));
  for (const fragment of fragments) {
    assert.equal(await page.locator(`[id="${fragment}"]`).count(), 1, fragment);
  }
  assert.ok((await page.locator("main code").allTextContents()).some(text => text.includes("<Issue URL>")), "command placeholders remain text");
  assert.equal(await page.getByRole("link", { name: "ADR-0001", exact: true, includeHidden: true }).first().getAttribute("href"),
    "https://github.com/JacobStephens2/thirdshift/blob/main/docs/adr/0001-rust-binary-with-embedded-skills.md");
  for (const path of ["/styles.css", "/docs/docs.css"]) {
    assert.equal((await page.request.get(`${origin}${path}`)).status(), 200, path);
  }
});

test("the homepage leads to a guide that teaches the process in order", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/`);
  await page.getByRole("link", { name: "How thirdshift works", exact: true }).first().click();
  assert.equal(new URL(page.url()).pathname, "/how-it-works/");
  assert.equal(await page.getByRole("heading", { name: "How thirdshift works", level: 1 }).isVisible(), true);
  assert.deepEqual(await page.locator("main h2").allTextContents(), [
    "Start with work you can describe",
    "Give the Run its own checkout",
    "Let the agent implement and review",
    "Deliver against the latest Base branch",
    "Know what comes back",
    "Scale one Run into a Spec run",
    "Choose how the next work starts",
  ]);
});

test("guide navigation and Repair loop links reach visible explanations without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  await page.goto(`${origin}/how-it-works/`);
  const links = page.getByRole("navigation", { name: "Guide sections" }).getByRole("link");
  assert.equal(await links.count(), 7);
  for (const link of await links.all()) {
    const href = await link.getAttribute("href");
    await link.click();
    assert.equal(new URL(page.url()).hash, href);
    assert.equal(await page.locator(href).isVisible(), true);
    assert.ok(await page.locator(href).evaluate(node => Math.abs(node.getBoundingClientRect().top) < 40), `${href} scrolls into view`);
  }
  await page.getByRole("link", { name: "Then return to merging the Base branch" }).click();
  assert.equal(new URL(page.url()).hash, "#delivery-sync");
  assert.equal(await page.locator("#delivery-sync").isVisible(), true);
  await page.getByRole("link", { name: "thirdshift / home" }).click();
  assert.equal(new URL(page.url()).pathname, "/");
});

test("the guide's local links and styles resolve, including prompt deep links", async (t) => {
  const page = await pageForTest(t);
  // Exercise navigation before the deferred fragment-reveal script finishes loading.
  await page.route("**/main.js", async route => {
    await new Promise(resolve => setTimeout(resolve, 200));
    await route.continue();
  });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(`${origin}/how-it-works/`);
  const paths = await page.locator('a[href^="/"], link[href^="/"]').evaluateAll(nodes => [...new Set(nodes.map(node => node.getAttribute("href")))]);
  for (const path of paths) {
    const response = await page.request.get(`${origin}${path}`);
    assert.equal(response.status(), 200, path);
  }
  await page.getByRole("heading", { name: "Let the agent implement and review", exact: true }).click();
  await page.getByRole("link", { name: "the review procedure" }).click();
  await page.locator("#skill-thirdshift-code-review .job-body").waitFor({ state: "visible", timeout: 5000 });
  assert.deepEqual(errors, []);
});

for (const width of [320, 390, 768, 1280]) {
  test(`the guide's diagrams remain readable at ${width}px without JavaScript`, async (t) => {
    const page = await pageForTest(t, {
      javaScriptEnabled: false,
      reducedMotion: "reduce",
      viewport: { width, height: 900 },
    });
    await page.goto(`${origin}/how-it-works/`);
    for (const heading of await page.locator("main h2").all()) await heading.click();
    const diagrams = page.locator("main figure");
    assert.equal(await diagrams.count(), 6);
    for (const diagram of await diagrams.all()) {
      assert.equal(await diagram.isVisible(), true);
      assert.ok(await diagram.locator("figcaption").textContent());
    }
    await assertFits(page, "main, header, .guide-diagram, .guide-node, .guide-flow > li, pre", "text and diagrams fit their containers");
  });
}

for (const javaScriptEnabled of [false, true]) {
  test(`guide and manual deep links unfold their content (JavaScript: ${javaScriptEnabled})`, async (t) => {
    const page = await pageForTest(t, { javaScriptEnabled });
    for (const [path, fragment, text] of [
      ["/how-it-works/", "delivery-sync", "Bring the latest upstream changes"],
      ["/docs/", "when-the-claim-ends", "Sweep"],
    ]) {
      await page.goto(`${origin}${path}#${fragment}`);
      const target = page.locator(`#${fragment}`);
      await target.waitFor({ state: "visible", timeout: 2000 });
      assert.match(await target.textContent(), new RegExp(text));
      assert.ok(await target.evaluate(node => Math.abs(node.getBoundingClientRect().top) < 40), "fragment scrolls into view");
    }
    await page.goto(`${origin}/docs/`);
    const installLink = page.getByRole("navigation", { name: "Manual sections" }).getByRole("link", { name: "Install", exact: true });
    await installLink.click();
    assert.equal(await page.locator("#install > pre").first().isVisible(), true);
    await page.getByRole("heading", { name: "Install", exact: true }).click();
    assert.equal(await page.locator("#install > pre").first().isVisible(), false);
    await installLink.click();
    await page.locator("#install > pre").first().waitFor({ state: "visible", timeout: 2000 });
    assert.equal(await page.locator("#workflows").isVisible(), false, "unrelated sections stay folded");
  });
}

// The videos page's entries, in order.
const videos = [
  { title: "thirdshift in 31 seconds", audience: "Anyone", length: "0:32", id: "eRB6gGOQdJA" },
  { title: "thirdshift, briefly", audience: "Engineering leaders", length: "2:02", id: "yynbSGpHXGc" },
  { title: "How thirdshift works", audience: "Engineers", length: "2:38", id: "oL-5pi1AFC4" },
  { title: "How thirdshift works, step by step", audience: "Engineers, first look", length: "3:22", id: "gehONSA0viQ" },
  { title: "How to use thirdshift", audience: "New users", length: "3:21", id: "pNhNyW9JgqA" },
];
const playerUrl = id => `https://www.youtube-nocookie.com/embed/${id}?autoplay=1`;

// A stand-in for YouTube's player, so tests that play a video never leave the machine.
async function stubPlayer(page) {
  await page.route("https://www.youtube-nocookie.com/embed/**", route =>
    route.fulfill({ contentType: "text/html", body: "<title>player</title>" }));
}

test("the homepage and the guide link to the videos page without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  for (const path of ["/", "/how-it-works/"]) {
    await page.goto(`${origin}${path}`);
    await page.getByRole("link", { name: "Videos", exact: true }).first().click();
    assert.equal(new URL(page.url()).pathname, "/videos/", path);
  }
});

test("the videos page lists every video in order, each a thumbnail linking to YouTube without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false });
  await page.goto(`${origin}/videos/`);
  assert.deepEqual(await page.locator("main .video").evaluateAll(nodes => nodes.map(node => [
    node.querySelector("h2").textContent,
    node.querySelector(".video-audience").textContent,
    node.querySelector(".video-length").textContent,
    node.querySelector("a.video-play").getAttribute("href"),
  ])), videos.map(({ title, audience, length, id }) => [title, audience, length, `https://youtu.be/${id}`]));
  for (const summary of await page.locator("main .video-text > p:last-child").allTextContents()) {
    assert.equal(summary.match(/[.!?](\s|$)/g)?.length, 1, `one sentence on what the video covers: ${summary}`);
  }
  for (const [index, { title, id }] of videos.entries()) {
    const thumbnail = page.locator("main .video").nth(index).locator("a.video-play img");
    assert.equal(await thumbnail.getAttribute("src"), `/videos/${id}.jpg`);
    assert.equal(await thumbnail.getAttribute("width"), "1280");
    assert.equal(await thumbnail.getAttribute("height"), "720");
    assert.ok((await thumbnail.getAttribute("alt")).startsWith(`Title card: ${title}.`), title);
    const response = await page.request.get(`${origin}/videos/${id}.jpg`);
    assert.equal(response.status(), 200, id);
    assert.equal(response.headers()["content-type"], "image/jpeg");
  }
  await page.route("https://youtu.be/**", route => route.fulfill({ contentType: "text/html", body: "YouTube" }));
  await page.locator("a.video-play").first().click();
  assert.equal(page.url(), `https://youtu.be/${videos[0].id}`);
});

test("the videos page requests nothing from YouTube or Google until a video plays, then embeds only that video", async (t) => {
  const page = await pageForTest(t);
  await stubPlayer(page);
  const external = [];
  const failed = [];
  page.on("request", request => { if (!request.url().startsWith(`${origin}/`)) external.push(request.url()); });
  page.on("response", response => { if (response.url().startsWith(`${origin}/`) && response.status() !== 200) failed.push(response.url()); });
  await page.goto(`${origin}/videos/`, { waitUntil: "networkidle" });
  // Bring the lazily loaded thumbnails in too.
  await page.locator("main .video").last().scrollIntoViewIfNeeded();
  await page.waitForLoadState("networkidle");
  assert.equal(await page.locator("main .video img").evaluateAll(nodes => nodes.every(node => node.complete && node.naturalWidth > 0)), true);
  assert.deepEqual(external, [], "fonts, thumbnails and scripts are all served by thirdshift.app");
  assert.deepEqual(failed, []);
  assert.equal(await page.locator("iframe").count(), 0);

  const { title, id } = videos[2];
  const video = page.locator("main .video").nth(2);
  await video.locator("a.video-play").click();
  const player = video.locator("iframe");
  await player.waitFor();
  await page.waitForLoadState("networkidle");
  assert.equal(await page.locator("iframe").count(), 1);
  assert.equal(await video.locator("a.video-play").count(), 0, "the player replaces the thumbnail in place");
  assert.equal(await player.getAttribute("src"), playerUrl(id));
  assert.equal(await player.getAttribute("title"), title);
  assert.deepEqual(external, [playerUrl(id)]);
  const box = await player.boundingBox();
  assert.ok(Math.abs(box.width / box.height - 16 / 9) < 0.02, `16:9, not ${box.width}x${box.height}`);
});

test("each video plays from the keyboard, with a visible focus style", async (t) => {
  const page = await pageForTest(t);
  await stubPlayer(page);
  await page.goto(`${origin}/videos/`);
  const focusedPlay = () => page.evaluate(() => document.activeElement.matches("a.video-play"));
  for (let presses = 0; presses < 20 && !await focusedPlay(); presses++) await page.keyboard.press("Tab");
  assert.equal(await focusedPlay(), true, "Tab reaches the first video");
  assert.notEqual(await page.evaluate(() => getComputedStyle(document.activeElement).outlineStyle), "none");
  await page.keyboard.press("Enter");
  // Space, which scrolls the page from a link, plays the next one instead.
  await page.locator("a.video-play").first().focus();
  const scrollY = await page.evaluate(() => scrollY);
  await page.keyboard.press("Space");
  assert.equal(await page.evaluate(() => scrollY), scrollY);
  const players = page.locator("main .video iframe");
  assert.deepEqual(await players.evaluateAll(nodes => nodes.map(node => [node.title, node.src])),
    videos.slice(0, 2).map(({ title, id }) => [title, playerUrl(id)]));
  assert.equal(await page.evaluate(() => document.activeElement.title), videos[1].title, "focus moves to the player");
});

for (const width of [320, 390, 1280]) {
  for (const javaScriptEnabled of [false, true]) {
    test(`the videos page fits ${width}px without horizontal scrolling (JavaScript: ${javaScriptEnabled})`, async (t) => {
      const page = await pageForTest(t, { javaScriptEnabled, viewport: { width, height: 900 } });
      await stubPlayer(page);
      await page.goto(`${origin}/videos/`);
      if (javaScriptEnabled) await page.locator("a.video-play").first().click();
      await assertFits(page, "main, header, .video, .video-frame", "text and videos fit their containers");
    });
  }
}

// A skill's own SKILL.md starts open, so it shows as soon as its skill does.
const skillMdIds = async page => new Set(await page.locator("main h4").evaluateAll(nodes =>
  nodes.filter(node => /^[^/]+\/SKILL\.md$/.test(node.textContent)).map(node => node.id)));
const openIds = page => page.locator("main details[open]").evaluateAll(nodes =>
  nodes.map(node => node.querySelector("summary :is(h2,h3,h4)").id));
// The folds whose opening shows anything at all.
const openSections = "main > section > .wrap > details[open]";

test("every headed section but a skill's SKILL.md starts closed, and each toggles independently by its heading", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/prompts/`);
  const headings = page.locator("main :is(h2, h3, h4)");
  assert.ok(await headings.count() > 30);
  const ids = await headings.evaluateAll(nodes => nodes.map(node => node.id));
  const skillMds = await skillMdIds(page);
  assert.ok(skillMds.size > 5);
  const isSkillMd = id => skillMds.has(id);
  for (const id of ids) {
    assert.equal(await page.locator(`#${id}`).evaluate(heading => {
      const summary = heading.closest("summary");
      return Boolean(summary && summary.parentElement.tagName === "DETAILS");
    }), true, `${id} must control a section`);
    assert.equal(await page.locator(`#${id}`).evaluate(heading => heading.closest("details").open), isSkillMd(id), `${id} starts ${isSkillMd(id) ? "open" : "closed"}`);
  }
  assert.deepEqual(new Set(await openIds(page)), skillMds);
  assert.equal(await page.locator(openSections).count(), 0, "nothing shows before its section opens");
  for (const id of ids) {
    const heading = page.locator(`#${id}`);
    // Open only the containing sections, as a reader does before reaching a nested heading.
    const ancestors = await heading.evaluate(node => {
      const ids = [];
      for (let parent = node.closest("details")?.parentElement; parent; parent = parent.parentElement) {
        if (parent.tagName === "DETAILS") ids.unshift(parent.querySelector("summary :is(h2,h3,h4)").id);
      }
      return ids;
    });
    for (const parent of ancestors) {
      if (!await page.locator(`#${parent}`).evaluate(node => node.closest("details").open)) {
        await page.locator(`#${parent}`).click();
      }
    }
    // Fold a SKILL.md first, so every heading is tested opening then folding.
    if (isSkillMd(id)) await heading.click();
    await heading.click();
    assert.equal(await heading.evaluate(node => node.closest("details").open), true, id);
    const content = heading.locator("xpath=ancestor::summary/parent::details/*[not(self::summary)][1]");
    assert.equal(await content.isVisible(), true, `${id} reveals its content`);
    await heading.click();
    assert.equal(await heading.evaluate(node => node.closest("details").open), false, id);
    assert.equal(await content.isVisible(), false, `${id} hides its content`);
  }
  await page.locator("#command-title").click();
  await page.locator("#prompts-title").click();
  assert.equal(await page.locator("#command-title").evaluate(node => node.closest("details").open), true);
});

test("opening a skill shows its SKILL.md, which still folds by its heading", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/prompts/`);
  await page.locator("#skills-title").click();
  const skillMd = page.locator("#file-thirdshift-code-review-skill-md").locator("xpath=ancestor::details[1]/pre");
  assert.equal(await skillMd.isVisible(), false);
  await page.locator("#skill-thirdshift-code-review-title").click();
  assert.equal(await skillMd.isVisible(), true);
  // The supporting files stay folded.
  const others = page.locator("#skill-thirdshift-code-review h4:not(#file-thirdshift-code-review-skill-md)");
  assert.ok(await others.count() > 0);
  for (const heading of await others.all()) {
    assert.equal(await heading.evaluate(node => node.closest("details").open), false);
  }
  await page.locator("#file-thirdshift-code-review-skill-md").click();
  assert.equal(await skillMd.isVisible(), false);
});

test("fragment links reveal the prompt or supporting file and its containing sections", async (t) => {
  const page = await pageForTest(t);
  await page.goto(`${origin}/prompts/#prompt-resume`);
  assert.equal(await page.locator("#prompt-resume .job-text").isVisible(), true);
  await page.goto(`${origin}/prompts/`);
  await page.locator("#licence-title").click();
  await page.locator('a[href="#file-thirdshift-pr-credits-md"]').click();
  const credits = page.locator("#file-thirdshift-pr-credits-md").locator("xpath=ancestor::details[1]/pre");
  // The page reveals a new fragment on hashchange, which can fire after the click returns.
  await credits.waitFor({ state: "visible", timeout: 5000 });
  // A link to the current fragment must reveal a section the reader closed.
  await page.locator("#file-thirdshift-pr-credits-md").click();
  assert.equal(await credits.isVisible(), false);
  await page.locator('a[href="#file-thirdshift-pr-credits-md"]').click();
  assert.equal(await credits.isVisible(), true);
});

test("mobile readers can toggle headings with the keyboard without JavaScript", async (t) => {
  const page = await pageForTest(t, { javaScriptEnabled: false, viewport: { width: 390, height: 844 } });
  await page.goto(`${origin}/prompts/`);
  assert.equal(await page.locator(openSections).count(), 0);
  assert.deepEqual(new Set(await openIds(page)), await skillMdIds(page));
  const prompts = page.locator("#prompts-title").locator("xpath=parent::summary");
  await prompts.focus();
  await page.keyboard.press("Enter");
  assert.equal(await page.locator("#prompt-fresh-title").isVisible(), true);
  const fresh = page.locator("#prompt-fresh-title").locator("xpath=parent::summary");
  await fresh.focus();
  await page.keyboard.press("Space");
  assert.equal(await page.locator("#prompt-fresh .job-text").isVisible(), true);
  await page.keyboard.press("Space");
  assert.equal(await page.locator("#prompt-fresh .job-text").isVisible(), false);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
});
