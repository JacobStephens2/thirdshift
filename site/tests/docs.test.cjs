const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFile, stat } = require("node:fs/promises");
const { resolve, dirname } = require("node:path");

test("the Setup defaults example includes the Trial and Grader caps", async () => {
  const { marked } = await import("marked");
  const source = await readFile(resolve(__dirname, "../../docs/guide.md"), "utf8");
  const defaults = marked.lexer(source).find(token => token.type === "code" && token.lang === "toml" && token.text.includes("always = false"));
  assert.ok(defaults, "the manual includes the complete Setup defaults");
  for (const setting of ["[trial]", 'memory_high = "1G"', 'memory_max = "2G"', 'memory_swap_max = "1G"']) {
    assert.ok(defaults.text.includes(setting), `Setup defaults lack ${setting}`);
  }
});

test("notification documentation limits suppression to child Ticket runs", async () => {
  const glossary = (await readFile(resolve(__dirname, "../../GLOSSARY.md"), "utf8"))
    .split("\n**Run notification**:\n")[1].split("\n_Avoid_:")[0].replaceAll("**", "");
  assert.doesNotMatch(glossary, /and a Ticket run never sends one of its own/,
    "Notification suppression must be scoped to Ticket runs inside a Spec run");
  for (const file of ["../../docs/guide.md", "../docs/index.html"]) {
    const manual = await readFile(resolve(__dirname, file), "utf8");
    assert.doesNotMatch(manual, /dispatched Ticket runs, Ticket runs and Base fixes write none of their own/,
      "Notification sidecar suppression must be scoped to child Ticket runs");
  }
});

test("Session trace documentation does not advertise retired conversion commands", async () => {
  for (const file of ["../../docs/guide.md", "../docs/index.html"]) {
    const source = await readFile(resolve(__dirname, file), "utf8");
    assert.doesNotMatch(source, /thirdshift logs --(?:convert|remove-converted)\b/, file);
  }
});

test("the Linux-only manual does not direct native non-Linux Console execution", async () => {
  for (const file of ["../../docs/guide.md", "../docs/index.html"]) {
    const manual = await readFile(resolve(__dirname, file), "utf8");
    assert.doesNotMatch(manual, /On other operating systems, run/,
      `${file}: Linux-only documentation still directs native non-Linux execution`);
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
  for (const anchor of ["unattended-sessions", "security-review", "logs", "spec-runs",
    "what-a-run-does", "foreign-commits-in-a-merge-run", "failed-runs", "weeding", "fencing"]) {
    assert.ok(anchors.has(anchor), `existing README bookmark #${anchor}`);
  }
});
