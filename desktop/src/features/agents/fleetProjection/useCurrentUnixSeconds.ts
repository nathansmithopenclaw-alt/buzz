import * as React from "react";

export function useCurrentUnixSeconds(enabled = true) {
  const [now, setNow] = React.useState(() => Math.floor(Date.now() / 1_000));

  React.useEffect(() => {
    if (!enabled) return;
    const interval = window.setInterval(
      () => setNow(Math.floor(Date.now() / 1_000)),
      1_000,
    );
    return () => window.clearInterval(interval);
  }, [enabled]);

  return now;
}
