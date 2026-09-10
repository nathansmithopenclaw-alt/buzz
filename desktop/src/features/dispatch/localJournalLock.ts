const serial = new Map<string, Promise<void>>();
/** Serialize each durable journal independently; nested writes to other journals remain possible. */
export function withLocalJournalLock<T>(
  key: string,
  run: () => Promise<T>,
): Promise<T> {
  if (typeof navigator !== "undefined" && navigator.locks)
    return navigator.locks.request(key, run);
  const previous = serial.get(key) ?? Promise.resolve();
  const result = previous.then(run, run);
  const settled = result.then(
    () => {},
    () => {},
  );
  serial.set(key, settled);
  void settled.then(() => {
    if (serial.get(key) === settled) serial.delete(key);
  });
  return result;
}
