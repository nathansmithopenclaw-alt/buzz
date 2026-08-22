# Vendored ACP adapter (pinned Track-A fork)

`@agentclientprotocol/claude-agent-acp` at fork commit `5145251e` (Track-A
background-tasks extension; upstream base `ee9f300d`), packaged with `npm pack`.

- **Why vendored:** the harness spawns `claude-agent-acp` from PATH. A global npm
  install silently drifts from the source we verified (the exact
  installed-artifact-vs-source class this lane exists to close). The tarball here
  is pinned by sha256 in `PROVENANCE.json`; the runtime battery asserts the
  *installed* artifact's identity before any rollout is called green.
- **Install:** `scripts/install-vendored-acp-adapter.sh` — verifies the tarball
  hash, installs via npm from the `file:` pin in `package.json`, and writes a
  versioned shim into `~/.buzz/agents/bin/`. Never edit `node_modules` by hand.
- **Verify:** `scripts/verify-vendored-acp.sh` — re-checks the tarball hash and
  installed version, then runs the M3 live typed battery through the real
  consumer with `ACP_FORK_DIST` pointed at THIS vendored dist. Green means the
  installed bytes are the verified bytes, proven at runtime.
- **Rebuild:** see `rebuild` in `PROVENANCE.json`. A new fork commit means a new
  tarball, a new sha256, and a fresh PROVENANCE.json in the same change.
