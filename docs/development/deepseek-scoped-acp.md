# Owned DeepSeek Harness ACP composition

The Buzz `deepseek` preset provisions `buzz-acp` from embedded source. Its owned
`@buzz/dsh-acp` package is based on the MIT-licensed dsh ACP, ACP-demo composition,
and MCP-client modules at **0.1.1-rc.2**. `UPSTREAM.json` records original module
hashes and the upstream release commit; `LICENSE` preserves the upstream notice.
The agent loop, registry, tools, attachment store and persistence remain upstream
packages. Startup checks the tested engine package versions.

## Route configuration

Set `DSH_LLM_BASE_URL`, `DSH_MODEL`, and `DSH_API_KEY` in the managed process
configuration. The first two have no fallback. Credentials are read by the
provider at its normal request boundary; there is no credential-file materialization
in this package. The adapter remains the pinned DeepSeek-compatible adapter, so
an arbitrary OpenAI-compatible server is not automatically qualified.

`DSH_CONTEXT_WINDOW` defaults conservatively to 32768 and must be a positive
integer. Output is capped at 4096 tokens; thinking is disabled. For the verified
2026-09-08 text/tool test the explicit route was Spark 1,
`http://100.80.202.75:8010/v1`, model `Qwen/Qwen3.8-27B` (server limit 65536).
These are deployment observations, not hardcoded defaults. The retired `local-v4`
router must not be restored. Spark 2's recorded allocation is Flux/LoRA; this work
does not alter either host.

Inherited official web search and HMR are disabled. Model configuration advertises
only the explicit model and rejects unknown selections, including through Buzz's
legacy `session/set_model` method. Native tool mode is selected; delegation,
workflows, goals and Ralph loops remain opt-in. User Cordis/settings overrides
are trusted configuration and can still change the composition; they require
requalification. An edited private profile is not an OS security boundary.

## Session and process ownership

`session/new` validates the complete stdio MCP batch before spawning anything.
Each server is mounted during unpublished agent setup. Duplicate names fail in
one scope; sibling live-agent scopes may use identical names. Failed setup rolls
back. Inherited tools cannot authorize a child to use the parent's MCP grant.
There are at most eight sessions and eight servers per session, 16 discovery
pages and 256 tools per server, bounded initialization/list/call requests, and no
automatic reconnect in this composition.

Canonical structured results remain intact. Native rendering adds at most 32768
characters of structured JSON, avoids an equivalent JSON text mirror, and preserves
image diagnostics and ordered rich content. Excess structured output asks for a
smaller fresh observation. Compaction invalidates browser/native observation
permission until a successful fresh observation for the affected surface.
Host-side leases, takeover and generation validation remain authoritative.

Tool start/result updates are ordered with committed assistant messages. Prompt
cancellation is scoped. `session/close` cancels/drains the agent, descendants and
MCP clients before returning. Buzz identifies this owned bridge as `buzz-dsh-acp`
and waits up to ten seconds for cleanup; a missing/error receipt terminates the
adapter before subsequent dispatch can reuse it. Other adapters retain their
existing bounded-write close path.

Each stdio MCP server has a POSIX process-group supervisor and an owner-only pipe.
Normal close and owner SIGKILL both terminate ordinary descendants. Deliberate
server daemonization or hostile same-user code is outside this trusted-plugin
contract. Windows is explicitly unsupported by this transport.

## Reproduce source verification

Use Node 22.19+ and the pinned installed dsh dependency tree. A worktree-local,
ignored `dsh_profile_resources/node_modules` symlink can point to that dependency
tree for tests; do not edit the installed profile. Run from the repository root:

```sh
node --test desktop/src-tauri/src/managed_agents/dsh_profile_resources/tests/*.test.mjs
cargo test -p buzz-acp session_close --lib
just ci
```

The launcher test stages a disposable profile/home and drives the actual dsh
process over ACP. Default inference is a controlled loopback fixture; the MCP
servers are real disposable subprocesses. The scope test uses real Cordis and
ToolRuntime. The crash test kills its own transport owner and checks exact PIDs.
A fresh worktree must stage the existing computer-use bundle before Tauri checks:
`bash scripts/bundle-computer-use.sh`.

A separately authorized local-provider run uses:

```sh
DSH_TEST_ENDPOINT=http://100.80.202.75:8010/v1 \
DSH_TEST_MODEL=Qwen/Qwen3.8-27B \
node --test desktop/src-tauri/src/managed_agents/dsh_profile_resources/tests/launcher.test.mjs
```

This requests a bounded text/tool turn, not native input. It records the actual
provider-returned model, verifies the tool result in the next model request,
and checks progress and cleanup. It does not prove image understanding.

## Release boundaries

This is a source/package build. Installed Buzz rollout, browser/native user
acceptance, account-switch lease acceptance, and exact-route visual tests remain
separate gates. Publication durability/replay remains linked to existing issue
`28bf6227-72b9-44f2-a5a1-23cd0a3b1670`; this package does not upgrade the private
publisher's in-memory receipt behavior. Do not label this unattended messaging
acceptance, ACP resume support, or Code Mode qualification.
