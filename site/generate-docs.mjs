// docs/guide.md is the source; deployment serves the checked-in HTML unchanged.
import { readFile, writeFile } from "node:fs/promises";
import { Marked } from "marked";
import GithubSlugger from "github-slugger";

const source = new URL("../docs/guide.md", import.meta.url);
const output = new URL("docs/index.html", import.meta.url);
const slugger = new GithubSlugger();
const sections = [];
const markdown = new Marked({
  renderer: {
    heading({ tokens, depth, text }) {
      const id = slugger.slug(text);
      const content = this.parser.parseInline(tokens);
      if (depth === 2) sections.push(`<li><a href="#${id}">${content}</a></li>`);
      return `<h${depth} id="${id}">${content}</h${depth}>\n`;
    },
  },
  walkTokens(token) {
    if (token.type !== "link" && token.type !== "image") return;
    // Fragment and external links stay intact. Repository paths are relative
    // to docs/guide.md, so resolve them there before linking back to GitHub.
    if (/^(?:[a-z][a-z\d+.-]*:|#|\/\/)/i.test(token.href)) return;
    token.href = new URL(token.href, "https://github.com/JacobStephens2/thirdshift/blob/main/docs/").href;
  },
});
const content = markdown.parse(await readFile(source, "utf8"));
const page = `<!doctype html>
<!-- Generated from docs/guide.md by site/generate-docs.mjs. Run: cd site && npm run docs -->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>thirdshift documentation · Install, configure and run coding agents</title>
<meta name="description" content="The thirdshift user manual: installation, commands, configuration, workflows, logs and scheduling.">
<link rel="canonical" href="https://thirdshift.app/docs/">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/styles.css">
<link rel="stylesheet" href="/docs/docs.css">
</head>
<body class="manual">
<a class="skip-manual" href="#thirdshift">Skip to the manual</a>
<header class="wrap">
  <nav aria-label="Site"><a href="/">thirdshift / home</a><a href="/how-it-works/">How thirdshift works</a><a href="/prompts/">Prompts and skills</a><a href="https://github.com/JacobStephens2/thirdshift/blob/main/docs/guide.md">Markdown source</a></nav>
  <p>This manual is published from the same Markdown you can read in the repository.</p>
  <nav aria-label="Manual sections"><ul>${sections.join("\n")}</ul></nav>
</header>
<main class="wrap">${content}</main>
</body>
</html>
`;

if (process.argv.slice(2).some(arg => arg !== "--check")) {
  throw new Error("Usage: node generate-docs.mjs [--check]");
}
if (process.argv.includes("--check")) {
  if (await readFile(output, "utf8") !== page) {
    throw new Error("Documentation is stale. Run: cd site && npm run docs");
  }
} else {
  await writeFile(output, page);
}
