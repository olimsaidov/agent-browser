import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const packageRoot = join(root, "..");
const outDir = join(packageRoot, "pkg");

await mkdir(outDir, { recursive: true });

await Promise.all([
  rm(join(outDir, "wasm", ".gitignore"), { force: true }),
  rm(join(outDir, "wasm", "package.json"), { force: true }),
  rm(join(outDir, "wasm", "README.md"), { force: true }),
]);

await copyFile(join(packageRoot, "js", "index.js"), join(outDir, "index.js"));
await copyFile(join(packageRoot, "js", "index.d.ts"), join(outDir, "index.d.ts"));
await copyFile(join(packageRoot, "js", "cursor.js"), join(outDir, "cursor.js"));
await copyFile(join(packageRoot, "README.md"), join(outDir, "README.md"));
await copyFile(join(packageRoot, "..", "..", "LICENSE"), join(outDir, "LICENSE"));

// Reuse upstream artwork only. Its recording event listeners must not run in
// the browser client, where human input is independent from agent commands.
const cursorSource = await readFile(join(packageRoot, "../../cli/src/native/recording-cursor.js"), "utf8");
const hostStyle = cursorSource.match(/host\.style\.cssText = '([^']+)';/)?.[1];
const markup = cursorSource.match(/shadow\.innerHTML = `([^`]+)`;/)?.[1];
if (!hostStyle || !markup || markup.includes("${")) throw new Error("Upstream cursor artwork changed");
await writeFile(join(outDir, "cursor-artwork.js"),
  `export const hostStyle = ${JSON.stringify(hostStyle)};\nexport const markup = ${JSON.stringify(markup)};\n`);

const packageJson = {
  name: "@olimsaidov/agent-browser-wasm",
  version: JSON.parse(await readFile(join(packageRoot, "package.json"), "utf8")).version,
  description: "Run agent-browser in a browser with a supplied CDP transport",
  type: "module",
  license: "Apache-2.0",
  repository: {
    type: "git",
    url: "git+https://github.com/olimsaidov/agent-browser.git",
    directory: "packages/wasm-core",
  },
  files: ["index.js", "index.d.ts", "cursor.js", "cursor-artwork.js", "README.md", "LICENSE", "wasm/*"],
  exports: {
    ".": {
      types: "./index.d.ts",
      import: "./index.js",
    },
    "./agent_browser_wasm_bg.wasm": "./wasm/agent_browser_wasm_bg.wasm",
  },
  main: "index.js",
  module: "index.js",
  types: "index.d.ts",
  sideEffects: ["./wasm/*"],
};

await writeFile(join(outDir, "package.json"), `${JSON.stringify(packageJson, null, 2)}\n`);
