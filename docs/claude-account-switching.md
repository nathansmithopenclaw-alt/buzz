# Claude account switching

## Product contract

Claude setup uses named long-lived tokens. OpenAI subscription login remains available. Account controls use Buzz's shared Button, DropdownMenu, dialog, spacing, typography and theme primitives. Browser-native account selects are not an accepted replacement for Buzz's menus.

The approved lifecycle design is one default account plus explicit agent overrides. When an agent's account changes, offer that agent or all agents using the old account on this Mac, across communities. Include stopped agents in the binding update without starting them. If replacing the default account for the whole group, move that default too. Returning to the old plan is always explicit; usage tracking is separate.

The intended operation pauses affected work, changes the account, and continues unfinished work on the destination account. A persistent operation record must preserve the affected agents, community runtimes, prior running/stopped intent, original requests, provider session references and per-agent outcome. Restarting Buzz during a switch must recover that operation. A failed destination leaves work paused with Retry and Choose another account; it must not silently select a different payer.

Removing an account must offer moving its agents to a replacement or stopping/detaching them. Credentials are removed only after every consumer is detached. The user should never have to hunt through communities to stop consumers manually.

## Verification standard

For Smith Crafts user-facing work, "verified" means exercising the installed product as a person would and answering:

1. Does the workflow work without errors?
2. Does the behavior and wording make sense to a person?
3. Is the workflow efficient for the person doing the work?

Record the installed build, entry point, actions, actual result and evidence. Source review, unit tests, mocked browser tests, successful compilation, runtime receipts and provider acceptance are separate evidence categories. None alone establishes installed human acceptance. Record failures and untested cases explicitly.

Account acceptance must cover Settings, an agent profile, the in-conversation editor and DM account control; mouse and keyboard; long account names; token expiry; invalid destination; individual and all-agent scope; multiple communities; stopped agents; active work; repeated switch requests; removal; and application restart during a switch. Retained work must be continued without replaying confirmed side effects. Unknown outcomes of external operations require reconciliation before repetition.

## Current implementation boundary

The initial migration slice replaces the shared raw select with Buzz's radio menu and retires Claude login creation/reconnect. Existing login accounts remain readable and removable; existing consumers keep their guarded startup path until explicitly migrated. Token creation and renewal remain available. The account-store secret containment and OpenAI login behavior are preserved.

The default/override model, fleet-scope dialog, durable pause/switch/resume transaction and integrated move-and-remove workflow are subsequent lifecycle work. The initial slice does not implement them and must not be described as the completed redesign.

An isolated real Claude adapter probe demonstrated process restart and session loading using the same token. A second probe canceled an active shell operation after its first file write, then loaded the session and completed the remaining write once. That is adapter-level evidence; it does not establish cross-account continuation, full Buzz integration, or exactly-once behavior for arbitrary external tools.
