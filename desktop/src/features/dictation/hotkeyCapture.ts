/** Shared ownership between the settings recorder and the runtime shortcut. */
export const KEY_CAPTURE_EVENT = "buzz:dictation-key-capture";
let captureOwners = 0;
export const isCapturingDictationKey = () => captureOwners > 0;
export function claimDictationKey() {
  captureOwners += 1;
  window.dispatchEvent(new Event(KEY_CAPTURE_EVENT));
  return () => {
    captureOwners = Math.max(0, captureOwners - 1);
  };
}

type KeyEvent = Pick<
  KeyboardEvent,
  "code" | "key" | "ctrlKey" | "altKey" | "getModifierState"
>;

/** macOS Option is never AltGr, including WebKit's modifier reporting. */
export function isAltGraphEvent(event: KeyEvent, isMac: boolean): boolean {
  return (
    !isMac &&
    (event.key === "AltGraph" ||
      event.getModifierState("AltGraph") ||
      (event.ctrlKey && event.altKey))
  );
}

/** Keep typing and the app/OS editing, navigation and menu keys available. */
export function isAvailableDictationKey(code: string): boolean {
  return (
    ["AltLeft", "AltRight", "F8", "F9"].includes(code) ||
    /^F(1[3-9]|2[0-4])$/.test(code)
  );
}
