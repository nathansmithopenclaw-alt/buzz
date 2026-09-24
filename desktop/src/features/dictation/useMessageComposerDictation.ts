import type { Editor } from "@tiptap/core";
import * as React from "react";

import { useFeatureEnabled } from "@/shared/features";
import { isMacPlatform } from "@/shared/lib/platform";
import type { RegisterMention } from "./lib/renderSegments";
import { useHuddleIsLive } from "./lib/useHuddleIsLive";
import type { DictationButtonProps } from "./ui/DictationButton";
import { hotkeyLabel } from "./ui/DictationButton";
import {
  type DictationCandidates,
  useComposerDictation,
} from "./useComposerDictation";
import { useDictationSettings } from "./useDictationSettings";

export type MessageComposerDictationInput = {
  editor: Editor | null;
  draftKey: string | null;
  channelId: string | null;
  editTargetId: string | null;
  isMain: boolean;
  disabled: boolean;
  getCandidates: () => DictationCandidates;
  registerMention: RegisterMention;
  recentMentionPubkeys?: readonly string[];
};

/**
 * Everything `MessageComposer` needs for dictation behind the "dictation"
 * experiment: the controller, whether sending must wait, and the toolbar
 * button's props (`null` when the experiment is off).
 */
export function useMessageComposerDictation(
  input: MessageComposerDictationInput,
) {
  const enabled = useFeatureEnabled("dictation");
  const huddleActive = useHuddleIsLive();
  const { settings } = useDictationSettings(enabled);
  const dictation = useComposerDictation({
    ...input,
    enabled,
    huddleActive,
  });
  const shortcut = hotkeyLabel(settings?.hotkey ?? "AltRight", isMacPlatform());
  const buttonProps = React.useMemo<Omit<
    DictationButtonProps,
    "onCaptureSelection"
  > | null>(
    () =>
      enabled
        ? {
            status: dictation.status,
            getLevel: dictation.getLevel,
            startedAt: dictation.startedAt,
            onToggle: dictation.toggle,
            disabledReason: huddleActive
              ? "Dictation is off during a huddle"
              : input.disabled
                ? "Composer is disabled"
                : null,
            shortcut,
          }
        : null,
    [
      dictation.getLevel,
      dictation.startedAt,
      dictation.status,
      dictation.toggle,
      enabled,
      huddleActive,
      input.disabled,
      shortcut,
    ],
  );
  return {
    enabled,
    status: dictation.status,
    isBusy: dictation.isBusy,
    notifyBusy: dictation.notifyBusy,
    buttonProps,
  };
}
