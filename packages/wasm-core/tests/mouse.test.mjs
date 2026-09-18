import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createAgentBrowser } from "../pkg/index.js";

const wasmUrl = await readFile(new URL("../pkg/wasm/agent_browser_wasm_bg.wasm", import.meta.url));

async function fixture(options = {}) {
  const events = [];
  const evaluations = [];
  const calls = [];
  let failHeldMove = false;
  const agent = await createAgentBrowser({
    wasmUrl,
    ...options,
    transport: {
      async send(method, params = {}, sessionId) {
        calls.push({ method, params, sessionId });
        if (method === "Target.getTargets") return { targetInfos: [{ targetId: "page", type: "page" }] };
        if (method === "Target.attachToTarget") return { sessionId: "session" };
        if (method === "Input.dispatchMouseEvent") {
          events.push(params);
          if (failHeldMove && params.type === "mouseMoved" && params.buttons) throw new Error("Transport failed");
        }
        if (method === "Runtime.evaluate") {
          evaluations.push(params.expression);
          const value = params.expression.includes('const selector = "#source"')
            ? { x: 20, y: 30 } : { x: 240, y: 160 };
          return { result: { value } };
        }
        return {};
      },
    },
  });
  return { agent, events, evaluations, calls, failHeldMove: () => { failHeldMove = true; } };
}

async function run(agent, command) {
  const result = await agent.run(command);
  assert.equal(result.ok, true, result.stderr);
  return result;
}

test("reports the WASM package version for both strings and argv", async () => {
  const { agent } = await fixture();
  const { version } = JSON.parse(await readFile(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal((await run(agent, "--version")).stdout, `agent-browser ${version}`);
  assert.equal((await run(agent, ["--version"])).stdout, `agent-browser ${version}`);
});

test("instant movement stays opt-in-free and does not install a cursor", async () => {
  const { agent, events, evaluations } = await fixture();
  await run(agent, "mouse move 240 160");
  assert.deepEqual(events, [{ type: "mouseMoved", x: 240, y: 160, buttons: 0 }]);
  assert.equal(evaluations.length, 0);
});

test("human curves are deterministic, seeded, and end at exact coordinates", async () => {
  const a = await fixture();
  const b = await fixture();
  const c = await fixture();
  await run(a.agent, "mouse move 240 160 --human --seed 42");
  await run(b.agent, "mouse move 240 160 --human --seed 42");
  await run(c.agent, "mouse move 240 160 --human --seed 43");
  assert.deepEqual(a.events, b.events);
  assert.notDeepEqual(a.events, c.events);
  assert.ok(a.events.length > 2);
  assert.deepEqual(a.events.at(-1), { type: "mouseMoved", x: 240, y: 160, buttons: 0 });
  assert.ok(a.events.slice(0, -1).some(({ x, y }) => Math.abs(y - x * 2 / 3) > 0.1));
});

test("human defaults can be overridden and do not leak between agents", async () => {
  const human = await fixture({ inputMode: "human" });
  const instant = await fixture();
  await run(human.agent, "mouse move 240 160");
  assert.ok(human.events.length > 2);
  await run(instant.agent, "mouse down");
  assert.deepEqual(instant.events.at(-1), { type: "mousePressed", x: 0, y: 0, button: "left", buttons: 1, clickCount: 1 });
  human.events.length = 0;
  await run(human.agent, ["mouse", "move", "300", "200", "--input-mode", "instant"]);
  assert.equal(human.events.length, 1);
});

test("explicit steps clamp safely and invalid options fail", async () => {
  const { agent, events } = await fixture();
  await run(agent, "mouse move 10 20 --steps 0");
  assert.equal(events.length, 1);
  events.length = 0;
  await run(agent, "mouse move 20 30 --steps 9999");
  assert.equal(events.length, 240);
  assert.equal((await agent.run("mouse move 1 2 --duration -1")).ok, false);
  assert.equal((await agent.run("mouse move 1 2 --input-mode invalid")).ok, false);
  await assert.rejects(fixture({ inputMode: "invalid" }), /inputMode must/);
});

test("held buttons and coordinates persist across mouse commands", async () => {
  const { agent, events } = await fixture();
  await run(agent, "mouse move 20 30");
  await run(agent, "mouse down left");
  await run(agent, "mouse down right");
  await run(agent, "mouse move 80 90 --steps 3");
  assert.ok(events.slice(-3).every(({ buttons }) => buttons === 3));
  await run(agent, "mouse up left");
  assert.equal(events.at(-1).buttons, 2);
  await run(agent, "mouse wheel 100 20");
  assert.deepEqual(events.at(-1), { type: "mouseWheel", x: 80, y: 90, buttons: 2, deltaX: 20, deltaY: 100 });
  await run(agent, "mouse up right");
  assert.equal(events.at(-1).buttons, 0);
});

test("cursor updates follow each successful mouse command in its CDP session", async () => {
  const { agent, events, evaluations, calls } = await fixture({ cursor: true });
  await run(agent, "click #target --human");
  assert.ok(events.length > 4);
  assert.deepEqual(events.slice(-2).map(({ type, buttons }) => [type, buttons]), [
    ["mousePressed", 1], ["mouseReleased", 0],
  ]);
  const scripts = evaluations.filter((script) => script.includes("agent-browser.wasm.cursor"));
  assert.equal(scripts.length, events.length);
  assert.ok(!scripts[0].includes("isTrusted"));
  for (const [index, call] of calls.entries()) {
    if (call.method !== "Input.dispatchMouseEvent") continue;
    assert.equal(calls[index + 1].method, "Runtime.evaluate");
    assert.equal(calls[index + 1].sessionId, call.sessionId);
    const { type, x, y, buttons = 0 } = call.params;
    assert.ok(calls[index + 1].params.expression.includes(JSON.stringify({ type, x, y, buttons })));
  }
  await run(agent, "mouse move 20 30");
  assert.equal(evaluations.filter((script) => script.includes("agent-browser.wasm.cursor")).length, events.length);
});

test("rejected mouse commands do not produce cursor feedback", async () => {
  const { agent, evaluations, failHeldMove } = await fixture({ cursor: true });
  await run(agent, "mouse down");
  const count = evaluations.length;
  failHeldMove();
  assert.equal((await agent.run("mouse move 20 30")).ok, false);
  assert.equal(evaluations.length, count);
});

test("drag keeps the button held through the curve and releases at the target", async () => {
  const { agent, events } = await fixture();
  await run(agent, "drag #source #target --human");
  const down = events.findIndex(({ type }) => type === "mousePressed");
  assert.ok(down > 1);
  assert.ok(events.slice(down + 1, -1).every(({ type, buttons }) => type === "mouseMoved" && buttons === 1));
  assert.deepEqual(events.at(-1), { type: "mouseReleased", x: 240, y: 160, button: "left", buttons: 0, clickCount: 1 });
});

test("failed drag still releases the held button", async () => {
  const { agent, events, failHeldMove } = await fixture();
  failHeldMove();
  const result = await agent.run("drag #source #target --human");
  assert.equal(result.ok, false);
  assert.match(result.stderr, /Transport failed/);
  assert.equal(events.at(-1).type, "mouseReleased");
  assert.equal(events.at(-1).buttons, 0);
});
