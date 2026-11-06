import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const root = import.meta.dir;
const skillsRoot = resolve(root, "../../skills");
const skills: [string, string][] = [];
async function collect(directory: string, prefix = ""): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const name = prefix + entry.name;
    if (entry.isDirectory()) await collect(join(directory, entry.name), name + "/");
    else if (entry.isFile()) skills.push([name, await readFile(join(directory, entry.name), "utf8")]);
    else throw new Error(`Unsupported embedded skill entry: ${name}`);
  }
}
await collect(skillsRoot);
await mkdir(join(root, "build"), { recursive: true });
// A transient entrypoint lets the compiler embed every skill and supporting file
// without checking generated copies into the repository.
await writeFile(join(root, "build/entry.ts"),
  `import { main } from "../src/ticket.ts";\nconst skills: [string, string][] = ${JSON.stringify(skills)};\nawait main(skills);\n`);
const result = Bun.spawnSync([process.execPath, "build", "--compile", "--target=bun-linux-x64", "build/entry.ts", "--outfile", "build/thirdshift"], {
  cwd: root, stdout: "inherit", stderr: "inherit",
});
if (result.exitCode !== 0) process.exit(result.exitCode);
