import * as React from "react";

import { observeElementBlockSize } from "@/shared/layout/observeElementBlockSize";

export function shouldUseComposerPaddingFallback({
  nextPadding,
  previousPadding,
}: {
  nextPadding: number;
  previousPadding: number | null;
}): boolean {
  return previousPadding === null || nextPadding > previousPadding;
}

/**
 * Observes the height of the composer overlay and sets the scroll
 * container's `paddingBottom` to match, so content is never hidden
 * behind the absolutely-positioned composer.
 *
 * If the reader is already near the bottom, the legacy fallback follows only
 * initial measurement and growth. Consumers with semantic bottom ownership may
 * provide `settleAtBottom` to handle both growth and shrink safely.
 */
export function useComposerHeightPadding(
  scrollContainerRef: React.RefObject<HTMLElement | null>,
  composerRef: React.RefObject<HTMLElement | null>,
  resetKey?: unknown,
  mode: "padding" | "css-variable" = "padding",
  settleAtBottom?: () => boolean,
) {
  const settleAtBottomRef = React.useRef(settleAtBottom);
  settleAtBottomRef.current = settleAtBottom;

  React.useEffect(() => {
    void resetKey;
    const scrollEl = scrollContainerRef.current;
    const composerEl = composerRef.current;

    if (!scrollEl || !composerEl) {
      return;
    }

    // Floating timeline controls are siblings of the scroll element. Set the
    // measurement on their shared parent in both modes so they inherit the
    // overlay's live height. CSS-variable mode also uses it for the virtual
    // trailing spacer; padding mode still applies padding directly below.
    const cssVariableTarget = scrollEl.parentElement ?? scrollEl;

    const getScrollElement = (): HTMLElement =>
      mode === "css-variable"
        ? (scrollEl.querySelector<HTMLElement>(
            '[data-testid="message-timeline"]',
          ) ?? scrollEl)
        : scrollEl;

    let lastPadding: number | null = null;
    let followBottomFrame: number | null = null;

    const isNearBottom = (): boolean => {
      const target = getScrollElement();
      const threshold = 32;
      const trailingClearance =
        mode === "css-variable" ? (lastPadding ?? 0) : 0;
      return (
        target.scrollHeight -
          target.scrollTop -
          target.clientHeight -
          trailingClearance <
        threshold
      );
    };

    const followBottom = () => {
      const target = getScrollElement();
      target.scrollTop = target.scrollHeight;
    };

    const applyPadding = (height: number) => {
      const padding = Math.ceil(height);
      if (lastPadding !== null && Math.abs(padding - lastPadding) <= 1) {
        return;
      }

      const previousPadding = lastPadding;
      const wasAtBottom = isNearBottom();

      cssVariableTarget.style.setProperty(
        "--composer-overlay-height",
        `${padding}px`,
      );
      if (mode === "padding") {
        scrollEl.style.paddingBottom = `${padding}px`;
      }
      lastPadding = padding;

      if (wasAtBottom && previousPadding !== padding) {
        if (settleAtBottomRef.current?.()) {
          return;
        }
        if (
          !shouldUseComposerPaddingFallback({
            nextPadding: padding,
            previousPadding,
          })
        ) {
          return;
        }
        followBottom();
        if (followBottomFrame !== null) {
          cancelAnimationFrame(followBottomFrame);
        }
        followBottomFrame = requestAnimationFrame(() => {
          followBottomFrame = null;
          followBottom();
        });
      }
    };

    const disconnect = observeElementBlockSize(composerEl, applyPadding);

    return () => {
      disconnect();
      if (followBottomFrame !== null) {
        cancelAnimationFrame(followBottomFrame);
      }
      cssVariableTarget.style.removeProperty("--composer-overlay-height");
      if (mode === "padding") {
        scrollEl.style.paddingBottom = "";
      }
    };
  }, [scrollContainerRef, composerRef, mode, resetKey]);
}
