// A pipe held only by dsh is the lifetime capability. If dsh crashes (even
// SIGKILL), EOF still reaches this process, which terminates its owned group.
import { spawn } from "node:child_process";
import { createReadStream } from "node:fs";
const child = spawn(process.argv[2], process.argv.slice(3), {
  env: process.env,
  stdio: ["inherit", "inherit", "inherit"],
  shell: false,
});
let stopping = false;
function stop() {
  if (stopping) return;
  stopping = true;
  setTimeout(() => {
    process.kill(-process.pid, "SIGKILL");
  }, 250);
  process.kill(-process.pid, "SIGTERM");
}
process.on("SIGTERM", stop);
process.on("SIGINT", stop);
child.once("error", stop);
child.once("exit", stop);
const lifeline = createReadStream(null, { fd: 3 });
lifeline.on("end", stop);
lifeline.on("error", stop);
lifeline.resume();
