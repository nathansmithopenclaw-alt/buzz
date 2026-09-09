/** Exact daily times, grouped by minute to avoid cron's hour/minute cross-product. */
export function dailyTimesToCrons(times: readonly string[]): string[] {
  if (
    !times.length ||
    times.length > 24 ||
    new Set(times).size !== times.length ||
    times.some((t) => !/^([01]\d|2[0-3]):[0-5]\d$/.test(t))
  )
    throw new Error("Choose 1–24 unique daily times.");
  const groups = new Map<number, number[]>();
  for (const time of [...times].sort()) {
    const [h, m] = time.split(":").map(Number);
    groups.set(m, [...(groups.get(m) ?? []), h]);
  }
  return [...groups].map(
    ([minute, hours]) => `${minute} ${hours.join(",")} * * *`,
  );
}
