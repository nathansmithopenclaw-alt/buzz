import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { fileURLToPath } from "node:url";
import { OwnedStdioTransport } from "../packages/dsh-acp-buzz/stdio.js";
const transport = new OwnedStdioTransport({
  command: process.execPath,
  args: [fileURLToPath(new URL("./mcp-fixture.mjs", import.meta.url))],
  cwd: process.cwd(),
  env: { MCP_FIXTURE_GRANDCHILD: "1" },
});
const client = new Client({ name: "crash-test", version: "1" });
await client.connect(transport, { timeout: 2000 });
const result = await client.callTool({
  name: "observe",
  arguments: { mode: "structured" },
});
process.stdout.write(
  `${JSON.stringify({ ...result.structuredContent, supervisorPid: transport.pid })}\n`,
);
