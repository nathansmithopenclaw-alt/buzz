#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const IMPORTER_VERSION = "buzz-linear-tracker-v1";
const LINEAR_NAMESPACE = "7a133f7e-3df5-5ce7-8586-65f471ec7e8a";

function fail(message, details = {}) {
  process.stderr.write(`${JSON.stringify({ error: "migration_error", message, ...details })}\n`);
  process.exit(1);
}

function parseArgs(argv) {
  const mode = argv.shift();
  if (!["plan", "apply", "verify"].includes(mode)) {
    fail("first argument must be plan, apply, or verify");
  }
  const flags = { mode };
  while (argv.length) {
    const flag = argv.shift();
    if (!flag.startsWith("--")) fail(`unexpected argument ${flag}`);
    const key = flag.slice(2).replaceAll("-", "_");
    if (key === "resume") {
      flags.resume = true;
      continue;
    }
    const value = argv.shift();
    if (value === undefined || value.startsWith("--")) fail(`${flag} requires a value`);
    flags[key] = value;
  }
  return flags;
}

function required(flags, key) {
  const value = flags[key];
  if (!value) fail(`--${key.replaceAll("_", "-")} is required`);
  return value;
}

function loadJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`cannot parse ${path}`, { detail: String(error) });
  }
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
  }
  return value;
}

function canonicalJson(value) {
  return JSON.stringify(canonical(value));
}

function uuidBytes(value) {
  const hex = value.replaceAll("-", "");
  if (!/^[0-9a-f]{32}$/i.test(hex)) fail(`invalid UUID ${value}`);
  return Buffer.from(hex, "hex");
}

