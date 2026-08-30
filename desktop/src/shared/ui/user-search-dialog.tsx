import { Bot, UserRoundPlus, X } from "lucide-react";
import * as React from "react";

import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "./dialog";

const MEMBER_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

/**
 * The one "find a person or agent" modal.
 *
 * The channel members popup is the reference design and the Dispatch assignee
 * picker is the second call site. They render *this* component rather than two
 * copies of the same markup, so the next change to the popup cannot land on one
 * surface and silently leave the other behind. Anything genuinely specific to a
 * call site — which sections exist, what selecting a row does — is a child, not
 * a prop on the shell.
 */
export function UserSearchDialog({
  children,
  inputDisabled = false,
  inputId,
  inputTestId,
  notice,
  onOpenChange,
  onSearchChange,
  onSearchKeyDown,
  open,
  searchPlaceholder,
  searchValue,
  testId,
  title,
}: {
  children: React.ReactNode;
  inputDisabled?: boolean;
  inputId: string;
  inputTestId: string;
  notice?: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  onSearchChange: (value: string) => void;
  onSearchKeyDown?: (event: React.KeyboardEvent<HTMLInputElement>) => void;
  open: boolean;
  searchPlaceholder: string;
  searchValue: string;
  testId: string;
  title: React.ReactNode;
}) {
  const searchInputRef = React.useRef<HTMLInputElement>(null);

  // Belt and braces with `onOpenAutoFocus` below: a caller that re-opens the
  // dialog while its content is still mounted gets no autofocus event, and
  // landing on a modal whose only control is a search field with the caret
  // somewhere else is a dead end for keyboard users.
  React.useEffect(() => {
    if (!open) return;
    searchInputRef.current?.focus({ preventScroll: true });
  }, [open]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="max-w-xl gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
        data-testid={testId}
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          searchInputRef.current?.focus({ preventScroll: true });
        }}
        showCloseButton={false}
      >
        <DialogHeader className="space-y-0 pb-5">
          <div className="flex items-center justify-between gap-4">
            <DialogTitle>{title}</DialogTitle>
            <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogClose>
          </div>
          <label className={MODAL_SEARCH_SHELL_CLASS} htmlFor={inputId}>
            <UserRoundPlus className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className={MODAL_SEARCH_INPUT_CLASS}
              data-testid={inputTestId}
              disabled={inputDisabled}
              id={inputId}
              onChange={(event) => {
                onSearchChange(event.target.value);
              }}
              onKeyDown={onSearchKeyDown}
              placeholder={searchPlaceholder}
              ref={searchInputRef}
              spellCheck={false}
              type="text"
              value={searchValue}
            />
          </label>
          {notice}
        </DialogHeader>

        <div className="max-h-[calc(100vh-12rem)] overflow-y-auto pb-6">
          {children}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * The bordered, fixed-height results box inside {@link UserSearchDialog}.
 *
 * The height is fixed rather than content-sized so the modal does not resize
 * under the pointer as results stream in. `onScroll` is where a call site hangs
 * infinite-scroll paging.
 */
export function UserSearchResultsPanel({
  children,
  onScroll,
  testId,
}: {
  children: React.ReactNode;
  onScroll?: (event: React.UIEvent<HTMLDivElement>) => void;
  testId: string;
}) {
  return (
    <section>
      <div
        className="h-[min(50vh,24rem)] overflow-y-auto rounded-xl border border-border/70 bg-background/70"
        data-testid={testId}
        onScroll={onScroll}
      >
        {children}
      </div>
    </section>
  );
}

/** Sticky group heading inside a {@link UserSearchResultsPanel}. */
export function UserSearchSectionTitle({
  action,
  children,
}: {
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="sticky top-0 z-10 mr-3 flex min-h-9 items-center gap-2 bg-background/95 px-4 pb-1.5 pt-3 text-xs font-medium text-muted-foreground/75 backdrop-blur supports-[backdrop-filter]:bg-background/80">
      <span>{children}</span>
      {action ? <span>{action}</span> : null}
    </div>
  );
}

/** The display name a search candidate is listed and ranked under. */
export function formatUserSearchCandidateName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

/**
 * One selectable person or agent.
 *
 * The whole row is a click target (the absolutely positioned button behind the
 * content); the trailing button repeats the action so it is reachable by name
 * and readable as an affordance. `actionLabel` is the only thing that varies
 * between call sites — "Add" to a channel, "Assign" to an issue.
 */
export function UserSearchResultRow({
  actionLabel,
  disabled,
  onSelect,
  ownerLabel,
  testId,
  user,
}: {
  actionLabel: string;
  disabled: boolean;
  onSelect: (user: UserSearchResult) => void;
  ownerLabel?: string | null;
  testId: string;
  user: UserSearchResult;
}) {
  return (
    <div
      className={cn(
        "group/add-result relative isolate flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        MEMBER_ROW_INSET_DIVIDER_CLASS,
      )}
      data-testid={testId}
    >
      <button
        aria-label={`Select ${formatUserSearchCandidateName(user)}`}
        className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        disabled={disabled}
        onClick={() => onSelect(user)}
        type="button"
      />
      <UserAvatar
        avatarUrl={user.avatarUrl}
        className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
        displayName={formatUserSearchCandidateName(user)}
        size="sm"
      />
      <div className="pointer-events-none relative z-10 min-w-0 flex-1">
        {user.isAgent ? (
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium tracking-tight">
                {formatUserSearchCandidateName(user)}
              </span>
              <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                <Bot aria-hidden="true" className="h-4 w-4" />
                agent
              </span>
            </div>
            <span className="block truncate font-mono text-2xs text-muted-foreground">
              {truncatePubkey(user.pubkey)}
            </span>
            {ownerLabel ? (
              <span className="block truncate text-xs text-muted-foreground">
                managed by {ownerLabel}
              </span>
            ) : null}
          </div>
        ) : (
          <span className="block truncate text-sm font-medium tracking-tight">
            {formatUserSearchCandidateName(user)}
          </span>
        )}
      </div>
      <Button
        className="relative z-20 shrink-0"
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          onSelect(user);
        }}
        size="sm"
        type="button"
      >
        {actionLabel}
      </Button>
    </div>
  );
}
