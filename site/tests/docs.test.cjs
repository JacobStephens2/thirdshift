const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFile, stat } = require("node:fs/promises");
const { resolve, dirname } = require("node:path");

test("Session trace documentation does not advertise retired conversion commands", async () => {
  for (const file of ["../../docs/guide.md", "../docs/index.html"]) {
    const source = await readFile(resolve(__dirname, file), "utf8");
    assert.doesNotMatch(source, /thirdshift logs --(?:convert|remove-converted)\b/, file);
  }
});

test("the README and manual's repository links reach files and section anchors", async () => {
  const { marked } = await import("marked");
  const { default: GithubSlugger } = await import("github-slugger");
  const root = resolve(__dirname, "../..");
  async function document(path) {
    const source = await readFile(path, "utf8");
    const tokens = marked.lexer(source);
    const slugger = new GithubSlugger();
    const anchors = new Set([...source.matchAll(/<a name="([^"]+)"/g)].map(match => match[1]));
    marked.walkTokens(tokens, token => {
      if (token.type === "heading") anchors.add(slugger.slug(token.text));
    });
    return { tokens, anchors };
  }
  for (const file of ["README.md", "docs/guide.md"]) {
    const path = resolve(root, file);
    const { tokens } = await document(path);
    const links = [];
    marked.walkTokens(tokens, token => {
      if (token.type === "link" && !/^(?:[a-z][a-z\d+.-]*:|\/)/i.test(token.href)) links.push(token.href);
    });
    for (const link of links) {
      const [target, fragment] = link.split("#");
      const destination = target ? resolve(dirname(path), target) : path;
      assert.ok(await stat(destination), `${file}: ${link}`);
      if (fragment) assert.ok((await document(destination)).anchors.has(fragment), `${file}: ${link}`);
    }
  }

  // These README URLs were linked from the site before the manual moved.
  const { anchors } = await document(resolve(root, "README.md"));
  for (const anchor of ["unattended-sessions", "security-review", "logs", "spec-runs"]) {
    assert.ok(anchors.has(anchor), `existing README bookmark #${anchor}`);
  }
});
