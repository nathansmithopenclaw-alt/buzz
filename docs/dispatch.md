# Working with Dispatch

Dispatch keeps the work, its conversation and its evidence together in Buzz.
An issue has a human owner; an execution attempt can have a different executor
and an independent reviewer. These are separate responsibilities.

## Find the next useful action

Triage is for deciding whether incoming work is ready, who owns it and its priority.
Todo is accepted work. Board and List support grouping, filtering and bulk changes;
closed Board work stays collapsed until expanded. My Issues focuses on your ownership.
Attention views surface decisions, ready work, blockers, pending review, delegated
work and recent delivery. Save a personal view on this device or a shared workspace
view for the team. Routing suggestions explain capacity, capabilities and dependencies.
Suggestions do not assign work by themselves.

## Open Dispatch quickly

After launch, Buzz prepares your last Dispatch workspace in the background.
Previously synchronized issues can appear while current history is still loading.
The saved view is labeled, and editing becomes available after synchronization.
Opening Dispatch again reuses the current view while checking for updates.

The saved view requires a current workspace membership check. A first visit, a
changed community or identity, or an unavailable relay can still require loading.
Use Refresh Dispatch to check for new workspaces and current changes explicitly.

## Make a clear issue

Edit the title and description directly, or use Edit issue for owner, project,
labels, due date, estimate, conversation, relationships, links and attachments.
The readable issue alias is a shortcut; the underlying issue identity remains stable.
Use the issue's section shortcuts to reach details, execution, decisions, delivery
and activity without scrolling through every panel.

A conversation message can create an issue or attach to existing work. Capture
keeps a source link and lets you review the description. Repeating a capture opens
the same source-derived issue. Imported work keeps its provenance and needs explicit
activation before unattended processing.

Choose **Remind me** on an issue to select a due-time preset or a custom time.
The reminder appears in Reminders; opening it or its due notification returns to
the issue. Use **Remind later**, mark it done, or cancel it there, just like any
other reminder.

## Execute and review

Choose **Assign** on an issue. Select an **Executor**, confirm the **Independent reviewer**,
and add **Notes**. Executors with capacity records appear first. The reviewer defaults
to the issue owner when they are not the executor; an executor cannot review their own work.
The assignment always goes to the issue’s home channel. Open **Advanced** for a routing
override or an optional deadline, which is included in the assignment notes. The owner
stays accountable. The assignment links to the
issue before the executor is notified. Blockers, deliverables and task history stay
visible in Execution and evidence.

“Reported complete” is an executor claim. Review the artifact against the agreed
method, add evidence and observations, then accept or reject that result. The executor
cannot independently review their own result. A failed attempt remains history; a
new attempt receives a new identity. Closing the issue is a separate decision.

Use decisions to record a question, options, consequences and evidence for a named
person. Only that decision maker can choose or revise the answer. Delivery observations
record distinct facts such as tests, review, merge, installation and acceptance.
Adding an observation does not silently promote the issue or task to another stage.

## Close the conversation loop

Share an update previews the current issue status, blocker, completion or decision
and lets you edit the message before sending. It checks the destination can read the
included source context. An optional decision notification targets the named decider.
Sharing the same change again reuses the accepted update. Conversation work links read
the current issue state; they do not derive status from prose.

## Recover a saved change

If confirmation is lost, Check and retry first checks accepted history and resends
only the same saved event if appropriate. When the issue has moved on, keep the old
change as a draft. Saved drafts support readable copying, export and opening the current
issue. Keeping a draft does not recall a write already sent. Review current history
before applying its content again. Storage, membership or community errors preserve
recovery instead of pretending the change was successful.

## Projects, automation and mobile

Project home connects ownership, purpose, outcome, milestones and relevant conversation
or document links. Context export includes bounded current scope, provenance and
accepted history. Duplicate suggestions require your decision. Metrics separate
observed durations from imported or missing history; they are not personnel scores.

Automation starts with preview and a configured service. Review its current signed
health, exact scope, destinations, time limit and total allowance before activating.
Pause does not undo prior work. A configuration-only legacy service must be upgraded
before the new UI can control it; see the rollout guide.

The mobile Work tab offers open/personal queues, triage and named decisions using
the existing Buzz identity. Its relay must support the Dispatch read/preparation
command. Offline and uncertain sends retain exact changes and saved drafts for recovery.

### Desktop navigation and operator service controls

Dispatch has five views: Board, List, Triage, My Issues, and Projects. Retired
`view=insights` and `view=automation` links open Board. The Triage badge counts
unassigned issues in Triage and shows the oldest time in that status, in whole
days; it disappears when no unassigned Triage issues remain.

The fork-only external dispatcher controls live in Settings > Dispatch service,
behind the Dispatch experiment. The entry appears only for a maintainer of the
selected Dispatch workspace with an existing active automation service record.
Select the workspace in Dispatch first when more than one workspace is available.
The existing service connection, heartbeat, action allowance, expiry, preview,
and pause behavior is unchanged. Insights has no replacement Flow block.

## Capture shortcuts

With the Dispatch experiment enabled, hover a delivered message and choose
**Track in Dispatch**, or focus its row and press **T**. The More menu keeps the
same action. **Cmd+Shift+T** opens capture for your newest loaded, delivered
message in the open channel. Shortcuts leave text entry, composition, dialogs,
and menus in control of their own keys.

Start a new channel or thread message with `/track ` to send the message and
open capture with its first line as the title. The command is removed from the
sent text. Capture opens only after the send succeeds; review and create the
linked issue in the dialog. Editing an existing message does not execute the
command. On mobile, long-press a delivered message and choose **Track in
Dispatch** to review its title and create a Triage issue in that channel. Work
shows the issue; uncertain sends retain the exact signed change for recovery.

Channel editors can enable **Pin reactions create Dispatch issues**. This saves
a channel-scoped `dispatch-capture` workflow with a fixed 📌 reaction trigger
and `dispatch.create_issue` action. Turning it off sets `enabled:false` and
preserves its history. The relay must support that action; saving an unsupported
workflow reports the error without claiming it was enabled.

## Automation in the relay

Pin capture, unowned-work alerts and project Autopilot run in Buzz's relay
workflow engine. An administrator enables the workflow and its signed scope
record. The capture record names the receiving Dispatch workspace; the workflow
alone cannot infer it. Missing or revoked authorization appears in automation
history as a skipped action.

Autopilot selects Todo work with no blockers, optionally filtered by a label.
Defaults are two unfinished attempts per agent and ten project assignments per
UTC day. The issue owner independently reviews the result. Three assignments in
the first hour pause further assignment for confirmation; pausing never undoes
existing work. When the executor picks up a linked task, its issue moves to In
Progress automatically; reported completion moves it to In Review.

See [NIP-BT](nips/NIP-BT.md#built-in-relay-automation-authorizations-kind-40026)
for the signed records and [NIP-WF](nips/NIP-WF.md#dispatch-actions) for workflow
definitions. The external [dispatcher](dispatcher.md) remains an optional
operator mode with its existing controls; it is not required for these relay
actions. Desktop controls that publish the new scope records are a separate
integration from the relay protocol.

If an accepted pickup or completion does not update the board because projection
failed, its task history remains saved. The next Dispatch action retries the
update, including the next pin in a capture-only workspace. Periodic recovery
requires a scheduled Dispatch workflow; without one, a quiet workspace has no
background retry. Administrators can keep an assignment schedule running with
Autopilot paused to retry updates without starting new work. Missing legacy
admission evidence appears as a skipped unowned alert and needs an administrator
baseline; it does not block reminders for other issues.
