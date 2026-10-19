const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFile, readdir, stat } = require("node:fs/promises");
const { resolve, dirname } = require("node:path");

test("Markdown documentation links resolve in a repository checkout", async () => {
  const { Marked } = await import("marked");
  const { gfmHeadingId } = await import("marked-gfm-heading-id");
  const repo = resolve(__dirname, "../..");
  const guide = resolve(repo, "docs/guide");
  const files = [resolve(repo, "README.md"), ...(await readdir(guide)).filter(name => name.endsWith(".md")).map(name => resolve(guide, name))];
  for (const file of files) {
    const links = [];
    const marked = new Marked({ walkTokens(token) {
      if (token.type === "link" || token.type === "image") links.push(token.href);
    } });
    marked.parse(await readFile(file, "utf8"));
    for (const href of links) {
      if (/^(?:[a-z][a-z\d+.-]*:|\/)/i.test(href)) continue;
      const [path, fragment] = href.split("#");
      const target = path ? resolve(dirname(file), path) : file;
      assert.ok(await stat(target), `${file}: ${href}`);
      if (fragment && target.endsWith(".md")) {
        const html = new Marked(gfmHeadingId()).parse(await readFile(target, "utf8"));
        assert.ok(html.includes(`id="${fragment}"`), `${file}: ${href}`);
      }
    }
  }
});
