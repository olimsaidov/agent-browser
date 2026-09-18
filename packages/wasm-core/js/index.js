import initWasm, {
  run_argv,
  run_command,
} from "./wasm/agent_browser_wasm.js";
import { cursorScript } from "./cursor.js";

const defaultWasmUrl = new URL("./wasm/agent_browser_wasm_bg.wasm", import.meta.url);

let wasmReady = null;

function ensureWasm(wasmUrl) {
  wasmReady ??= initWasm({ module_or_path: wasmUrl ?? defaultWasmUrl }).catch((error) => {
    wasmReady = null;
    throw error;
  });
  return wasmReady;
}

function parseOutput(text) {
  const output = JSON.parse(text);
  const exitCode = Number(output.exitCode ?? 1);
  return {
    stdout: String(output.stdout ?? ""),
    stderr: String(output.stderr ?? ""),
    exitCode,
    ok: exitCode === 0,
  };
}

function normalizeParams(paramsJson) {
  if (!paramsJson) return {};
  const value = JSON.parse(paramsJson);
  return value && typeof value === "object" && !Array.isArray(value) ? value : {};
}

function createRawTransport(transport, cursor) {
  let cursorReady = false;
  return async (method, paramsJson, sessionId) => {
    // Each command reconnects. Re-check after navigation or page-side cleanup.
    if (method === "Runtime.enable" || method === "Page.navigate") cursorReady = false;
    if (cursor && method === "Input.dispatchMouseEvent" && !cursorReady) {
      const result = await transport.send("Runtime.evaluate", {
        expression: cursorScript,
        returnByValue: true,
      }, sessionId || undefined);
      if (result?.exceptionDetails) throw new Error("Could not initialize the cursor overlay");
      cursorReady = true;
    }
    const result = await transport.send(
      method,
      normalizeParams(paramsJson),
      sessionId || undefined,
    );
    return JSON.stringify(result ?? {});
  };
}

function assertCommandArray(command) {
  if (!Array.isArray(command)) {
    throw new TypeError("agent.run(command) requires a string or string[]");
  }

  for (const part of command) {
    if (typeof part !== "string") {
      throw new TypeError("agent.run(command) array entries must be strings");
    }
  }
}

export async function createAgentBrowser(options) {
  if (!options || !options.transport || typeof options.transport.send !== "function") {
    throw new TypeError("createAgentBrowser requires a transport with send(method, params, sessionId)");
  }
  if (options.inputMode !== undefined && !["instant", "smooth", "human"].includes(options.inputMode)) {
    throw new TypeError("inputMode must be instant, smooth, or human");
  }

  await ensureWasm(options.wasmUrl);
  const rawTransport = createRawTransport(options.transport, options.cursor === true);
  const defaultArgs = options.inputMode ? ["--input-mode", options.inputMode] : [];

  return {
    async run(command) {
      if (typeof command === "string") {
        return parseOutput(await run_command([...defaultArgs, command].join(" "), rawTransport));
      }

      assertCommandArray(command);
      return parseOutput(await run_argv(JSON.stringify([...defaultArgs, ...command]), rawTransport));
    },
  };
}
