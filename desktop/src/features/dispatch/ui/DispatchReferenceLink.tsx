import { parseDispatchLink } from "../dispatchLink";
import { Markdown } from "@/shared/ui/markdown";
import type { DispatchExternalRef } from "../types";
import { parseMessageLink } from "@/features/messages/lib/messageLink";
import { parseEntityLink } from "@/shared/lib/entityLink";
export function isDispatchResourceUrl(value: string) {
  try {
    const url = new URL(value);
    return (
      ["http:", "https:"].includes(url.protocol) ||
      parseMessageLink(value).ok ||
      parseEntityLink(value).ok ||
      Boolean(parseDispatchLink(value))
    );
  } catch {
    return false;
  }
}
export function DispatchReferenceLink({
  value,
}: {
  value: DispatchExternalRef;
}) {
  if (!isDispatchResourceUrl(value.ref))
    return (
      <span className="break-all text-xs">
        {value.type}: {value.ref}
      </span>
    );
  return (
    <Markdown
      content={`<${value.ref.replaceAll(">", "%3E").replaceAll("<", "%3C").replaceAll("\n", "")}>`}
      className="text-sm"
      linkPreviewsSuppressed
    />
  );
}
