import type { ReactNode } from "react";
import { useOpenSummaryLink } from "../lib/useOpenSummaryLink";
export function SummaryLinkAnchor({
  href,
  children,
}: {
  href: string;
  children: ReactNode;
}) {
  const open = useOpenSummaryLink();
  return (
    <a
      href={href}
      className="underline"
      onClick={(event) => {
        event.preventDefault();
        open(href);
      }}
    >
      {children}
    </a>
  );
}
