NIP-AH
======

Agent Hosts and Liveness
------------------------

`draft` `optional`

This NIP defines two public, owner-authored addressable events: a durable host
declaration and the latest lifecycle-only status for that host. Together they
let a client render where an owner's public managed-agent projections run and
whether the last host report is still fresh.

The relay remains a signed-event store. Roster joins, freshness, and foreign
owner enumeration are client policy; the relay does not synthesize fleet rows.

## Kinds and coordinates

This NIP claims two NIP-33 parameterized replaceable kinds:

| Kind | Name | Coordinate |
|------|------|------------|
| `30180` | Agent host declaration | `(owner pubkey, 30180, host_id)` |
| `30181` | Agent host status | `(owner pubkey, 30181, host_id)` |

`host_id` is a UUID generated once for one host installation and persisted
device-locally. Its wire form MUST be the canonical lowercase hyphenated UUID
string. A host name is presentation metadata and MUST NOT be used as identity.

Both kinds are durable addressable events. In particular, kind `30181` is NOT
ephemeral: a restarted observer must be able to read the stored head and render
honest last-seen state.

## Roles

- **owner** — the Nostr identity that authors host and managed-agent
  projections;
- **host** — one installation that runs zero or more agents for the owner;
- **observer** — a client that reads the projections and may communicate with
  their agents but does not acquire authority to run or control them.

Publishing a declaration or status proves only that the owner signed that
projection. Host execution authority, key custody, and lifecycle control are
outside this NIP.

## Host declaration: kind 30180

### Envelope

```jsonc
{
  "kind": 30180,
  "pubkey": "<owner-pubkey>",
  "created_at": 1786910000,
  "tags": [
    ["d", "018f42c2-9bc2-7dc1-a41e-2f77769f0142"]
  ],
  "content": "<json-body>"
}
```

There MUST be exactly one `d` tag and it MUST contain the canonical host UUID.
An `alt` tag MAY be included for unknown-kind viewers. Other tags have no
semantic effect. In particular, an `h` tag never makes this event
channel-scoped.

### Content

The plaintext content is an exact JSON object:

```json
{
  "v": 1,
  "name": "Build host",
  "platform": "macos",
  "created_at": 1786910000
}
```

| Field | Contract |
|-------|----------|
| `v` | integer `1` |
| `name` | non-empty, already trimmed, at most 128 Unicode scalar values, no control characters |
| `platform` | exactly `macos`, `linux`, or `windows` |
| `created_at` | positive Unix seconds when the host identity was first created |

The serialized content MUST NOT exceed 4,096 UTF-8 bytes. Unknown fields are
invalid. `created_at` MAY remain unchanged when the host is renamed; it MUST
not be more than 900 seconds ahead of relay time when submitted.

## Host status: kind 30181

### Envelope

The envelope has the same author and `d` coordinate as the declaration:

```jsonc
{
  "kind": 30181,
  "pubkey": "<owner-pubkey>",
  "created_at": 1786910030,
  "tags": [
    ["d", "018f42c2-9bc2-7dc1-a41e-2f77769f0142"]
  ],
  "content": "<json-body>"
}
```

There MUST be exactly one canonical host UUID `d` tag. An `h` tag has no
routing effect; the event is stored globally.

### Content

```json
{
  "v": 1,
  "seq": 42,
  "ts": 1786910030,
  "agents": [
    {
      "pubkey": "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
      "state": "running",
      "since": 1786909900
    }
  ]
}
```

| Field | Contract |
|-------|----------|
| `v` | integer `1` |
| `seq` | positive `u64`, monotonically increasing for this host |
| `ts` | positive Unix seconds when the host sampled the status |
| `agents` | zero to 256 lifecycle entries |
| `agents[].pubkey` | unique, curve-valid, 64-character lowercase hex Nostr public key |
| `agents[].state` | exactly `running`, `stopped`, or `error` |
| `agents[].since` | positive Unix seconds no later than `ts` |

The serialized content MUST NOT exceed 65,536 UTF-8 bytes. The top-level
object and every agent object use exact-key allowlists; unknown fields are
invalid. `ts` MUST be within 900 seconds of the signed event's `created_at`.

