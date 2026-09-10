const uuid =
  /^[a-f0-9]{8}-[a-f0-9]{4}-[45][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/i;
export type DispatchLink = { workspace: string; issue: string; relay: string };
export function parseDispatchLink(href: string): DispatchLink | null {
  try {
    const url = new URL(href);
    if (
      url.protocol !== "buzz:" ||
      url.hostname !== "dispatch" ||
      !["", "/"].includes(url.pathname) ||
      url.username ||
      url.password ||
      url.port ||
      url.hash
    )
      return null;
    if (
      [...url.searchParams.keys()].length !== 3 ||
      ["workspace", "issue", "relay"].some(
        (name) => url.searchParams.getAll(name).length !== 1,
      )
    )
      return null;
    const workspace = url.searchParams.get("workspace") ?? "",
      issue = url.searchParams.get("issue") ?? "",
      relay = url.searchParams.get("relay") ?? "";
    const address = new URL(relay);
    if (
      !uuid.test(workspace) ||
      !uuid.test(issue) ||
      relay.length > 2048 ||
      !["ws:", "wss:"].includes(address.protocol) ||
      address.username ||
      address.password ||
      address.hash
    )
      return null;
    return {
      workspace: workspace.toLowerCase(),
      issue: issue.toLowerCase(),
      relay,
    };
  } catch {
    return null;
  }
}
export function buildDispatchLink(input: DispatchLink) {
  const href = `buzz://dispatch?${new URLSearchParams(input)}`;
  if (!parseDispatchLink(href)) throw new Error("Invalid Dispatch issue link");
  return href;
}
