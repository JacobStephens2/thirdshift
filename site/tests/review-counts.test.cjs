const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFile } = require("node:fs/promises");
const { join } = require("node:path");

test("published review counts exclude declined observations", async () => {
  const root = join(__dirname, "..", "..");
  for (const path of ["docs/guide.md", "site/docs/index.html", "site/how-it-works/index.html"]) {
    const text = await readFile(join(root, path), "utf8");
    assert.ok(!text.includes("including findings later fixed or declined"), `${path}: stale count contract`);
    assert.ok(text.includes("actionable findings"), `${path}: actionable findings missing`);
    assert.ok(text.includes("observations that require no change"), `${path}: no-action exclusion missing`);
  }
});
