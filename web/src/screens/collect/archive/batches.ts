import { sharedEvents } from "@/lib/sharedEvents";
import { CommandWatch } from "@/lib/watchCommands";

import { ruleArchived, suggestionGone } from "../cache";
import { getBatch } from "./api";
import { BatchTracker } from "./batchFollow";

let shared: BatchTracker | null = null;

/**
 * The tab's one {@link BatchTracker}. It lives as long as the page, not as
 * long as the 구독 tab: the server goes on with a batch the user started even
 * when the tab is left, and the cached lists must still learn what it archived.
 */
export function batches(): BatchTracker {
  shared ??= new BatchTracker({
    watching: new CommandWatch(sharedEvents()),
    read: getBatch,
    gone: suggestionGone,
    archived: ruleArchived,
  });
  return shared;
}
