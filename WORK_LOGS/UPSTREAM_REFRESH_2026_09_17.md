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
