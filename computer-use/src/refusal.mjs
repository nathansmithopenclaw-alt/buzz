/** Only host pre-dispatch guards may establish that no input was sent. */
export class InputRefused extends Error {}

/** The native driver has typed refusal envelopes and one exact unsupported AX error. */
export function nativeRefused(result) {
  if (!result.isError) return false;
  const values = [result.structuredContent];
  for (const block of result.content ?? []) {
    if (block.type !== "text") continue;
    if (
      block.text ===
      "AX action failed: AXUIElementPerformAction(AXPress) returned -25206"
    )
      values.push({ effect: "refused", code: "unsupported_axpress" });
    try {
      values.push(JSON.parse(block.text));
    } catch {
      /* Plain diagnostic text is not evidence. */
    }
  }
  if (values.some((value) => value?.outcome_uncertain === true)) return false;
  return values.some(
    (value) => value?.effect === "refused" && typeof value.code === "string",
  );
}
