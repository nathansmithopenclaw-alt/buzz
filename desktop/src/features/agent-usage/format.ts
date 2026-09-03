import type {
  AgentSessionStarts,
  AgentTurnProvenance,
  CostField,
  UsageField,
} from "@/shared/api/tauriArchive";

/**
 * Rendering rules for NIP-AM usage values.
 *
 * The single invariant this module exists to hold: **unknown is not zero.** The
 * backend distinguishes "no events reported this field" (`value: null`) from
 * "the reported total is a lower bound" (`incomplete: true`) from an exact
 * figure, and a UI that flattens those into a number tells the operator their
 * agents used nothing when the truth is that nobody measured.
 */

/** What the operator should be told about a single usage value. */
export type UsageDisplay = {
  /** Formatted number, or `null` when nothing is known. */
  text: string | null;
  /** True when the value is a reported lower bound rather than a total. */
  partial: boolean;
};

/** Rendered in place of a number that is not known. Never "0". */
export const UNKNOWN_PLACEHOLDER = "—";

export function formatTokenCount(raw: string | null): string | null {
  if (raw === null) return null;
  // Counters cross 2^53, so they arrive as decimal strings. Group with BigInt
  // rather than Number to avoid silently rounding a large total.
  let value: bigint;
  try {
    value = BigInt(raw);
  } catch {
    return null;
  }
  return value.toLocaleString("en-US");
}

export function usageDisplay(field: UsageField): UsageDisplay {
  return { text: formatTokenCount(field.value), partial: field.incomplete };
}

export function costDisplay(field: CostField): UsageDisplay {
  if (field.value === null) return { text: null, partial: field.incomplete };
  return {
    text: field.value.toLocaleString("en-US", {
      style: "currency",
      currency: "USD",
      maximumFractionDigits: field.value < 1 ? 4 : 2,
    }),
    partial: field.incomplete,
  };
}

/**
 * Cache-read share of inclusive input, as a percentage string.
 *
 * `null` unless BOTH figures are exact: a ratio built from a lower bound is not
 * a ratio, and showing "98%" derived from partial data is worse than showing
 * nothing.
 */
export function cacheHitRate(
  input: UsageField,
  cacheRead: UsageField,
): string | null {
  if (input.incomplete || cacheRead.incomplete) return null;
  if (input.value === null || cacheRead.value === null) return null;
  let inputValue: bigint;
  let cacheValue: bigint;
  try {
    inputValue = BigInt(input.value);
    cacheValue = BigInt(cacheRead.value);
  } catch {
    return null;
  }
  if (inputValue === 0n) return null;
  // Scale before dividing: integer division would floor every ratio to 0.
  const basisPoints = Number((cacheValue * 10_000n) / inputValue);
  return `${(basisPoints / 100).toFixed(1)}%`;
}

/** Human label for a NIP-AM `sessionCreationCause` wire value. */
const CAUSE_LABELS: Record<string, string> = {
  worker_spawn: "Worker started",
  scope_first_turn: "First turn in channel",
  rotation_turn_limit: "Turn-limit rotation",
  provider_limit: "Provider limit",
  owner_rotate: "Owner rotated",
  owner_interrupt: "Owner interrupted",
  model_switch: "Model switch",
  membership_change: "Membership change",
  adapter_failure: "Adapter failure",
  agent_exited: "Agent exited",
  turn_timeout: "Turn timeout",
  error: "Error",
};

export function sessionCauseLabel(cause: string | null): string {
  if (cause === null) return "Not reported";
  // A cause this build has never heard of must render as itself, not as
  // "unknown": NIP-AM lets publishers add causes, and swallowing the wire
  // value would hide a real teardown reason behind a shrug.
  return CAUSE_LABELS[cause] ?? cause;
}

/**
 * Whether a cause also names why the previous session in that scope ended.
 *
 * `worker_spawn` and `scope_first_turn` are the two "no predecessor" values, so
 * they must not be counted in a teardown histogram.
 */
export function isTeardownCause(cause: string | null): boolean {
  return (
    cause !== null && cause !== "worker_spawn" && cause !== "scope_first_turn"
  );
}

export function totalSessions(starts: AgentSessionStarts[]): number {
  return starts.reduce((sum, s) => sum + s.sessions, 0);
}

/**
 * One-line summary of a provenance count against its denominator.
 *
 * Returns `null` when nothing reported the field, so the caller renders "not
 * reported" rather than "0" — `0 of 0` is not evidence of zero retries.
 */
export function provenanceRatio(
  count: number,
  denominator: number,
): string | null {
  if (denominator <= 0) return null;
  return `${count.toLocaleString("en-US")} of ${denominator.toLocaleString("en-US")}`;
}

export function peakContextSummary(
  provenance: AgentTurnProvenance,
): string | null {
  const peak = formatTokenCount(provenance.peakContextTokens);
  if (peak === null) return null;
  const window = formatTokenCount(provenance.contextWindow);
  return window === null ? peak : `${peak} / ${window}`;
}
