export function parseSummaryLink(
  href: string,
): { id: string; relay: string } | null {
  try {
    const url = new URL(href);
    if (
      url.protocol !== "buzz:" ||
      url.hostname !== "summary" ||
      (url.pathname !== "" && url.pathname !== "/") ||
      url.hash ||
      url.username ||
      url.password ||
      url.port
    )
      return null;
    const keys = [...url.searchParams.keys()];
    if (
      keys.length !== 2 ||
      url.searchParams.getAll("id").length !== 1 ||
      url.searchParams.getAll("relay").length !== 1
    )
      return null;
    const id = url.searchParams.get("id"),
      relay = url.searchParams.get("relay");
    if (!id || !relay) return null;
    const address = new URL(relay);
    if (
      !/^[a-f0-9]{64}$/i.test(id) ||
      !["ws:", "wss:"].includes(address.protocol) ||
      address.username ||
      address.password ||
      address.hash ||
      relay.length > 2048
    )
      return null;
    return { id: id.toLowerCase(), relay };
  } catch {
    return null;
  }
}
export function buildSummaryLink(id: string, relay: string): string {
  const href = `buzz://summary?${new URLSearchParams({ id, relay })}`;
  if (!parseSummaryLink(href)) throw new Error("Invalid Summary link");
  return href;
}
