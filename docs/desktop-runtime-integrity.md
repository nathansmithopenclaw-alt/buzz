# Desktop runtime integrity

Buzz Desktop, its bundled `buzz-acp` broker, and each agent adapter form one
runtime. A passing source build does not prove that the running app uses those
bytes.

## Build and install

1. Work from an isolated worktree at the intended commit.
2. Run `just ci`.
3. Run `just desktop-release-build`. This compiles every bundled sidecar from
   the same checkout, creates only the macOS app bundle, ad-hoc signs the local
   bundle as a complete unit, and runs `scripts/verify-desktop-bundle.sh`.
4. Quit Buzz normally. Do not kill it during an agent turn.
5. Run `just desktop-install-macos`. The installer refuses a live Desktop,
   retains the previous `/Applications/Buzz.app` as a hidden recoverable
   `.bundle` backup, consumes the target-directory `.app`, and installs the new
   bundle at `/Applications/Buzz.app`.
6. Launch with `open /Applications/Buzz.app`. Never launch a bundle below a
   checkout or Cargo target directory.

## Required evidence

Before claiming the new app is live, record all of the following from the same
launch:

- running Desktop executable path and SHA-256;
- bundled broker path and SHA-256;
- token source, redacted fingerprint, length, and whitespace flag;
- requested model;
- ACP-accepted model from `session/set_config_option`;
- provider-reported response model from the first real turn.

The Desktop emits `runtime_identity` and `agent_runtime_identity`; the broker
emits `buzz_acp::runtime_identity`, `model_selection_verified`, and
`response_model_verified`. Token values are never logged.

Any missing required binary, invalid code signature, explicit model rejection,
or model mismatch is a failed handoff. Fix or report it; do not accept the
adapter default silently.

The local build is ad-hoc signed, not Developer ID signed or notarized. A
release artifact still requires the repository's Apple signing workflow; a
strict local `codesign` pass must not be represented as certificate trust.

## Duplicate-bundle recovery

If Spotlight shows multiple Buzz apps, first inspect their paths. Preserve the
canonical `/Applications/Buzz.app`. Rename stale target-directory bundles so
their names no longer end in `.app`; do not delete them while diagnosing. Then
rebuild and install through the commands above.
