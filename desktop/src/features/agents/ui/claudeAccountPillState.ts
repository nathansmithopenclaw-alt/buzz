export function claudeAccountPillShowsWarning(
  error: string | null,
  accountShadowed: boolean,
): boolean {
  return error !== null || accountShadowed;
}
