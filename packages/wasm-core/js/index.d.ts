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
};

export function createAgentBrowser(
  options: CreateAgentBrowserOptions,
): Promise<AgentBrowser>;
