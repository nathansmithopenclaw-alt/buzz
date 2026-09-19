/** The population probe asks for one event per workspace in one multi-filter REQ.
 * The discovery fixture has exactly one event per populated workspace. */
export function dispatchPopulationProbeFilter<
  T extends { kinds?: number[]; limit?: number; "#h"?: string[] },
>(filters: T[]): T | null {
  if (
    filters.length < 2 ||
    filters.some(
      (filter) =>
        filter.limit !== 1 ||
        filter["#h"]?.length !== 1 ||
        !filter.kinds?.includes(40020) ||
        JSON.stringify(filter.kinds) !== JSON.stringify(filters[0].kinds),
    )
  )
    return null;
  return {
    ...filters[0],
    "#h": filters.flatMap((filter) => filter["#h"] ?? []),
    limit: filters.length,
  };
}
