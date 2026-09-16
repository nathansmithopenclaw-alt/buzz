import type {
  DispatchDue,
  DispatchExternalRef,
  DispatchIssue,
  DispatchRelation,
} from "./types";

export type DispatchIssueEdit = Partial<
  Pick<
    DispatchIssue,
    | "status"
    | "priority"
    | "assignee"
    | "title"
    | "body"
    | "projectId"
    | "labels"
    | "due"
    | "estimateMinutes"
    | "homeChannelId"
    | "identifier"
  >
> & { relations?: DispatchRelation[]; externalRefs?: DispatchExternalRef[] };

export function editRevisionFields(
  issue: DispatchIssue,
  fields: DispatchIssueEdit,
) {
  const set: Record<string, unknown> = {};
  const clear: string[] = [];
  const keys = {
    status: "status",
    priority: "priority",
    assignee: "assignee",
    title: "title",
    body: "body",
    projectId: "project_id",
    labels: "labels",
    due: "due",
    estimateMinutes: "estimate_minutes",
    homeChannelId: "home_channel_id",
    identifier: "identifier",
  } as const;
  for (const [key, wireKey] of Object.entries(keys)) {
    const typedKey = key as keyof typeof keys;
    let value = fields[typedKey];
    if (
      value === undefined ||
      JSON.stringify(value) === JSON.stringify(issue[typedKey])
    )
      continue;
    if (key === "title" || key === "identifier") {
      if (typeof value !== "string" || !value.trim())
        throw new Error(
          `${key === "title" ? "Title" : "Identifier"} cannot be empty.`,
        );
      value = value.trim();
    }
    if (key === "labels")
      value = [
        ...new Set(
          (value as string[])
            .map((label) => label.trim().toLowerCase())
            .filter(Boolean),
        ),
      ].sort();
    if (
      key === "estimateMinutes" &&
      value !== null &&
      (!Number.isSafeInteger(value) || Number(value) < 0)
    )
      throw new Error("Estimate must be a whole number of minutes.");
    if (value === null) clear.push(wireKey);
    else set[wireKey] = value;
  }
  const relationKey = (value: DispatchRelation) =>
    `${value.kind}:${value.targetIssueId}:${value.targetRootEvent}`;
  const refKey = (value: DispatchExternalRef) => `${value.type}:${value.ref}`;
  function difference<T>(
    previous: T[],
    next: T[] | undefined,
    key: (value: T) => string,
  ) {
    if (!next) return { add: [], remove: [] };
    return {
      add: next.filter(
        (value) => !previous.some((old) => key(old) === key(value)),
      ),
      remove: previous.filter(
        (old) => !next.some((value) => key(old) === key(value)),
      ),
    };
  }
  const relations = difference(issue.relations, fields.relations, relationKey);
  return {
    set,
    clear,
    relations: {
      add: relations.add.map(relationWire),
      remove: relations.remove.map(relationWire),
    },
    external_refs: difference(issue.externalRefs, fields.externalRefs, refKey),
  };
}

function relationWire(value: DispatchRelation) {
  return {
    type: value.kind,
    target_issue_id: value.targetIssueId,
    target_root_event: value.targetRootEvent,
  };
}

/** Store a human calendar date at midnight in the device's actual IANA timezone. */
export function dueFromDateInput(value: string): DispatchDue | null {
  if (!value) return null;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value))
    throw new Error("Choose a valid due date.");
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(year, month - 1, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  )
    throw new Error("Choose a valid due date.");
  return {
    value: Math.floor(date.valueOf() / 1000),
    precision: "date",
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  };
}

export function dueDateInput(due: DispatchDue | null): string {
  if (!due) return "";
  const parts = new Intl.DateTimeFormat("en-US", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    timeZone: due.timezone,
  }).formatToParts(new Date(due.value * 1000));
  const part = (key: string) =>
    parts.find((part) => part.type === key)?.value ?? "";
  return `${part("year")}-${part("month")}-${part("day")}`;
}

/** Display an unambiguous short alias without changing any durable identifier. */
export function issueAlias(
  issue: Pick<DispatchIssue, "identifier" | "issueId">,
  issues: readonly Pick<DispatchIssue, "identifier" | "issueId">[] = [],
): string {
  const match = /^([a-z][a-z0-9]{0,11})-([0-9a-f]{32})$/i.exec(
    issue.identifier,
  );
  if (!match) return issue.identifier;
  const hex = match[2].toUpperCase();
  for (let length = 6; length <= hex.length; length += 2) {
    const alias = `${match[1].toUpperCase()}-${hex.slice(0, length)}`;
    if (
      !issues.some(
        (other) =>
          other.issueId !== issue.issueId &&
          other.identifier.toUpperCase().startsWith(alias),
      )
    )
      return alias;
  }
  return issue.identifier;
}
