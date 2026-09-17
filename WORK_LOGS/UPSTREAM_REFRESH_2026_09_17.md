# Upstream refresh — September 17, 2026

Task: 01a0ab8d-e5c9-7ae1-92e6-407b564f226b. Nathan authorized the next local release steps.

## Pinned inputs

- Canonical base: `9bc3606498b4f98580079f0ebc2ade3a25315186`.
- Upstream: `4ab4f786085a23fe6126529861840eff6048ceee` (four missing commits).
- Existing installed source: `b55b45c30ad03e1fc903864e073724d7839eddb7`.
- Evidence: `/Users/titusclaw/.buzz/RESEARCH/upstream-refresh-20260917/`.

## Additions

- `891208914`: authenticated WebSocket recovery metrics and race-safe terminal authentication transitions.
- `6dfd145cd`: push gateway chart version 0.3.0.
- `b36600fc4`: mobile Settings shows the native build number; updates two authentication test constructors.
- `4ab4f7860`: mobile invite downloads resolve to the iOS or Android store; Fire OS retains the generic destination.

## Merge audit

One textual conflict, `crates/buzz-relay/src/connection.rs`: retain the fork's `public_scheme` while adopting upstream's synchronous authentication mutex. The synthetic conflict tree was `f47ec75bafc2e35b1e09d201c9e359fa0ee4634e`.

Additional compatibility edits outside the conflict: the fork-only Summary authorization fixture in `crates/buzz-relay/src/api/bridge/summary_tests.rs` now constructs the same standard mutex and removes its unused asynchronous RwLock import. Four new upstream test call sites now supply the fork-specific request public scheme to URL and connection helpers. The first Clippy run caught these stale upstream signatures. No other product hand edits. No migration changes.

## Verification and scope

Validation is pending at this commit and is recorded in the evidence directory. This source merge alone is not a passing release, installation, shared relay deployment, or mobile distribution. Existing browser gate issues retain their current owners.

## Editor gate classification and fixture correction

Fresh browser tests against the unchanged `9a06a910f` application reproduced the two inherited-label failures 6/6, at the named provider assertion. Source review identified a fixture/contract mismatch: the shared Tyler fixture selects Goose, while `getInheritedAgentDefaults` and the contributor contract scope structured global defaults to the preferred runtime (Buzz Agent when unset), and bundled provider/model defaults to Buzz Agent only.

The two existing assertions remain unchanged. Their fixtures now explicitly select `buzz-agent`. A third browser test asserts that a Goose instance does not inherit the Buzz Agent global or bundled provider/model. The initial corrected run passed 9/9 against the exact same frozen application assets. This corrects stale test setup; it does not restore cross-runtime model leakage or change product code. The full repeat and release batteries are recorded in the external evidence directory. Existing D-365a4001 and D-ac89d28b assignments are not changed or awakened.
