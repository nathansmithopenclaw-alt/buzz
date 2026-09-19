/** Authorization payloads are flat objects with primitive values and ordered agent arrays.
 * Native serde may reorder object keys; that must not change a saved intent. */
export function authorizationIntent(value: object): string {
  return JSON.stringify(value, Object.keys(value).sort());
}
