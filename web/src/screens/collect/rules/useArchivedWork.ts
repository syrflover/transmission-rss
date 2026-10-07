import { useEffect, useState } from "react";

import { getArchivedWork, type ArchivedWork } from "./api";

const DEBOUNCE_MS = 300;

/**
 * The work in the archive folder that a rule saving to `directory` would bring
 * into the collect folder, for the forms to say so before the rule is made or
 * saved. The folder is sent as typed, a moment after the last edit, and the
 * server finds the work folder. A form that edits a stored rule passes the
 * folder the rule has as `from`. The answer is the library's records, not the
 * disk, so it can be a little stale; the server decides again from the disk
 * when the rule is saved, and a failed question just shows nothing.
 */
export function useArchivedWork(directory: string, enabled = true, from: string | null = null): ArchivedWork | null {
  const [found, setFound] = useState<ArchivedWork | null>(null);
  const typed = enabled ? directory.trim() : "";

  useEffect(() => {
    if (typed === "") {
      setFound(null);
      return;
    }
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      getArchivedWork(typed, from, controller.signal).then(
        (archived) => {
          if (!controller.signal.aborted) setFound(archived);
        },
        () => {
          if (!controller.signal.aborted) setFound(null);
        },
      );
    }, DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [typed, from]);

  return typed === "" ? null : found;
}
