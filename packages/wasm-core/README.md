# @olimsaidov/agent-browser-wasm

Run the agent-browser command engine in a browser with a supplied CDP transport.

```ts
import { createAgentBrowser } from "@olimsaidov/agent-browser-wasm";

const agent = await createAgentBrowser({
  cursor: true,
  inputMode: "human",
  transport: {
    send(method, params, sessionId) {
      return cdp.send(method, params, sessionId);
    },
  },
});

const result = await agent.run("snapshot -i");
console.log(result.stdout);
```

`agent.run()` accepts either a shell-like command string or an already-tokenized argv array:

```ts
await agent.run("fill @e1 Olim");
await agent.run(["fill", "@e1", "Olim"]);
```

Do not include the `agent-browser` binary name. The library receives commands as the CLI arguments that would normally follow the binary name.

## Mouse movement and cursor feedback

The package uses the command parser, mouse path interpolation, and cursor artwork from agent-browser 0.38.1. Set `cursor: true` to display the pointer and click ripples in the controlled page. Set `inputMode` to `"instant"` (the default), `"smooth"`, or `"human"` to choose the default mouse movement. These options are independent and both are opt-in.

```ts
await agent.run("mouse move 240 160 --human --seed 42");
await agent.run("click #submit --human");
await agent.run("drag #source #target --human");
await agent.run(["mouse", "move", "400", "200", "--duration", "300", "--steps", "24"]);
```

Mouse position and held buttons persist between commands for each agent instance. Human mode uses reproducible eased curves with exact endpoints and elapsed-time scheduling. The cursor overlay also observes synthetic pointer events from in-page CDP transports; it does not modify event trust.

This is a browser-hosted command adapter, not the native daemon. Browser/process management, filesystem operations, and recording/encoding commands remain unavailable. The live cursor option is not video recording, and a matching upstream version does not imply that every native command is supported.
