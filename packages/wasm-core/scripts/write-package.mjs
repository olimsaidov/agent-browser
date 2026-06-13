import { copyFile, mkdir, rm, writeFile } from "node:fs/promises";
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

const packageJson = {
  name: "@olimsaidov/agent-browser-wasm",
  version: "0.27.3",
  description: "Run agent-browser in a browser with a supplied CDP transport",
  type: "module",
  license: "Apache-2.0",
  repository: {
    type: "git",
    url: "git+https://github.com/olimsaidov/agent-browser.git",
    directory: "packages/wasm-core",
  },
  files: ["index.js", "index.d.ts", "README.md", "LICENSE", "wasm/*"],
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
