# Agent profile identity

## An explicit key is exact

Opening a public key from a message, member, DM, deep link, or Instances row
always opens that identity. Active, stopped, archived, and relay-only keys obey
the same rule. A local managed record may supply controls only for that exact
key. An owner-signed kind 30177 `persona_id` (or an archive request's historical
persona link) is **not** an identity alias and does not grant access to a local
sibling's controls, definition, configuration, or Start action.

Only explicit persona navigation (for example, the persona card in My Agents)
selects an archive-aware representative using `pickProfileAgent`. With no live
representative, it remains a persona-only surface and may offer Start. An
explicit relay-only key must not turn into that persona-only surface, even if a
matching definition exists locally. There is no synthetic/secretless
`ManagedAgent` record.

This intentionally supersedes the old historical-message redirect: a message
from stopped A no longer opens running B merely because they share persona P.
The requested key takes precedence even if the caller also supplies persona
context. Local instance profiles may still show their own linked definition and
an explicitly navigable Instances list. Ownership alone permits owner-scoped
relay reads, not local management.

Implementation: `useCanonicalManagedAgentProfile` and `UserProfilePanel`.
The obsolete historical-persona relay lookup and inactive-instance redirect
helper have been removed instead of adding another exception flag.

## Regression gates

- `profile/lib/resolveCanonicalManagedAgent.test.mjs`: exact identity across
  active/stopped/archived/relay-only keys, and persona representative selection.
- `profile/lib/useCanonicalManagedAgentProfile.test.mjs`: remote A cannot borrow
  persona P/local B, including persona-only navigation and returning to A.
- `tests/e2e/exact-key-profile.spec.ts`: relay-only profile controls versus
  explicit persona navigation, with and without a local sibling; exact archived
  keys with a live sibling and persona-only navigation when all are archived.
- `tests/e2e/profile.spec.ts`: historical messages match the exact Instances
  selection rather than the current persona card; profile ingress parity.
- `tests/e2e/identity-archive.spec.ts`: archive/unarchive authority and flair.

## Picture selection survives rollout

A saved picture changes only through an explicit picture edit. Changing a name,
model, runtime, description, or another setting carries the picture forward.
The instance editor changes that exact public key. “Edit shared definition”
changes the definition and its linked instances. The authorized CLI exposes the
same scopes through `agents set-avatar --scope instance|definition`; definition
is the compatibility default. Use `--url https://…` to select and `--clear` to
remove. Removal is durable, not a request to rediscover a runtime icon.

The store carries `avatar_selection` independently of whole-record timestamps:
`revision`, `edit_id`, and nullable `url`. Direct edits advance the revision;
Keep and background saves do not. Existing saved URLs bootstrap at revision zero
without classifying stock artwork as replaceable. Conflicting legacy saved and
published pictures require a direct edit to resolve. Empty legacy `avatar_url`
represents explicit removal; null means no selection has been established.

Whole-store writes merge selections before one atomic persist. Owner-signed
30175/30177 projections carry the same edit so a later settings snapshot cannot
replace a newer picture. Selection order is `(revision, edit_id)`; definition
edits advance past linked selections and explicitly fan out. Retained events
remain bound to their existing owner/community authorization boundary.

Kind-0 publication is serialized per public key and relay. It re-reads saved
intent after asynchronous reads, preserves current profile fields for avatar-only
updates, advances the profile timestamp, and carries a `buzz_avatar_edit` marker
with the revision and edit ID. An older device cannot supersede a newer marked
publication while it waits for the corresponding owner edit. This marker is an
ordering hint in an agent-signed profile, not an independent authorization grant.

Relay acknowledgments are stored separately by relay and exact edit ID. A failed
or interrupted send leaves the choice pending across restart; the fleet host
retries every thirty seconds. The instance editor exposes pending publication.
A failed image fetch never creates a replacement choice. Existing clients that
only send a full-form `avatarUrl` cannot change a picture; clients must send
`avatarEdit: {action: "keep"|"set"|"remove"}` (Set also requires `url`).
