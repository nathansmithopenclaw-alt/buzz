# DeepSeek model selection in Buzz

The DeepSeek preset exposes its model/provider capabilities through the same
Rust KnownAcpRuntime metadata used for saved configuration and launch. It stays
a preset for installation and catalog ordering. Its provider subset is transported
to every selection surface through the runtime catalog.

Choose **DeepSeek API** and enter a DeepSeek API key, or choose
**Local / OpenAI-compatible endpoint** and enter the API base URL. The local
endpoint token is optional. Models come from that endpoint's `/models` response;
Buzz does not invent model IDs or fall back to another provider. Provider changes
clear the dependent model selection through the existing selection reducer.

Hosted routing always uses `https://api.deepseek.com` and `DEEPSEEK_API_KEY`.
Compatible routing uses `DSH_LLM_BASE_URL` and optional `DSH_API_KEY`. Selection
persists as `DSH_PROVIDER` and `DSH_MODEL` in the resolved launch environment.
The saved/form env layering remains authoritative, including explicit blank
values. Hosted discovery never uses the local endpoint or token. For a local
server without authentication, Buzz supplies the non-secret `local-no-auth`
placeholder when starting dsh because its upstream adapter requires a token.

Discovery calls the selected endpoint directly before a model has been selected.
Starting the model-dependent dsh profile for initial discovery would be circular.
Requests have a ten-second timeout, a one-MiB response limit, and no redirects.
HTTP errors include the status and setup guidance, not the untrusted response body
or URL. Empty and malformed catalogs remain failures with no provider fallback.

The retired local-v4 router is not a default. On September 8, the separately
verified local server was Spark 1 at `http://100.80.202.75:8010/v1`, advertising
`Qwen/Qwen3.8-27B`. This address is an operational example, not a baked default.
The other Spark's Flux/LoRA allocation is outside this feature's scope.

## Verification

Activate the repository toolchain with `. ./bin/activate-hermit` first.

- `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib dsh_`
- `cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib hosted_route`
- `pnpm --dir desktop test`
- `node --test desktop/src-tauri/src/managed_agents/dsh_profile_resources/tests/*.test.mjs`
- `just ci`
- Build E2E, acquire the shared suite lock, then run
  `BUZZ_E2E_PORT=4387 pnpm --dir desktop exec playwright test deepseek-model-selection.spec.ts --project=smoke --retries=0`.

The browser test is a UI/IPC fixture, the Rust discovery test uses a real HTTP
fixture, and the launcher test runs actual pinned dsh with scoped MCP subprocesses.
A separately authorized Qwen run validates local inference and tool result delivery.
None of these alone is installed desktop acceptance or hosted API account proof.

Official hosted API contracts: [model catalog](https://api-docs.deepseek.com/api/list-models)
and [OpenAI-compatible base URL](https://api-docs.deepseek.com/).
