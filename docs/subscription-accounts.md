# Subscription Accounts

Subscription Accounts is a local experiment under Settings → AI. Enable it in
Settings → Experiments, choose a supported harness, finish that harness's browser
login, then give the verified login a nickname. The Edit Agent selector and the
DM header read the same account store. Selection applies to the agent in every
conversation and community on this machine.

An explicit selection remains authoritative when the experiment is disabled.
Unavailable accounts stay visible. Buzz refuses the launch instead of choosing a
different account. Only selecting **Ambient** restores the harness's existing
login. Existing legacy Claude bindings stay in effect until explicitly replaced;
migration requires a fresh provider login and rebind before deleting the old entry.

## Provider policy

| Harness | Current implementation | Remaining acceptance |
| --- | --- | --- |
| Claude Code | macOS credential-only Keychain namespace; configuration remains in its existing context | Fresh browser login, status, selected managed turn, expiry and logout |
| Codex | Isolated home; explicitly selected file credential backend; non-secret configuration composed from the existing context | Fresh browser login and managed turn; unchanged ambient control; provider refresh/logout behavior |
| Grok | Displayed as unavailable | Establish subscription-specific status and safe configuration composition |

Buzz neither reads nor copies provider credentials. The harness owns login,
refresh, status and logout. Codex can support other credential backends; this
adapter deliberately selects file storage for its owned profile and does not
claim keyring/auto isolation. Configuration containing competing authentication
or unsupported path semantics is rejected with an actionable error. Known
non-secret configuration paths are preserved or resolved against their original
home. A different account never implicitly selects different agent settings.

## Runtime guarantees

- Account metadata and bindings are local. Portable definitions and team
  snapshots exclude subscription identifiers and credential homes.
- Login attempts are bounded, cancelable, and generation checked. A watchdog
  bounds orphaned login processes after a Desktop crash. Reconnect preserves
  account identity and nickname.
- A switch first obtains an idle admission barrier from every affected broker.
  Active turns and tracked background work block switching. Idle brokers stop
  before the binding is saved; replacement failures stay visible. Running and
  selected bindings are reported separately. Provider sessions are not resumed
  across account changes.
- One live adapter consumer per profile is enforced until concurrent refresh is
  proven. Live PID leases and durable runtime receipts retain ownership after
  Desktop failure. A crash between reservation and PID recording leaves an
  explicit blocked reservation requiring operator verification of orphan cleanup;
  a missing receipt alone never authorizes reuse. Remote deployment refuses local
  subscription bindings.
- A managed authentication failure durably holds the original signed requests
  and pauses new dispatch. Reconnect does not execute them automatically. Use
  **Resume retained work** in the agent's Account controls after reconnecting and
  starting the agent. Earlier partial work may already have run. Resume rechecks
  current channel membership; stored requests remain held if access is missing.
  Restart re-arms the hold, and completed event receipts suppress replay.
- Removal checks saved and live consumers, runs logout in the owned profile,
  records logout completion, and then deletes the owned directory. Failed
  cleanup remains a retryable removal record. Legacy credentials are untouched.

## Validation boundaries

Automated tests use synthetic credentials/configuration and fake provider
executables. Browser tests use the application's mock IPC bridge. These establish
source behavior, not real provider authentication, billing, installed Desktop
acceptance, or concurrent refresh safety.

For each enabled platform/version, acceptance requires an owner-operated fresh
login through Desktop, nickname, selection, and a real managed turn. Check the
ambient control, authentication override refusal, idle/busy switching, feature
off, expiry/reconnect/resume, cancel/crash cleanup and removal. Use a fresh Codex
profile; never test against or copy the shared Codex authentication home.
