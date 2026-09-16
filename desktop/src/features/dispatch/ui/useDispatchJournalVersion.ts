import * as React from "react";
import { DISPATCH_JOURNAL_CHANGED } from "../journalStorage";
export function useDispatchJournalVersion() {
  const [version, setVersion] = React.useState(0);
  React.useEffect(() => {
    const update = () => setVersion((value) => value + 1);
    window.addEventListener(DISPATCH_JOURNAL_CHANGED, update);
    window.addEventListener("storage", update);
    return () => {
      window.removeEventListener(DISPATCH_JOURNAL_CHANGED, update);
      window.removeEventListener("storage", update);
    };
  }, []);
  return version;
}
