// Disposable protocol fixture; no browser, native input, files, or network.
import readline from "node:readline";
import { spawn } from "node:child_process";
const grandchild = process.env.MCP_FIXTURE_GRANDCHILD
  ? spawn(
      process.execPath,
      ["-e", "process.on('SIGTERM',()=>{}); setInterval(()=>{},1000)"],
      { stdio: "ignore" },
    )
  : undefined;
const input = readline.createInterface({ input: process.stdin });
input.on("line", (line) => {
  const r = JSON.parse(line);
  if (r.id === undefined) return;
  let result;
  if (r.method === "initialize")
    result = {
      protocolVersion: r.params.protocolVersion,
      capabilities: { tools: { listChanged: true } },
      serverInfo: { name: "assessment-fixture", version: "1" },
    };
  else if (r.method === "tools/list")
    result = {
      tools: [
        {
          name: "observe",
          description: "Fixture observation",
          inputSchema: {
            type: "object",
            properties: { mode: { type: "string" } },
          },
        },
      ],
      ...(process.env.MCP_FIXTURE_LIST_LOOP ? { nextCursor: "repeat" } : {}),
    };
  else if (r.method === "tools/call") {
    const mode = r.params.arguments.mode;
    if (mode === "hang") return;
    const structuredContent = {
      marker: "STRUCTURED_ONLY_7D",
      pid: process.pid,
      ...(grandchild ? { grandchildPid: grandchild.pid } : {}),
      inheritedSecretPresent:
        process.env.ASSESSMENT_SECRET_SENTINEL !== undefined,
      explicitTokenPresent:
        process.env.ASSESSMENT_TOKEN_EXPLICIT === "synthetic-only",
    };
    result = {
      content:
        mode === "structured"
          ? []
          : mode === "image"
            ? [
                { type: "text", text: "before" },
                { type: "image", mimeType: "image/png", data: "AA==" },
                { type: "text", text: "after" },
              ]
            : [
                {
                  type: "text",
                  text: mode === "error" ? "fixture error" : "fixture text",
                },
              ],
      structuredContent,
      ...(mode === "error" ? { isError: true } : {}),
    };
  } else result = {};
  process.stdout.write(
    JSON.stringify({ jsonrpc: "2.0", id: r.id, result }) + "\n",
  );
});
