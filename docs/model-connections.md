# Model connections

Settings → Agents → Model connections connects a model server once for local
DeepSeek and Prime agents. Add connection automatically searches this computer,
local service advertisements and common model ports. Manual setup is always
available. The optional private-network switch reads online Tailscale peers;
it does not scan the overlay address range. Discovery never installs software
or runs inference. Authentication-required candidates remain unverified until
Connect validates their model catalog.

Select Saved model connection in an agent's model source/provider field, select
a connection, then a model. A supported harness change preserves that choice.
Legacy DeepSeek endpoints can be saved as reusable connections from their editor.
Connection edits apply when an agent next starts. Running sessions are not
rerouted. Removing a connection blocks its future use rather than substituting
a different server. Remote provider-backed agents cannot use device-local
connections; they need configuration on their own execution host.

## Boundaries

- Catalog discovery supports OpenAI-compatible `/models` endpoints; manual root
  addresses receive `/v1`, while explicit API paths are preserved.
- Scans check ports 11434, 1234, 8000, 8010 and 8080, plus advertised service ports.
  They search at most two local /24-sized sets, 128 advertised candidates, and
  128 known online Tailscale peers, with bounded concurrency and timeouts.
  Custom ports, isolated networks and IPv6-only discovery may need manual setup.
- HTTP redirects are disabled. Discovery sends no credentials. User-supplied
  keys go only to the explicitly chosen endpoint. Responses are capped at 1 MiB.
- Persistent connections live in the device app-data directory, with atomic
  file replacement. API keys live in the existing OS-backed SecretStore;
  neither keys nor their storage references appear in public connection IPC.
- Agent configuration stores the stable connection ID, model and provider.
  Local native spawn resolves the current connection. DeepSeek receives its
  native environment; Prime receives an owned per-agent config directory with
  a key environment reference. No global Prime configuration is overwritten.
- Availability checks fetch catalogs only. Test a model sends one short text
  request; a text test does not establish tool-calling or harness compatibility.
  No model is selected automatically merely because a catalog is reachable.
- The initial Prime adapter uses conservative 32K context / 4K output defaults
  and disables unsupported developer-role and reasoning-effort request fields.
  Model-specific advanced tuning and an optional server advertising helper are
  separate extensions, not required for the supported connection workflow.

## Verification

Native tests cover URL normalization, catalog-only discovery, credential routing,
redirect refusal, malformed/oversized responses, explicit inference checks,
private-network bounds, persistent references and private file permissions.
A deliberately ignored live test runs bounded network discovery when explicitly
requested. Browser tests cover automatic connection, model selection and harness
switching, manual failure/retry, availability retry, explicit test and removal,
and the existing manual DeepSeek flow. See the task verification report for exact
commits, current gate results and installed acceptance evidence.