function uuidV5(namespace, name) {
  const digest = createHash("sha1")
    .update(Buffer.concat([uuidBytes(namespace), Buffer.from(name)]))
    .digest()
    .subarray(0, 16);
  digest[6] = (digest[6] & 0x0f) | 0x50;
  digest[8] = (digest[8] & 0x3f) | 0x80;
  const hex = digest.toString("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

function nodes(value) {
  if (Array.isArray(value)) return value;
  if (Array.isArray(value?.nodes)) return value.nodes;
  if (Array.isArray(value?.edges)) return value.edges.map((edge) => edge?.node).filter(Boolean);
  if (Array.isArray(value?.data)) return value.data;
  return [];
}

function issueLabels(issue) {
  return [...new Set(nodes(issue.labels).map((label) => (label?.name ?? label).toString().trim().toLowerCase()))];
}

function normalizeExport(raw) {
  const root = raw.data?.workspace ?? raw.workspace ?? raw.data ?? raw;
  const workspaceId = root.id ?? raw.workspace_id ?? raw.metadata?.workspace_id;
  if (!workspaceId) fail("export does not identify its Linear workspace");
  const complete =
    raw.complete_export === true ||
    raw.complete === true ||
    raw.metadata?.pagination_complete === true ||
    raw.metadata?.complete_export === true;
  if (!complete) {
    fail("export completeness proof is absent; set complete_export=true only after every page was extracted");
  }
  const issues = nodes(root.issues ?? raw.issues);
  const projects = nodes(root.projects ?? raw.projects);
  const comments = nodes(raw.comments).length
    ? nodes(raw.comments)
    : issues.flatMap((issue) => nodes(issue.comments).map((comment) => ({ ...comment, issue_id: issue.id })));
  if (!issues.length) fail("complete export contains no issues");
  return { workspaceId, issues, projects, comments, raw };
}

function stateKey(issue) {
  return issue.state?.id ?? issue.state?.name ?? issue.state_id ?? issue.status;
}

function userKey(user) {
  return user?.id ?? user?.email ?? user?.name ?? null;
}

function projectKey(project) {
  return project?.id ?? project?.name ?? null;
}

function normalizePriority(value) {
  if (["P0", "P1", "P2", "P3"].includes(value)) return value;
  const key = typeof value === "string" ? value.toLowerCase() : value;
  return {
    1: "P0",
    2: "P1",
    3: "P2",
    4: "P3",
    urgent: "P0",
    high: "P1",
    medium: "P2",
    low: "P3",
  }[key] ?? null;
}

function lookup(map, candidates) {
  for (const key of candidates.filter(Boolean)) {
    if (Object.hasOwn(map, key)) return map[key];
  }
  return undefined;
}

function issueIdentifier(issue) {
  return issue.identifier ?? `${issue.team?.key ?? "LIN"}-${issue.number ?? issue.id}`;
}

function sourceUrl(issue) {
  return issue.url ?? issue.source_url ?? null;
}

// Tracker relations are typed and directed (M12 removed undirected
// "related"), so only source types with a canonical mapping can migrate.
// Everything else is dropped explicitly and surfaced in the plan report.
const RELATION_TYPE_MAP = {
  blocks: "blocks",
  blockedBy: "blocked_by",
  duplicate: "duplicate_of",
  duplicateOf: "duplicate_of",
  parent: "parent_of",
  child: "child_of",
};

function allRelationRecords(issues) {
  return issues.flatMap((issue) =>
    nodes(issue.relations).map((relation) => ({
      source: issue.id,
      target: relation.target?.id ?? relation.relatedIssue?.id ?? relation.issue_id ?? relation.target_id,
      type: relation.type ?? relation.relation_type,
    })),
  );
}

function relationRecords(issues) {
  return allRelationRecords(issues).filter((relation) => RELATION_TYPE_MAP[relation.type] !== undefined);
}

function droppedRelationRecords(issues) {
  return allRelationRecords(issues).filter((relation) => RELATION_TYPE_MAP[relation.type] === undefined);
}

function linkRecords(issues) {
  return issues.flatMap((issue) =>
    nodes(issue.links ?? issue.attachments).map((link) => ({
      issue: issue.id,
      type: link.type ?? "url",
      ref: link.ref ?? link.url ?? link.id,
      title: link.title ?? link.label ?? null,
    })),
  );
}

function increment(counts, key) {
  const normalized = key ?? "none";
  counts[normalized] = (counts[normalized] ?? 0) + 1;
}

function sourceSnapshotEpoch(source) {
  const explicit = source.raw.exported_at ?? source.raw.exportedAt ?? source.raw.metadata?.exported_at;
  const candidates = [
    explicit,
    ...source.issues.flatMap((issue) => [issue.updatedAt, issue.updated_at, issue.createdAt, issue.created_at]),
    ...source.comments.flatMap((comment) => [comment.updatedAt, comment.updated_at, comment.createdAt, comment.created_at]),
    ...source.projects.flatMap((project) => [project.updatedAt, project.updated_at, project.createdAt, project.created_at]),
  ].filter(Boolean).map((value) => Date.parse(value)).filter(Number.isFinite);
  if (!candidates.length) fail("export has no stable timestamp for deterministic import provenance");
  return Math.floor(Math.max(...candidates) / 1000);
}

function setDifference(expected, actual) {
  return [...expected].filter((value) => !actual.has(value)).sort();
}

function relationKey(source, type, target) {
  const normalized = {
    blockedBy: "blocked_by",
    duplicate: "duplicate_of",
    duplicateOf: "duplicate_of",
    parent_of: "parent",
    child_of: "child",
  }[type] ?? type;
  if (normalized === "blocked_by") return `${target}:blocks:${source}`;
  if (normalized === "child") return `${target}:parent:${source}`;
  return `${source}:${normalized}:${target}`;
}

function linkKey(issue, type, reference) {
  return `${issue}:${type}:${reference}`;
}

function validatePilotMap(pilotMap, issueIds) {
  const seenLinear = new Set();
  const seenEvents = new Set();
  const conflicts = [];
  for (const [linearId, pilot] of Object.entries(pilotMap)) {
    if (!issueIds.has(linearId)) conflicts.push(`pilot Linear issue ${linearId} is absent from export`);
    if (!/^[0-9a-f]{64}$/.test(pilot.event_id ?? "")) conflicts.push(`pilot ${linearId} lacks a full event ID`);
    if (seenLinear.has(linearId) || seenEvents.has(pilot.event_id)) conflicts.push(`duplicate pilot mapping for ${linearId}`);
    seenLinear.add(linearId);
    seenEvents.add(pilot.event_id);
    const labels = pilot.labels ?? [];
    const values = (prefix) => labels.filter((label) => label.startsWith(`${prefix}:`)).map((label) => label.slice(prefix.length + 1));
    const linearLabels = values("linear");
    const priorities = values("priority");
    const dues = values("due");
    const projects = values("project");
    if (linearLabels.length !== 1 || linearLabels[0] !== linearId) conflicts.push(`pilot ${linearId} must carry exactly one matching linear: label`);
    if (priorities.length !== 1 || !["P0", "P1", "P2", "P3"].includes(priorities[0])) conflicts.push(`pilot ${linearId} must carry exactly one valid priority: label`);
    if (dues.length > 1 || (dues.length === 1 && !/^\d{4}-\d{2}-\d{2}$/.test(dues[0]))) {
      conflicts.push(`pilot ${linearId} must carry at most one valid date due: label`);
    }
    if (projects.length !== 1 || !projects[0]) conflicts.push(`pilot ${linearId} must carry exactly one project: label`);
  }
  return conflicts;
}

function validateMaps(stateMap, userMap, projectMap) {
  const conflicts = [];
  const statuses = new Set(["triage", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"]);
  for (const [key, value] of Object.entries(stateMap)) {
    const status = typeof value === "string" ? value : value?.status;
    if (!statuses.has(status)) conflicts.push(`state map ${key} has invalid status ${status}`);
  }
  for (const [key, value] of Object.entries(userMap)) {
    const pubkey = value?.pubkey ?? value;
    if (pubkey !== null && !/^[0-9a-f]{64}$/.test(pubkey ?? "")) conflicts.push(`user map ${key} has invalid pubkey`);
  }
  const seenProjects = new Map();
  for (const [key, value] of Object.entries(projectMap)) {
    if (!value || !/^[0-9a-f]{8}-[0-9a-f]{4}-[45][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value.project_id ?? "")) {
      conflicts.push(`project map ${key} has invalid project_id`);
      continue;
    }
    if (!value.name?.trim() || !value.slug?.trim()) conflicts.push(`project map ${key} requires name and slug`);
    const configured = value.statuses ?? [...statuses];
    if (!configured.length || new Set(configured).size !== configured.length || configured.some((status) => !statuses.has(status))) {
      conflicts.push(`project map ${key} has invalid statuses`);
    }
    const identity = `${value.project_id}:${value.slug}`;
    const prior = seenProjects.get(value.project_id);
    if (prior && prior !== identity) conflicts.push(`project UUID ${value.project_id} maps to conflicting slugs`);
    seenProjects.set(value.project_id, identity);
  }
  return conflicts;
}

function pilotFields(pilot, projectMap) {
  if (!pilot) return {};
  const labels = pilot.labels ?? [];
  const value = (prefix) => labels.find((label) => label.startsWith(`${prefix}:`))?.slice(prefix.length + 1);
  const projectSlug = value("project");
  const project = Object.values(projectMap).find((candidate) => candidate?.slug === projectSlug);
  return { priority: value("priority"), due: value("due"), project, projectSlug };
}

function buildPlan(flags) {
  const inputPath = required(flags, "input");
  const inputBytes = readFileSync(inputPath);
  const source = normalizeExport(JSON.parse(inputBytes.toString("utf8")));
  const stateMap = loadJson(required(flags, "state_map"));
  const userMap = loadJson(required(flags, "user_map"));
  const projectMap = loadJson(required(flags, "project_map"));
  const pilotMap = loadJson(required(flags, "pilot_map"));
  const workspace = required(flags, "workspace");
  const issueIds = new Set(source.issues.map((issue) => issue.id));
  if (issueIds.size !== source.issues.length) fail("export contains duplicate Linear issue IDs");

  const unmappedStates = [];
  const unmappedUsers = [];
  const unmappedProjects = [];
  const states = {};
  const teams = {};
  const projects = {};
  const lifecycle = { terminal: 0, non_terminal: 0, archived: 0 };
  const conflicts = [
    ...validateMaps(stateMap, userMap, projectMap),
    ...validatePilotMap(pilotMap, issueIds),
  ];
  for (const issue of source.issues) {
    if (typeof issue.title !== "string" || !issue.title.trim()) {
      conflicts.push(`issue ${issue.id} title must be non-empty text`);
    } else if (Buffer.byteLength(issue.title.trim(), "utf8") > 256) {
      conflicts.push(`issue ${issue.id} title exceeds the Tracker CLI 256-byte limit`);
    }
    const labels = issueLabels(issue);
    if (labels.length > 16) conflicts.push(`issue ${issue.id} has ${labels.length} labels; Tracker CLI limit is 16`);
    const sourceState = stateKey(issue);
    const mappedState = lookup(stateMap, [sourceState, issue.state?.name, issue.state?.type]);
    if (!mappedState) unmappedStates.push({ issue_id: issue.id, state: sourceState });
    states[sourceState ?? "unknown"] = (states[sourceState ?? "unknown"] ?? 0) + 1;
    increment(teams, issue.team?.key ?? issue.team?.name ?? issue.team_id);
    increment(projects, projectKey(issue.project));
    const assigneeKey = userKey(issue.assignee);
    if (assigneeKey && lookup(userMap, [assigneeKey, issue.assignee?.email, issue.assignee?.name]) === undefined) {
      unmappedUsers.push({ issue_id: issue.id, user: assigneeKey });
    }
    const sourceProject = projectKey(issue.project);
    if (sourceProject && lookup(projectMap, [sourceProject, issue.project?.name]) === undefined) {
      unmappedProjects.push({ issue_id: issue.id, project: sourceProject });
    }
    const status = typeof mappedState === "string" ? mappedState : mappedState?.status;
    if (["done", "cancelled"].includes(status)) lifecycle.terminal += 1;
    else lifecycle.non_terminal += 1;
    if (issue.archivedAt != null || issue.archived === true) lifecycle.archived += 1;
  }
  for (const comment of source.comments) {
    const sourceIssueId = comment.issue_id ?? comment.issue?.id;
    if (!sourceIssueId || !issueIds.has(sourceIssueId)) {
      conflicts.push(`comment ${comment.id} references absent issue ${sourceIssueId ?? "<missing>"}`);
    }
    const authorKey = userKey(comment.user ?? comment.creator);
    if (
      authorKey &&
      lookup(userMap, [authorKey, comment.user?.email, comment.user?.name, comment.creator?.email, comment.creator?.name]) === undefined
    ) {
      unmappedUsers.push({ comment_id: comment.id, user: authorKey });
    }
  }
  const relations = relationRecords(source.issues);
  const links = linkRecords(source.issues);
  for (const [linearId, pilot] of Object.entries(pilotMap)) {
    const fields = pilotFields(pilot, projectMap);
    if (!fields.project) conflicts.push(`pilot ${linearId} project slug ${fields.projectSlug} is absent from project map`);
    const issue = source.issues.find((candidate) => candidate.id === linearId);
    if (!issue) continue;
    const livePriority = normalizePriority(issue.priority?.label ?? issue.priority);
    const liveDue = issue.dueDate ?? issue.due_date ?? null;
    const liveProject = lookup(projectMap, [projectKey(issue.project), issue.project?.name]);
    const liveProjectSlug = liveProject?.slug ?? null;
    for (const [field, pilotValue, liveValue] of [
      ["priority", fields.priority ?? null, livePriority],
      ["due", fields.due ?? null, liveDue],
      ["project", fields.projectSlug ?? null, liveProjectSlug],
    ]) {
      if (pilotValue !== liveValue) {
        conflicts.push(`pilot ${linearId} ${field} diverges from export (pilot=${JSON.stringify(pilotValue)}, export=${JSON.stringify(liveValue)})`);
      }
    }
  }
  for (const relation of relations) {
    if (!relation.target || !issueIds.has(relation.target)) conflicts.push(`relation from ${relation.source} has missing target ${relation.target}`);
  }
  const actions = {
    create_projects: Object.keys(projectMap).length,
    create_issues: source.issues.length,
    reconcile_pilots: Object.keys(pilotMap).length,
    create_comments: source.comments.length,
    create_relations: relations.length,
    create_links: links.length + Object.keys(pilotMap).length,
  };
  const droppedRelations = droppedRelationRecords(source.issues);
  const plan = {
    mode: "plan",
    importer_version: IMPORTER_VERSION,
    input: inputPath,
    input_sha256: sha256(inputBytes),
    imported_at: sourceSnapshotEpoch(source),
    complete_export: true,
    linear_workspace_id: source.workspaceId,
    tracker_workspace_id: workspace,
    source: {
      issues: source.issues.length,
      comments: source.comments.length,
      projects: source.projects.length,
      relations: relations.length,
      links: links.length,
    },
    states,
    teams,
    projects,
    lifecycle,
    actions,
    pilot_handling: "create Tracker issues for every pilot, then link each legacy NIP-34 event as git_issue",
    unmapped_states: unmappedStates,
    unmapped_users: unmappedUsers,
    unmapped_projects: unmappedProjects,
    conflicts,
    safe_to_apply:
      unmappedStates.length === 0 &&
      unmappedUsers.length === 0 &&
      unmappedProjects.length === 0 &&
      conflicts.length === 0,
  };
  plan.plan_sha256 = sha256(canonicalJson(plan));
  return { plan, source, stateMap, userMap, projectMap, pilotMap };
}

function writeReport(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function sleep(milliseconds) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
}

function buzz(args, input = undefined, allowFailure = false) {
  const command = process.env.BUZZ_BIN ?? "buzz";
  let result;
  for (let attempt = 0; attempt < 5; attempt += 1) {
    result = spawnSync(command, args, { encoding: "utf8", input });
    if (result.status !== 2) break;
    sleep(250 * (2 ** attempt));
  }
  const stdout = result.stdout?.trim() ?? "";
  const stderr = result.stderr?.trim() ?? "";
  if (result.status !== 0 && !allowFailure) {
    fail(`buzz command failed: ${command} ${args.join(" ")}`, {
      exit_code: result.status,
      stdout,
      stderr,
    });
  }
  let parsed = null;
  if (stdout) {
    try {
      parsed = JSON.parse(stdout.split("\n").at(-1));
    } catch {
      if (!allowFailure) fail("buzz command returned non-JSON output", { stdout, stderr });
    }
  }
  if (result.status === 0 && !["list", "get"].includes(args[1])) sleep(50);
  return { status: result.status, stdout, stderr, parsed };
}

function importProvenance(issue, plan, originalAuthor = null, legacyLabels = []) {
  const archived = issue.archivedAt != null || issue.archived === true;
  return {
    system: "linear",
    external_id: issue.id,
    source_url: sourceUrl(issue),
    imported_at: plan.imported_at,
    source_created_at: issue.createdAt ?? issue.created_at ?? null,
    source_updated_at: issue.updatedAt ?? issue.updated_at ?? null,
    source_completed_at: issue.completedAt ?? issue.completed_at ?? null,
    source_cancelled_at: issue.canceledAt ?? issue.cancelled_at ?? null,
    archived: archived ? true : undefined,
    estimate_points: Number.isInteger(issue.estimate) && issue.estimate >= 0 ? issue.estimate : null,
    export_sha256: plan.input_sha256,
    importer_version: IMPORTER_VERSION,
    original_author: originalAuthor ?? undefined,
    legacy_labels: legacyLabels.length ? legacyLabels : undefined,
  };
}

function commentProvenance(comment, issue, plan) {
  const provenance = importProvenance(comment, plan, userKey(comment.user ?? comment.creator));
  provenance.external_id = comment.id;
  provenance.source_url = comment.url ?? sourceUrl(issue);
  provenance.source_created_at = comment.createdAt ?? comment.created_at ?? null;
  provenance.source_updated_at = comment.updatedAt ?? comment.updated_at ?? null;
  return provenance;
}

const PROVENANCE_FIELDS = [
  "system",
  "external_id",
  "source_url",
  "imported_at",
  "source_created_at",
  "source_updated_at",
  "source_completed_at",
  "source_cancelled_at",
  "archived",
  "estimate_points",
  "export_sha256",
  "importer_version",
  "original_author",
  "legacy_labels",
];

function provenanceValue(provenance, field) {
  if (field === "legacy_labels") return provenance?.[field] ?? [];
  return provenance?.[field] ?? null;
}

function compareProvenance(expected, actual) {
  return PROVENANCE_FIELDS.flatMap((field) => {
    const expectedValue = provenanceValue(expected, field);
    const actualValue = provenanceValue(actual, field);
    return canonicalJson(expectedValue) === canonicalJson(actualValue)
      ? []
      : [{ field, expected: expectedValue, actual: actualValue }];
  });
}

function mappedIssue(issue, context) {
  const state = lookup(context.stateMap, [stateKey(issue), issue.state?.name, issue.state?.type]);
  const assignee = lookup(context.userMap, [userKey(issue.assignee), issue.assignee?.email, issue.assignee?.name]);
  const pilot = context.pilotMap[issue.id] ?? null;
  const project = lookup(context.projectMap, [projectKey(issue.project), issue.project?.name]);
  const issueId = uuidV5(LINEAR_NAMESPACE, `${context.source.workspaceId}:${issue.id}`);
  const requestId = uuidV5(LINEAR_NAMESPACE, `${issue.id}:root:${issue.updatedAt ?? issue.updated_at ?? context.plan.input_sha256}`);
  const labels = issueLabels(issue);
  const legacyLabels = pilot?.labels ?? [];
  return {
    state, assignee, project, pilot, issueId, requestId, labels, legacyLabels,
    priority: normalizePriority(issue.priority?.label ?? issue.priority),
    due: issue.dueDate ?? issue.due_date ?? null,
  };
}

function checkpoint(path, planHash) {
  let value;
  try {
    value = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") return { plan_sha256: planHash, completed: {} };
    fail(`cannot parse checkpoint ${path}`, { detail: String(error) });
  }
  if (value?.plan_sha256 !== planHash) fail("checkpoint belongs to a different migration plan");
  if (!value.completed || typeof value.completed !== "object" || Array.isArray(value.completed)) {
    fail("checkpoint completed field must be an object");
  }
  return value;
}

function saveCheckpoint(path, value) {
  const temporary = `${path}.tmp-${process.pid}`;
  try {
    writeReport(temporary, value);
    renameSync(temporary, path);
  } catch (error) {
    try {
      unlinkSync(temporary);
    } catch {}
    fail(`cannot write checkpoint ${path}`, { detail: String(error) });
  }
}

function recordApply(report, checkpointValue, checkpointPath, key, receipt) {
  const row = { key, accepted: true, receipt };
  report.records.push(row);
  checkpointValue.completed[key] = receipt;
  saveCheckpoint(checkpointPath, checkpointValue);
}

function applyPlan(flags, context) {
  const confirmed = required(flags, "confirm_plan_sha256");
  if (confirmed !== context.plan.plan_sha256) fail("--confirm-plan-sha256 does not match the deterministic plan");
  if (!context.plan.safe_to_apply) fail("plan is not safe to apply", { plan_sha256: context.plan.plan_sha256 });
  const checkpointPath = required(flags, "checkpoint");
  const checkpointValue = checkpoint(checkpointPath, context.plan.plan_sha256);
  if (Object.keys(checkpointValue.completed).length && !flags.resume) {
    fail("checkpoint contains progress; pass --resume or use a fresh checkpoint");
  }
  const report = { mode: "apply", plan_sha256: context.plan.plan_sha256, records: [], failures: [] };
  const workspace = context.plan.tracker_workspace_id;

  const uniqueProjects = new Map();
  for (const value of Object.values(context.projectMap)) {
    if (value?.project_id) uniqueProjects.set(value.project_id, value);
  }
  for (const project of uniqueProjects.values()) {
    const key = `project:${project.project_id}`;
    if (checkpointValue.completed[key]) continue;
    const existing = buzz(["track", "project", "get", "--workspace", workspace, "--project", project.project_id], undefined, true);
    if (existing.status === 0) {
      recordApply(report, checkpointValue, checkpointPath, key, existing.parsed);
      continue;
    }
    const requestId = uuidV5(LINEAR_NAMESPACE, `project:${project.project_id}`);
    const args = [
      "track", "project", "create", "--workspace", workspace,
      "--project-id", project.project_id,
      "--name", project.name,
      "--slug", project.slug,
      "--request-id", requestId,
    ];
    for (const status of project.statuses ?? ["triage", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"]) {
      args.push("--status", status);
    }
    if (project.default_channel_id) args.push("--default-channel", project.default_channel_id);
    if (project.default_verify) args.push("--default-verify", project.default_verify);
    recordApply(report, checkpointValue, checkpointPath, key, buzz(args).parsed);
  }

  const issueMap = new Map();
  for (const issue of context.source.issues) {
    const mapped = mappedIssue(issue, context);
    issueMap.set(issue.id, mapped.issueId);
    const key = `issue:${issue.id}`;
    if (checkpointValue.completed[key]) continue;
    const project = mapped.project;
    const channel = issue.channel_id ?? project?.default_channel_id ?? flags.default_channel;
    if (!channel) fail(`issue ${issue.id} has no mapped home channel`);
    const provenance = importProvenance(issue, context.plan, userKey(issue.creator), mapped.legacyLabels);
    const args = [
      "track", "create", "--workspace", workspace,
      "--issue-id", mapped.issueId,
      "--identifier", issueIdentifier(issue),
      "--channel", channel,
      "--title", issue.title,
      "--body", issue.description ?? issue.body ?? "",
      "--status", typeof mapped.state === "string" ? mapped.state : mapped.state.status,
      "--request-id", mapped.requestId,
      "--import-json", JSON.stringify(provenance),
      "--external-id", `linear:${issue.id}`,
    ];
    if (mapped.priority) args.push("--priority", mapped.priority);
    if (mapped.assignee) args.push("--assignee", mapped.assignee.pubkey ?? mapped.assignee);
    if (project?.project_id) args.push("--project", project.project_id);
    if (mapped.due ?? issue.dueDate ?? issue.due_date) args.push("--due", mapped.due ?? issue.dueDate ?? issue.due_date, "--tz", flags.timezone ?? "America/New_York");
    for (const label of mapped.labels) args.push("--label", label);
    recordApply(report, checkpointValue, checkpointPath, key, buzz(args).parsed);

    if (mapped.pilot) {
      const pilotKey = `pilot:${issue.id}`;
      if (!checkpointValue.completed[pilotKey]) {
        const requestId = uuidV5(LINEAR_NAMESPACE, `${issue.id}:pilot:${mapped.pilot.event_id}`);
        const receipt = buzz([
          "track", "link", "--workspace", workspace, "--issue", mapped.issueId,
          "--type", "git_issue", "--ref", mapped.pilot.event_id,
          "--request-id", requestId, "--import-json", JSON.stringify(provenance),
        ]).parsed;
        recordApply(report, checkpointValue, checkpointPath, pilotKey, receipt);
      }
    }
  }

  for (const relation of relationRecords(context.source.issues)) {
    const key = `relation:${relation.source}:${relation.type}:${relation.target}`;
    if (checkpointValue.completed[key]) continue;
    const type = RELATION_TYPE_MAP[relation.type];
    const requestId = uuidV5(LINEAR_NAMESPACE, key);
    const receipt = buzz([
      "track", "relate", "--workspace", workspace,
      "--issue", issueMap.get(relation.source), "--type", type,
      "--target", issueMap.get(relation.target), "--request-id", requestId,
      "--import-json", JSON.stringify(importProvenance(
        context.source.issues.find((issue) => issue.id === relation.source), context.plan,
      )),
    ]).parsed;
    recordApply(report, checkpointValue, checkpointPath, key, receipt);
  }

  for (const link of linkRecords(context.source.issues)) {
    const key = `link:${link.issue}:${link.type}:${link.ref}`;
    if (checkpointValue.completed[key]) continue;
    const requestId = uuidV5(LINEAR_NAMESPACE, key);
    const receipt = buzz([
      "track", "link", "--workspace", workspace,
      "--issue", issueMap.get(link.issue), "--type", link.type,
      "--ref", link.ref, "--request-id", requestId,
      "--import-json", JSON.stringify(importProvenance(
        context.source.issues.find((issue) => issue.id === link.issue), context.plan,
      )),
    ]).parsed;
    recordApply(report, checkpointValue, checkpointPath, key, receipt);
  }

  for (const comment of context.source.comments) {
    const sourceIssueId = comment.issue_id ?? comment.issue?.id;
    const key = `comment:${sourceIssueId}:${comment.id}`;
    if (checkpointValue.completed[key]) continue;
    const sourceIssue = context.source.issues.find((issue) => issue.id === sourceIssueId);
    const requestId = uuidV5(LINEAR_NAMESPACE, `${key}:${comment.updatedAt ?? comment.createdAt ?? "unknown"}`);
    const provenance = commentProvenance(comment, sourceIssue, context.plan);
    const receipt = buzz([
      "track", "comment", "--workspace", workspace,
      "--issue", issueMap.get(sourceIssueId), "--content", comment.body ?? comment.content,
      "--request-id", requestId, "--import-json", JSON.stringify(provenance),
    ]).parsed;
    recordApply(report, checkpointValue, checkpointPath, key, receipt);
  }
  report.passed = true;
  return report;
}

function collectTrackerIssues(workspace) {
  const items = [];
  let cursor = null;
  do {
    const args = ["track", "list", "--workspace", workspace, "--include-closed", "--sort", "identifier", "--order", "asc", "--limit", "500"];
    if (cursor) args.push("--cursor", cursor);
    const page = buzz(args).parsed;
    items.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  return items;
}

function verifyPlan(context) {
  if (!context.plan.safe_to_apply) fail("cannot verify an unsafe plan");
  const workspace = context.plan.tracker_workspace_id;
  const summaries = collectTrackerIssues(workspace);
  const trackerByLinear = new Map();
  const duplicateSourceIds = [];
  const unexpectedTrackerIssues = [];
  const details = [];
  for (const summary of summaries) {
    const detail = buzz(["track", "get", "--workspace", workspace, "--issue", summary.issue_id]).parsed;
    details.push(detail.issue);
    const linearRefs = [];
    for (const ref of detail.issue.external_refs ?? []) {
      if (ref.type !== "linear" && ref.kind !== "linear") continue;
      const sourceId = ref.ref ?? ref.reference;
      linearRefs.push(sourceId);
      if (trackerByLinear.has(sourceId)) duplicateSourceIds.push(sourceId);
      trackerByLinear.set(sourceId, detail.issue);
    }
    if (linearRefs.length === 0) {
      unexpectedTrackerIssues.push({
        issue_id: detail.issue.issue_id,
        identifier: detail.issue.identifier ?? null,
        reason: "missing linear external reference",
      });
    }
  }
  const sourceIds = new Set(context.source.issues.map((issue) => issue.id));
  const missingSourceIds = [...sourceIds].filter((id) => !trackerByLinear.has(id));
  const unexpectedSourceIds = [...trackerByLinear.keys()].filter((id) => !sourceIds.has(id));
  const fieldMismatches = [];
  const provenanceMismatches = [];
  const issueMap = new Map();
  for (const issue of context.source.issues) {
    const target = trackerByLinear.get(issue.id);
    if (!target) continue;
    const mapped = mappedIssue(issue, context);
    issueMap.set(issue.id, mapped.issueId);
    const expectedStatus = typeof mapped.state === "string" ? mapped.state : mapped.state.status;
    if (target.identifier !== issueIdentifier(issue)) fieldMismatches.push({ source_id: issue.id, field: "identifier", expected: issueIdentifier(issue), actual: target.identifier });
    if (target.title !== issue.title) fieldMismatches.push({ source_id: issue.id, field: "title", expected: issue.title, actual: target.title });
    if (target.body !== (issue.description ?? issue.body ?? "")) fieldMismatches.push({ source_id: issue.id, field: "body" });
    if (target.status !== expectedStatus) fieldMismatches.push({ source_id: issue.id, field: "status", expected: expectedStatus, actual: target.status });
    if (mapped.project?.project_id && target.project?.project_id !== mapped.project.project_id) fieldMismatches.push({ source_id: issue.id, field: "project_id", expected: mapped.project.project_id, actual: target.project?.project_id });
    const expectedAssignee = mapped.assignee ? (mapped.assignee.pubkey ?? mapped.assignee) : null;
    if ((target.assignee_pubkey ?? null) !== expectedAssignee) fieldMismatches.push({ source_id: issue.id, field: "assignee_pubkey", expected: expectedAssignee, actual: target.assignee_pubkey ?? null });
    const expectedPriority = mapped.priority;
    if ((target.priority ?? null) !== expectedPriority) fieldMismatches.push({ source_id: issue.id, field: "priority", expected: expectedPriority, actual: target.priority ?? null });
    const expectedLabels = [...mapped.labels].sort();
    const actualLabels = [...(target.labels ?? [])].sort();
    if (canonicalJson(actualLabels) !== canonicalJson(expectedLabels)) fieldMismatches.push({ source_id: issue.id, field: "labels", expected: expectedLabels, actual: actualLabels });
    const expectedDue = mapped.due ?? issue.dueDate ?? issue.due_date ?? null;
    if ((target.due ?? null) !== expectedDue) fieldMismatches.push({ source_id: issue.id, field: "due", expected: expectedDue, actual: target.due ?? null });
    const expectedChannel = issue.channel_id ?? mapped.project?.default_channel_id ?? null;
    if (expectedChannel && target.home_channel_id !== expectedChannel) fieldMismatches.push({ source_id: issue.id, field: "home_channel_id", expected: expectedChannel, actual: target.home_channel_id });
    const rootActivity = (target.activity ?? []).find((activity) => activity.type === "created");
    const provenance = rootActivity?.import_provenance;
    const expectedProvenance = importProvenance(
      issue,
      context.plan,
      userKey(issue.creator),
      mapped.legacyLabels,
    );
    for (const mismatch of compareProvenance(expectedProvenance, provenance)) {
      provenanceMismatches.push({ source_id: issue.id, ...mismatch });
    }
  }
  const importedComments = new Map();
  for (const detail of details) {
    for (const comment of detail.comments ?? []) {
      if (comment.import?.system === "linear") importedComments.set(comment.import.external_id, { ...comment, issue_id: detail.issue_id });
    }
  }
  const sourceComments = new Map(context.source.comments.map((comment) => [comment.id, comment]));
  const sourceCommentIds = new Set(sourceComments.keys());
  const importedCommentIds = new Set(importedComments.keys());
  const commentMismatches = [
    ...[...sourceCommentIds].filter((id) => !importedCommentIds.has(id)).map((id) => ({ missing_source_comment_id: id })),
    ...[...importedCommentIds].filter((id) => !sourceCommentIds.has(id)).map((id) => ({ unexpected_source_comment_id: id })),
  ];
  for (const [id, sourceComment] of sourceComments) {
    const target = importedComments.get(id);
    if (!target) continue;
    const sourceIssueId = sourceComment.issue_id ?? sourceComment.issue?.id;
    if (target.issue_id !== issueMap.get(sourceIssueId) || target.body !== (sourceComment.body ?? sourceComment.content)) {
      commentMismatches.push({ source_comment_id: id, field: "issue_or_body" });
    }
    const sourceIssue = context.source.issues.find((issue) => issue.id === sourceIssueId);
    const expectedProvenance = commentProvenance(sourceComment, sourceIssue, context.plan);
    for (const mismatch of compareProvenance(expectedProvenance, target.import)) {
      commentMismatches.push({ source_comment_id: id, field: `import.${mismatch.field}`, expected: mismatch.expected, actual: mismatch.actual });
    }
  }
  const expectedRelations = new Set(relationRecords(context.source.issues).map((relation) =>
    relationKey(issueMap.get(relation.source), relation.type, issueMap.get(relation.target)),
  ));
  const actualRelations = new Set(details.flatMap((detail) =>
    (detail.relations ?? []).map((relation) => relationKey(detail.issue_id, relation.type, relation.target_issue_id)),
  ));
  const relationMismatches = [
    ...setDifference(expectedRelations, actualRelations).map((key) => ({ missing: key })),
    ...setDifference(actualRelations, expectedRelations).map((key) => ({ unexpected: key })),
  ];
  const expectedLinks = new Set();
  for (const issue of context.source.issues) expectedLinks.add(linkKey(issueMap.get(issue.id), "linear", issue.id));
  for (const link of linkRecords(context.source.issues)) expectedLinks.add(linkKey(issueMap.get(link.issue), link.type, link.ref));
  for (const [linearId, pilot] of Object.entries(context.pilotMap)) expectedLinks.add(linkKey(issueMap.get(linearId), "git_issue", pilot.event_id));
  const actualLinks = new Set(details.flatMap((detail) =>
    (detail.external_refs ?? []).map((link) => linkKey(detail.issue_id, link.type ?? link.kind, link.ref ?? link.reference)),
  ));
  const linkMismatches = [
    ...setDifference(expectedLinks, actualLinks).map((key) => ({ missing: key })),
    ...setDifference(actualLinks, expectedLinks).map((key) => ({ unexpected: key })),
  ];
  const expectedProjects = new Map();
  for (const project of Object.values(context.projectMap)) if (project?.project_id) expectedProjects.set(project.project_id, project);
  const actualProjects = new Map();
  let projectCursor = null;
  do {
    const args = ["track", "project", "list", "--workspace", workspace, "--limit", "500"];
    if (projectCursor) args.push("--cursor", projectCursor);
    const page = buzz(args).parsed;
    for (const project of page.items) actualProjects.set(project.project_id, project);
    projectCursor = page.next_cursor;
  } while (projectCursor);
  const projectMismatches = [];
  for (const [id, expected] of expectedProjects) {
    const actual = actualProjects.get(id);
    if (!actual) projectMismatches.push({ missing_project_id: id });
    else for (const field of ["name", "slug", "statuses", "default_channel_id", "default_verify"]) {
      const expectedValue = expected[field] ?? (field === "statuses" ? ["triage", "backlog", "todo", "in_progress", "in_review", "done", "cancelled"] : null);
      if (canonicalJson(actual[field] ?? null) !== canonicalJson(expectedValue)) projectMismatches.push({ project_id: id, field, expected: expectedValue, actual: actual[field] ?? null });
    }
  }
  for (const id of actualProjects.keys()) if (!expectedProjects.has(id)) projectMismatches.push({ unexpected_project_id: id });
  const result = {
    mode: "verify",
    plan_sha256: context.plan.plan_sha256,
    source_issue_count: sourceIds.size,
    tracker_issue_count: details.length,
    missing_source_ids: missingSourceIds,
    unexpected_source_ids: unexpectedSourceIds,
    unexpected_tracker_issues: unexpectedTrackerIssues,
    duplicate_source_ids: [...new Set(duplicateSourceIds)],
    field_mismatches: fieldMismatches,
    provenance_mismatches: provenanceMismatches,
    comment_mismatches: commentMismatches,
    relation_mismatches: relationMismatches,
    link_mismatches: linkMismatches,
    project_mismatches: projectMismatches,
    pilot_links_verified: Object.entries(context.pilotMap).filter(([id, pilot]) => actualLinks.has(linkKey(issueMap.get(id), "git_issue", pilot.event_id))).length,
    closed_history_verified: context.source.issues.filter((issue) => ["done", "cancelled"].includes(
      typeof lookup(context.stateMap, [stateKey(issue), issue.state?.name, issue.state?.type]) === "string"
        ? lookup(context.stateMap, [stateKey(issue), issue.state?.name, issue.state?.type])
        : lookup(context.stateMap, [stateKey(issue), issue.state?.name, issue.state?.type])?.status,
    )).length,
  };
  result.passed = [
    missingSourceIds,
    unexpectedSourceIds,
    unexpectedTrackerIssues,
    duplicateSourceIds,
    fieldMismatches,
    provenanceMismatches,
    commentMismatches,
    relationMismatches,
    linkMismatches,
    projectMismatches,
  ].every((values) => values.length === 0) && result.source_issue_count === result.tracker_issue_count;
  return result;
}

const flags = parseArgs(process.argv.slice(2));
const context = buildPlan(flags);
if (flags.mode === "plan") {
  writeReport(required(flags, "report"), context.plan);
  process.stdout.write(`${JSON.stringify(context.plan)}\n`);
  process.exit(context.plan.safe_to_apply ? 0 : 1);
}
if (flags.mode === "apply") {
  const report = applyPlan(flags, context);
  writeReport(required(flags, "report"), report);
  process.stdout.write(`${JSON.stringify(report)}\n`);
  process.exit(report.passed ? 0 : 1);
}
const report = verifyPlan(context);
writeReport(required(flags, "report"), report);
process.stdout.write(`${JSON.stringify(report)}\n`);
process.exit(report.passed ? 0 : 1);
