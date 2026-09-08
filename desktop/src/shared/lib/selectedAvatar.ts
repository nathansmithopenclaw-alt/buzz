/** Null is unset; an empty saved value is an explicit removal and stops fallback. */
export function resolveSelectedAvatar(
  ...sources: Array<string | null | undefined>
): string | null {
  for (const source of sources) {
    if (source != null) return source.trim() || null;
  }
  return null;
}
