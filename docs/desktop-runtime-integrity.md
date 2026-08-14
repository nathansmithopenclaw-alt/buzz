# Desktop runtime integrity

Buzz Desktop, its bundled `buzz-acp` broker, and each agent adapter form one
runtime. A passing source build does not prove that the running app uses those
bytes.

## Build and install

1. Work from an isolated worktree at the intended commit.
2. Run `just ci`.
3. Run `just desktop-release-build`. This compiles every bundled sidecar from
   the same checkout, creates only the macOS app bundle, signs the bundle as a
   complete unit, and runs `scripts/verify-desktop-bundle.sh`. The default is
   ad-hoc signing. A Developer ID rollout must set all three fail-closed inputs:

   ```sh
   BUZZ_MACOS_SIGNING_IDENTITY='<certificate SHA-1 or exact common name>' \
   BUZZ_MACOS_SIGNING_KEYCHAIN='/absolute/path/to/login.keychain-db' \
   BUZZ_MACOS_EXPECTED_TEAM_ID='<10-character Team ID>' \
   just desktop-release-build
   ```

   The Developer ID path applies the hardened runtime and required
   entitlements, requests an Apple timestamp, and verifies the expected
   authority, Team ID, embedded certificate chain, and timestamp on the main
   executable and every bundled sidecar. It repeats strict deep verification
   after a bounded delay before returning success.
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

The default local build is ad-hoc signed. Selecting Developer ID proves the
certificate-backed signature and stable Team ID, but it does not notarize the
artifact. Software distributed outside the Mac App Store must still be
submitted to Apple's notary service, stapled, and accepted by `spctl`; a strict
local `codesign` pass must not be represented as notarization.

## Duplicate-bundle recovery

If Spotlight shows multiple Buzz apps, first inspect their paths. Preserve the
canonical `/Applications/Buzz.app`. Rename stale target-directory bundles so
their names no longer end in `.app`; do not delete them while diagnosing. Then
rebuild and install through the commands above.
