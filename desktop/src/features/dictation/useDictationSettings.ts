import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  type DictationSettings,
  getDictationSettings,
  saveDictationSettings,
} from "./api";

export const DICTATION_SETTINGS_QUERY_KEY = ["dictation-settings"] as const;

/** Dictation settings, shared by the hotkey, composers and the settings card. */
export function useDictationSettings(enabled = true) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: DICTATION_SETTINGS_QUERY_KEY,
    queryFn: getDictationSettings,
    enabled,
    staleTime: 60_000,
  });
  const save = React.useCallback(
    async (settings: DictationSettings) => {
      const saved = await saveDictationSettings(settings);
      queryClient.setQueryData(DICTATION_SETTINGS_QUERY_KEY, saved);
      return saved;
    },
    [queryClient],
  );
  return { settings: query.data ?? null, isLoading: query.isLoading, save };
}
