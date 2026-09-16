import { guardObservations } from "./observation.js";
import { scopeOf } from "@deepseek-ai/dsh-scope";
import { isAbsolute } from "node:path";
import { RequestError } from "@agentclientprotocol/sdk";
import * as mcp from "./mcp-client.js";

const invalid = (message) => RequestError.invalidParams(undefined, message);

// ACP sends executable capabilities, not shell strings. Validate the entire
// batch before spawning anything; accept only the stdio transport Buzz supplies.
export function validateServers(servers = []) {
  if (!Array.isArray(servers) || servers.length > 8)
    throw invalid("at most 8 MCP servers are supported");
  const names = new Set();
  return servers.map((server) => {
    if (server.type && server.type !== "stdio")
      throw invalid("only stdio MCP is supported");
    if (!/^[A-Za-z0-9_-]{1,32}$/.test(server.name) || names.has(server.name))
      throw invalid("invalid or duplicate MCP server name");
    names.add(server.name);
    if (typeof server.command !== "string" || !isAbsolute(server.command))
      throw invalid("MCP command must be absolute");
    if (
      !Array.isArray(server.args) ||
      server.args.length > 128 ||
      server.args.some((arg) => typeof arg !== "string" || arg.includes("\0"))
    )
      throw invalid("invalid MCP args");
    if (!Array.isArray(server.env) || server.env.length > 128)
      throw invalid("invalid MCP env");
    const env = Object.create(null);
    for (const entry of server.env) {
      if (
        !/^[A-Za-z_][A-Za-z0-9_]*$/.test(entry.name) ||
        typeof entry.value !== "string" ||
        entry.value.includes("\0") ||
        Object.hasOwn(env, entry.name)
      )
        throw invalid("invalid or duplicate MCP env");
      env[entry.name] = entry.value;
    }
    return {
      transport: "stdio",
      serverName: server.name,
      command: server.command,
      args: [...server.args],
      env,
    };
  });
}

export async function setupSession(ctx, servers, cwd, route) {
  const owner = scopeOf(ctx);
  if (!owner) throw new Error("ACP MCP setup requires an agent scope");
  ctx.tools.guard((exec) =>
    exec.name.startsWith("mcp__") && exec.agent !== owner
      ? "MCP tools belong to the exact ACP agent"
      : undefined,
  );
  guardObservations(ctx, owner);
  ctx.on("agent/request", async (_payload, next) => ({
    ...(await next()),
    ...route,
  }));
  for (const server of servers) {
    const plugin = ctx.plugin(mcp, {
      ...server,
      cwd,
      toolCallTimeoutMs: 60000,
      failOnStartupError: true,
      reconnect: { enabled: false },
    });
    await plugin;
  }
}

// Advertise the one explicitly configured model. Unknown identifiers never
// fall through to the provider's permissive resolveModel fallback.
export function modelOptions(model) {
  return [
    {
      id: "model",
      name: "Model",
      category: "model",
      type: "select",
      currentValue: model,
      options: [{ value: model, name: model }],
    },
  ];
}
export function selectModel(record, params, allowed) {
  if (record.closing || record.inflight)
    throw invalid("model selection requires an idle session");
  if (params.configId !== "model" || params.value !== allowed)
    throw invalid("unknown model configuration");
  record.route.model = allowed;
  return { configOptions: modelOptions(allowed) };
}

export function closeRecord(ctx, record) {
  if (record.closing) return record.closing;
  record.closing = (async () => {
    const inflight = record.inflight;
    if (inflight) {
      inflight.cancelRequested = true;
      inflight.admissionController.abort(new Error("ACP session closed"));
      await inflight.admissionDone;
    }
    record.agent.cancel({ kind: "user" });
    await record.agent.whenIdle();
    await record.outputTail;
    await ctx.get("subagents")?.drainContinuableDescendants([record.agent]);
    await record.dispose();
  })();
  return record.closing;
}

export function installProgress(ctx, ownedRecord, notify) {
  const queue = (record, update) => {
    record.outputTail = record.outputTail
      .then(() => notify({ sessionId: record.agent.id, update }))
      .catch((error) => {
        if (record.inflight) record.inflight.outputError ??= error;
      });
  };
  ctx.on("tools/execute", async (exec, next) => {
    const record = exec.agent && ownedRecord(exec.agent);
    if (record)
      queue(record, {
        sessionUpdate: "tool_call",
        toolCallId: exec.callId,
        title: exec.name,
        kind: "other",
        status: "in_progress",
      });
    return next();
  });
  ctx.on("tools/result", (exec, result) => {
    const record = exec.agent && ownedRecord(exec.agent);
    if (record)
      queue(record, {
        sessionUpdate: "tool_call_update",
        toolCallId: exec.callId,
        status: result.isError ? "failed" : "completed",
      });
  });
}
