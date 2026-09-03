NIP-LR
======

Local Agent Session Residency and Continuity
---------------------------------------------

`draft` `optional` `local`

## Abstract

This document defines an opt-in local runtime contract for keeping agent
sessions resident, rotating them at predictable boundaries, and carrying a
bounded summary into a replacement session. It defines no Nostr event kind or
relay behavior. All state stays on the device that runs the agent.

## Motivation

Long-lived provider sessions can preserve reusable prefix state and avoid
repeated context reconstruction. They also need explicit resource limits and a
safe continuity path when a client intentionally tears them down. A single
local policy lets a client expose those controls without changing the signed
messaging protocol or requiring adapter-specific behavior.

## Activation and Compatibility

Implementations MUST gate this contract behind an explicit experimental
feature. Disabling the feature MUST restore the implementation's existing
residency behavior and MUST disable continuity capture and injection. Existing
agents and clients that do not implement this document remain compatible.

The reference environment variables are:

- `BUZZ_ACP_IDLE_POOL_SLEEP`: seconds before an idle pooled process sleeps;
  `0` disables idle sleeping.
- `BUZZ_ACP_MAX_TURNS_PER_SESSION`: successful turns before predictable
  session rotation; `0` disables count-based rotation.
- `BUZZ_ACP_SLEEP_WITH_SUMMARY`: enables local continuity capture.
- `BUZZ_ACP_SLEEP_SUMMARY_DIR`: an explicit pair-local state directory. An
  implementation MUST reject summary capture when no directory is supplied.
- `BUZZ_ACP_PREFIX_BUDGET_WARNINGS`: enables standing-prefix measurement and
  warnings.
- `BUZZ_ACP_PREFIX_CORE_WARNING_BYTES`: rendered core-memory warning threshold;
  the reference default is 12 KiB.
- `BUZZ_ACP_PREFIX_TOTAL_WARNING_BYTES`: rendered standing prompt plus
  configured MCP-descriptor warning threshold; the reference default is 200
  KiB.

Settings are sampled when the managed process starts. A running process MUST
NOT silently change residency policy because a profile is edited.

## Residency

An idle timeout applies to the pooled process, not to each provider session.
Activity MUST reset the idle deadline. An eager pool MUST remain resident and
therefore ignore the idle timeout.

The reference default is 900 seconds and no count-based rotation, preserving
the behavior that predates this experiment. Clients MAY expose larger values
per agent. A configured value greater than 900 seconds MUST keep the process
resident past 900 seconds when no other shutdown condition occurs.

## Predictable Rotation

A positive turn limit MUST count successful user turns within each scoped
provider session. Reaching the limit invalidates that scope only. Session
creation failures, cancelled turns, and private continuity prompts MUST NOT
increment the count.

Implementations MAY capture continuity immediately before these predictable
boundaries:

- configured idle sleep;
- configured turn-count rotation; and
- an explicit owner rotation.

Implementations MUST NOT attempt another provider turn after a hard timeout or
a provider-declared token/turn limit unless the provider explicitly declares
that operation safe. A previously stored summary may still be used after such
an unplanned teardown.

## Sleep-with-Summary

Before a predictable boundary, the runtime MAY issue one private prompt in the
existing provider session asking for a compact state handoff. That prompt MUST
instruct the model not to send messages, invoke tools, make external changes,
or include credentials. Prompt text is defense in depth: when tools remain
available in the provider session, the runtime cannot cryptographically
guarantee that the model will obey. Implementations SHOULD use a provider mode
that disables tools when one exists.

The capture MUST NOT be published as a conversation message or counted as the
preceding user's delivery evidence. Its usage MUST NOT replace the preceding
user turn's accounting.

The reference implementation applies these bounds:

- at most 4 KiB of UTF-8 summary text per scope;
- at most 64 entries and 512 KiB per local store;
- entries expire after 7 days;
- idle capture has a 15-second soft timeout and 20-second hard timeout; and
- one idle sweep considers at most four sessions and spends at most 30 seconds.

The runtime MUST remove control characters and SHOULD redact lines that look
like credentials before persistence. Such filtering is best effort and MUST
NOT be represented as complete secret detection.

The local directory and file SHOULD be restricted to the current user. On
Unix, the reference modes are `0700` for the directory and `0600` for the
file. Replacement MUST be atomic. The store MUST be isolated per managed
agent/runtime identity; it MUST NOT be shared across unrelated agents.

## Injection and Consumption

On replacement session creation, a valid entry for the exact conversation or
thread scope is appended to standing context after durable core, team, huddle,
and canvas context. It MUST be wrapped in an unambiguous semantic boundary and
state that it is a historical handoff rather than new user instruction.

The entry MUST remain available if session creation or the first replacement
turn fails. It MUST be removed only after the first successful user turn in
which it was supplied. Unknown, expired, malformed, or oversized local state
MUST fail closed by omitting the summary; it MUST NOT block ordinary message
delivery.

## Security and Privacy

Continuity state is local plaintext and can contain model-generated summaries
of private conversation state. It MUST NOT be published to a relay, included
in logs, or exposed to another agent identity. Implementations SHOULD offer a
clear deletion path and MUST treat local file permissions, bounds, escaping,
and scope identity as security boundaries.

## Prefix Budget Warnings

When enabled, the runtime SHOULD measure the UTF-8 byte contribution of each
standing section at every session creation. The reference breakdown includes
base, workspace, persona/system, team instructions, core memory, huddle
instructions, canvas, sleep summary, inter-section framing, and configured MCP
descriptors. It emits a warning when rendered core memory exceeds 12 KiB or the
rendered standing prompt plus MCP descriptors exceeds 200 KiB.

These limits are warnings only. An implementation MUST NOT truncate or omit
context, tools, memory, or reasoning merely because a budget is exceeded.
Thresholds MUST be configurable so operators can test policy and accommodate
different providers.

The MCP measurement covers only the descriptors the runtime serializes into
`session/new`. Provider-owned built-in tools and tool schemas discovered after
session creation are not observable at this boundary and MUST NOT be guessed.
Implementations MUST label the resulting total accordingly; it is not an exact
provider-token count.

Warning payloads MUST contain byte counts and thresholds only, never the
section contents. A local observer feed MAY surface the same structured warning
to a client UI without defining or changing a relay event kind.

## Relay Impact

None. No event, tag, subscription, permission, or relay storage rule changes.
