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
await copyFile(join(packageRoot, "README.md"), join(outDir, "README.md"));
await copyFile(join(packageRoot, "..", "..", "LICENSE"), join(outDir, "LICENSE"));

// Supplied CDP transports may emit synthetic pointer events. This cosmetic
// overlay observes both kinds without changing the events' trust status.
const cursorSource = await readFile(join(packageRoot, "../../cli/src/native/recording-cursor.js"), "utf8");
const trustGuard = "!event.isTrusted || ";
if (!cursorSource.includes(trustGuard)) throw new Error("Upstream cursor event guard changed");
const cursorScript = `if (!globalThis.__agentBrowserRecordingCursorCleanup) {\n${cursorSource.replace(trustGuard, "")}\n}`;
await writeFile(join(outDir, "cursor.js"), `export const cursorScript = ${JSON.stringify(cursorScript)};\n`);

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
  files: ["index.js", "index.d.ts", "cursor.js", "README.md", "LICENSE", "wasm/*"],
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
