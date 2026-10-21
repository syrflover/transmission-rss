import { useEffect, useState } from "react";

import { CommandWatch } from "./watchCommands";
import { sharedEvents } from "./sharedEvents";

/**
 * A {@link CommandWatch} for the screen that calls this: the same one on every
 * render, following nothing more once the screen is gone.
 */
export function useCommandWatch(): CommandWatch {
  const [watch] = useState(() => new CommandWatch(sharedEvents()));
  useEffect(() => () => watch.stop(), [watch]);
  return watch;
}
