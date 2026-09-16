import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
const require = createRequire(import.meta.url);
const expected = "0.1.1-rc.2";
// Do not silently run this private bridge against a different engine contract.
export function assertCompatibility() {
  for (const name of [
    "dsh-acp",
    "dsh-acp-demo",
    "dsh-mcp-client",
    "dsh-agent-loop",
    "dsh-tools",
  ]) {
    const entry = require.resolve(`@deepseek-ai/${name}`);
    const manifest = JSON.parse(
      readFileSync(join(dirname(entry), "../package.json"), "utf8"),
    );
    if (manifest.version !== expected)
      throw new Error(
        `Buzz ACP requires ${name}@${expected}; found ${manifest.version}`,
      );
  }
}
