export type CdpTransport = {
  send(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown>;
};

export type RunResult = {
  stdout: string;
  stderr: string;
  exitCode: number;
  ok: boolean;
};

export type AgentBrowser = {
  run(command: string | string[]): Promise<RunResult>;
};

export type CreateAgentBrowserOptions = {
  transport: CdpTransport;
  wasmUrl?: string | URL | Response | BufferSource | WebAssembly.Module;
  /** Show pointer and click feedback for this client's mouse commands only. */
  cursor?: boolean;
  /** Default mouse movement mode; individual commands can override it. */
  inputMode?: "instant" | "smooth" | "human";
};

export function createAgentBrowser(
  options: CreateAgentBrowserOptions,
): Promise<AgentBrowser>;
