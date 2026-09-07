# Buzz computer use

Host-local MCP service and a live browser viewer. The current implementation is a
Chromium image stream of the **same page** controlled by the agent, with human
input forwarded to that page. It is not a native embedded Chromium widget.

## Operation

The agent CLI exposes `buzz computer status` and `buzz computer mcp`; both run
locally without relay credentials. Buzz enables the service at the managed ACP
launch boundary and adds its MCP
server to the existing effective configuration. New connections receive tool
descriptions, operating instructions, structured observations and native MCP image
blocks. Browser sessions belong to one connection; no relay signing keys are
forwarded. Provider image delivery must be proven per adapter, separately from
these service tests.

The Browser panel enables tools on the next managed-agent start and displays private
loopback viewers. Restart existing agents after changing capability settings;
opening another task in an already-running worker is insufficient. Human takeover fences agent input; returning control is
explicit. Each page has an opaque session ID, control/navigation generation and
observation-scoped element handles. The default context is ephemeral, with no
personal Chrome state inherited. There are four pages per connection and eight
pages across the host registry. Closing the final page shuts down its engine.
Idle expiry is 15 minutes, absolute expiry is two hours, and MCP EOF closes all
owned contexts. Browser contexts isolate web storage, not hostile OS processes.

Native control is a separate opt-in, initially macOS only. Buzz directly launches
the pinned CuaDriver daemon with embedded attribution, standard permission mode,
a private socket and a parent-liveness pipe. MCP proxies do not launch a daemon
under their own identity. Backend telemetry is disabled. macOS grants belong to
the installed Buzz application; compilation and permission preflight do not
prove successful screen capture.

Agents call `native_acquire` before native observation or action and
`native_release` after the workflow. One workflow owns native input per daemon
generation. An idle native lease expires after one minute when no operation is
running; reacquisition requires a fresh observation. A timeout or dead owner
requires disabling and re-enabling native
control in Buzz: ownership is not reassigned while an action may still be running.
Restart affected agent sessions after a driver reset. Native tools preserve the
backend's actual image blocks and target/window contracts. Configuration, updates,
recording, force-kill and personal Chrome attachment are not exposed by this bridge.

## Build and tests

Activate the repository Hermit environment before running pnpm or Cargo.

- `node --test computer-use/test/*.test.mjs` runs real local Chromium integration tests.
- `BUZZ_TEST_NATIVE_DRIVER=/absolute/cua-driver node --test computer-use/test/native.test.mjs`
  also checks the real native MCP surface, contention and parent EOF lifecycle.
  It makes no native UI mutations and does not establish installed Buzz TCC attribution.
- `scripts/bundle-computer-use.sh aarch64-apple-darwin` stages production JS
  dependencies, Chromium and checksum-pinned CuaDriver 0.23.2 (upstream prerelease).
- `just desktop-release-build` includes this staging step. Use the repository's
  canonical installer for installed acceptance; never launch a target-directory app.

## Remaining acceptance and scope

This is an opt-in implementation under development. Do not label full desktop
parity or all adapters supported until installed acceptance proves model image
receipt, native capture and action, actual Buzz viewer behavior, cancellation,
crash cleanup and packaging. Named cookie, local storage and IndexedDB profiles
have owner isolation and an exclusive writer. Download/upload controls,
managed popups, rich clipboard/IME and durable task evidence are not implemented
by the first browser surface. Unsolicited popup pages are currently closed.
