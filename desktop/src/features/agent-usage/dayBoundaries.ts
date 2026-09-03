/**
 * Local-midnight bucket boundaries for the NIP-AM usage series.
 *
 * The backend takes exact Unix-second boundaries and treats each adjacent pair
 * as one `[start, end)` bucket, so N boundaries describe N-1 days. They must be
 * built from real calendar arithmetic, never `N * 86_400`: on a DST transition
 * a civil day is 23 or 25 hours, and a fixed stride would silently slide every
 * bucket edge away from local midnight for the rest of the range.
 */

/** Widest interval the backend admits (`MAX_INTERVAL_SECS`). */
const MAX_BUCKET_SECONDS = 48 * 3600;

/** Backend's `MAX_BOUNDARIES` — 366 daily buckets, one leap year. */
const MAX_BOUNDARIES = 367;

function startOfLocalDay(date: Date): Date {
  const d = new Date(date);
  d.setHours(0, 0, 0, 0);
  return d;
}

/**
 * Boundaries for the last `days` local days, ending at the local midnight that
 * starts *tomorrow* so the current, partial day is the final bucket.
 *
 * Returns `days + 1` boundaries. `days` is clamped to the backend's accepted
 * range, so a caller cannot construct a request the validator will reject.
 */
export function lastNDaysBoundaries(
  days: number,
  now: Date = new Date(),
): number[] {
  const bucketCount = Math.max(
    1,
    Math.min(Math.floor(days), MAX_BOUNDARIES - 1),
  );
  const end = startOfLocalDay(now);
  end.setDate(end.getDate() + 1);

  const boundaries: number[] = [];
  for (let i = bucketCount; i >= 0; i -= 1) {
    const d = new Date(end);
    // setDate() rolls months and years, and re-normalises to local midnight
    // across a DST shift — the property a fixed 86_400 stride cannot hold.
    d.setDate(d.getDate() - i);
    boundaries.push(Math.floor(d.getTime() / 1000));
  }
  return boundaries;
}

/**
 * Whether a boundary list satisfies the backend's validator: 2..=367 entries,
 * strictly increasing, and no interval wider than 48 hours.
 *
 * Exported so the caller can fail fast on a clock that jumped backwards mid-
 * render rather than surfacing a validation error string from Rust.
 */
export function areBoundariesValid(boundaries: number[]): boolean {
  if (boundaries.length < 2 || boundaries.length > MAX_BOUNDARIES) return false;
  for (let i = 0; i < boundaries.length - 1; i += 1) {
    const interval = boundaries[i + 1] - boundaries[i];
    if (interval <= 0 || interval > MAX_BUCKET_SECONDS) return false;
  }
  return true;
}
