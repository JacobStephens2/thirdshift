// Compile trusted repository Markdown to checked-in static pages. Readers and
// the publishing server need neither JavaScript nor a Markdown runtime.
import { readFile, readdir, mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked } from "marked";
import { gfmHeadingId } from "marked-gfm-heading-id";

const site = dirname(fileURLToPath(import.meta.url));
const repo = resolve(site, "..");
const sources = resolve(repo, "docs/guide");
const files = (await readdir(sources)).filter(name => name.endsWith(".md")).sort();
const routes = new Map(files.map(name => [resolve(sources, name), name === "README.md" ? "/docs/" : `/docs/${name.slice(0, -3)}/`]));
const check = process.argv.includes("--check");
const escape = text => text.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

for (const name of files) {
  const source = resolve(sources, name);
  const markdown = await readFile(source, "utf8");
  const title = markdown.match(/^# (.+)$/m)[1];
  const marked = new Marked(gfmHeadingId(), {
    walkTokens(token) {
      if (token.type !== "link" && token.type !== "image") return;
      if (/^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(token.href)) return;
      const [path, fragment] = token.href.split("#");
      const target = resolve(dirname(source), path);
      token.href = (routes.get(target) ?? `https://github.com/JacobStephens2/thirdshift/blob/main/${relative(repo, target)}`) + (fragment ? `#${fragment}` : "");
    },
  });
  const html = `<!doctype html>
<!-- Generated from docs/guide/${name}; edit Markdown and run npm run docs in site/. -->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)} · thirdshift</title>
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/styles.css">
<link rel="stylesheet" href="/docs/docs.css">
</head>
<body>
<header class="docs-header"><nav aria-label="Site"><a href="/">thirdshift / home</a> · <a href="/docs/">Documentation</a> · <a href="/how-it-works/">How it works</a> · <a href="/prompts/">Prompts and skills</a></nav></header>
<main class="docs-content">
${marked.parse(markdown)}</main>
<footer class="docs-footer"><a href="https://github.com/JacobStephens2/thirdshift/blob/main/docs/guide/${name}">Read or edit this page as Markdown on GitHub</a></footer>
</body>
</html>
`;
  const output = resolve(site, `.${routes.get(source)}index.html`);
  if (check) {
    const actual = await readFile(output, "utf8").catch(error => {
      if (error.code === "ENOENT") return null;
      throw error;
    });
    if (actual !== html) {
      console.error(`${relative(repo, output)} is stale; run npm run docs in site/`);
      process.exitCode = 1;
    }
  } else {
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, html);
  }
}
