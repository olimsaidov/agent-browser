# @olimsaidov/agent-browser-wasm

Run the agent-browser command engine in a browser with a supplied CDP transport.

```ts
import { createAgentBrowser } from "@olimsaidov/agent-browser-wasm";

const agent = await createAgentBrowser({
  transport: {
    send(method, params, sessionId) {
      return cdp.send(method, params, sessionId);
    },
  },
});

const result = await agent.run("snapshot -i");
console.log(result.stdout);
```

`agent.run()` accepts either a shell-like command string or an already-tokenized
argv array:

```ts
await agent.run("fill @e1 Olim");
await agent.run(["fill", "@e1", "Olim"]);
```

Do not include the `agent-browser` binary name. The library receives commands as
the CLI arguments that would normally follow the binary name.
