# Preserve the installed fork during the upstream upgrade

The September 16 upstream source merge is not sufficient on its own for a
Desktop installation. The canonical installed app was built from
`94c05e466935296e7e5fc69edc20385d754fc26b`, which contains 62 commits absent from
the earlier local main. This integration joins that installed history with
`ea4a582c5087600f88b78b95ec65d8f431f56bd0`, the validated merge of upstream
through `213092074c269bee90c87163672f9c2b8d714cfe`.

The installed source was identified by matching the desktop, CLI, ACP, and
developer-MCP SHA-256 values against the September 11 installation receipt.
The common ancestor of the two integration parents is
`960cd5064e4d7a9fb4ffd2a7cca303b986a4c5ae`.

## Retained installed features

- Summary scheduling, durable runs, observed-read authority, report archives,
  read coverage, and terminal delivery.
- Dispatch editing, assignments and reviews, cache and membership behavior,
  read-only relay responses, and CLI support.
- DeepSeek endpoint configuration, saved model connections, and Prime support.
- Scoped agent-edit delegation and externally owned Codex Desktop execution.
- Computer-use evidence and refusal handling, plus installed regression fixes.

## Compatibility resolutions

The initial merge had nine conflicted paths. ACP keeps both additional test
modules. HTTP quota isolation and the installed read-only Dispatch regression
both remain. The shared setup renderer places the installed endpoint controls
before upstream's credential validation and model controls, retaining one API-key
control. The new delegated-edit button uses upstream's renamed npub helper.
Database concurrency fixtures preserve the already validated fork behavior.
The iOS Podfile checksum is recomputed from the combined Podfile; this is not an
iOS build or distribution receipt.

The installed fork already owns migrations 0051 through 0053 for Summary.
Consequently the previously uninstalled upstream tombstone migration is now
`0054_retain_push_revocation_tombstones.sql`. All installed migrations 0001
through 0053 remain byte-for-byte unchanged. The upgrade regression starts at
0053 and checks historical checksums, revoked-row retention, live uniqueness,
and idempotence. This supersedes the prospective 0051 numbering in the earlier
`UPSTREAM_INTEGRATION_2026_09_16.md`; no live relay database is migrated here.

## Installation boundary and evidence

The upgrade is local to `/Applications/Buzz.app`. The signed installer preserves
the previous application in `/Applications/.buzz-backups/` and verifies signing
identity continuity. Stored identities, policies, model connections, and the
external-execution binding must survive the replacement. At preflight the old
global thread-session experiment was off and every stored record omitted
`session_policy`, so the new channel default preserves existing behavior.

The agent store contains 45 instances and 45 definitions. The external Codex
Desktop identity is stopped with autostart disabled. Its listener already reports
`app-update-requires-revalidation` for Codex Desktop before this Buzz upgrade;
that pre-existing compatibility hold must remain until separately validated.

Evidence for combined-source validation and subsequent installation is retained
in `/Users/titusclaw/.buzz/RESEARCH/upstream-install-20260916/`. Source tests,
signed-bundle checks, installed UI checks, and actual provider execution are
separate proof tiers. No GitHub push, shared relay/push-gateway deployment,
mobile distribution, or new live agent assignment is part of this installation.
