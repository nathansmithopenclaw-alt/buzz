import { createContext, useContext } from "react";
import type { DispatchIssue } from "../types";
import { issueAlias } from "../issueEditing";

/** Full-workspace identity scope keeps aliases unambiguous even in a filtered view. */
export const DispatchAliasContext = createContext<readonly DispatchIssue[]>([]);
export function useIssueAlias(issue: DispatchIssue) {
  const issues = useContext(DispatchAliasContext);
  return issues.length ? issueAlias(issue, issues) : issue.identifier;
}