`state` describes local process lifecycle only. A valid status MUST NOT carry
task content, channel names or identifiers, current work, prompts, filesystem
paths, environment values, credentials, auth material, backend configuration,
or secret-bearing error detail. Clients that need activity or task state use
the corresponding protocols rather than extending this public object.

Publishers persist `seq` with the host identity and increment it on each
publish. A relay can reject zero or malformed sequence numbers, but NIP-33 head
selection is based on event `created_at`, not `seq`. Writers therefore MUST
also choose an event `created_at` that supersedes the current head. Readers MAY
use a non-increasing sequence as a diagnostic signal but MUST select the head
using NIP-33 rules.

## Publication cadence and freshness

A host SHOULD publish kind `30181` on every reported agent lifecycle
transition and at least every 30 seconds while its host application is
running. It SHOULD publish kind `30180` when the host identity is first created
and whenever its presentation name changes.

Freshness is a client-side contract over the winning kind `30181` head:

1. Compute `age = max(0, now - ts)`.
2. If `age <= 90 seconds`, the status is **fresh** and its lifecycle states may
   be rendered.
3. Otherwise it is **stale**. Render the host as unreachable since `ts` and
   treat every contained agent state as unknown/stale, never as currently
   running or stopped.

Relays do not delete or rewrite a status when it becomes stale. Keeping the
last head is what makes honest last-seen survive observer restart and relay
reconnect.

## Relay behavior and access

Relays implementing this NIP:

- MUST require authenticated `UsersWrite` authority matching the event author;
- MUST store both kinds globally even if the signed event contains an `h` tag;
- MUST apply NIP-33 replacement by `(pubkey, kind, d)`;
- MUST enforce the coordinate, content, count, size, and time constraints
  above before a malformed event can replace a valid head;
- MUST keep both kinds community-readable, matching the read class of public
  managed-agent kind `30177`; they are neither author-only nor `shared`-gated;
- MUST NOT treat kind `30181` as ephemeral.

The relay's ordinary community authentication and tenancy boundary still
apply. "Community-readable" does not authorize cross-community disclosure.

## Reading and fleet projection

For one owner, a client reads all host declarations and statuses with:

```json
{"kinds":[30180,30181],"authors":["<owner-pubkey>"]}
```

It groups heads by `d`, validates content again, and joins agent entries to the
owner's kind `30177` managed-agent heads. Host `name` comes only from kind
`30180`; agent identity and display metadata remain owned by the existing
agent/persona protocols. A status entry does not create a runnable agent.

Clients SHOULD enumerate another owner's fleet only when that owner shares at
least one channel with the viewer. This membership-scoped recommendation
prevents a client from turning community-readable projection kinds into an
unprompted relay-wide people-and-host directory. Direct reads for an already
known owner MAY still be used where product policy permits. All UI surfaces
that enumerate or mention foreign agents SHOULD apply the same policy.

## Deletion

An owner retires a host declaration with a NIP-09 kind `5` deletion request
whose `a` tag is:

```text
30180:<owner-pubkey>:<host-id>
```

The request SHOULD also contain `["k", "30180"]`. The owner MAY similarly
delete the status coordinate with kind `30181`. Relays honor these requests
under their normal NIP-09 policy. A later valid event may recreate the
coordinate under ordinary NIP-33 semantics.

## Security and privacy

These events are plaintext public projections inside their community. The
exact-key schemas are a privacy boundary, not merely a parsing convenience.
Host names can still reveal user-chosen information, so clients SHOULD default
to an ordinary machine name and SHOULD warn before publishing unusually
sensitive text.

Liveness is owner-asserted, not remotely attested. A fresh signature says what
the owner reported; it does not prove that a process is healthy, responding,
or exclusively hosted on one device. Staleness MUST override optimistic agent
state so a stopped publisher cannot leave false green status indefinitely.

## Non-goals

Version 1 does not define:

- a server-side fleet or roster projection;
- remote start, stop, create, configure, or deployment commands;
- transfer of agent keys, environment values, or runnable configuration;
- host leases, exclusive ownership, handoff, fencing, or anti-resurrection;
- task, channel, prompt, transcript, or activity projection;
- teammate mutation of another owner's fleet.

Host handoff and lease semantics require a separate version with explicit CAS,
revocation, and migration verification. NIP-PMA kind `30179` is related private
storage work, but its generic stored head does not grant those semantics.
