import { api } from "@/lib/api";

/**
 * The status board's data (`src/web/status_api.rs`). The server never reads the
 * feeds or asks Transmission for it: channel reads and Transmission's counts
 * are snapshots the worker left, each with the time it was taken.
 */
export interface ChannelStatus {
  id: string;
  name: string | null;
  host: string;
  /** Whether the worker's last read of the feed worked; `null` if it has not tried yet. */
  ok: boolean | null;
  read_at: number | null;
  ok_at: number | null;
}

export interface Day {
  /** `YYYY-MM-DD` in the viewer's time zone. */
  date: string;
  count: number;
}

export interface Board {
  now: number;
  channels: ChannelStatus[];
  /** The last seven days, oldest first, today last. */
  received: { total: number; days: Day[] };
  /** Items that ended as failed or duplicate in those seven days. */
  problems: number;
  transmission: { downloading: number; seeding: number; taken_at: number } | null;
  /**
   * `next_at` is when the next cycle is due; `null` until a worker has recorded its interval.
   * `stalled` is true once it is more than one interval overdue: the worker is not checking.
   */
  cycle: { started_at: number; finished_at: number | null; next_at: number | null; stalled: boolean } | null;
  /** False until the collect folder is chosen; the worker adds no torrent before that. */
  collect_folder_set: boolean;
}

export function loadBoard(signal?: AbortSignal): Promise<Board> {
  // Minutes east of UTC decide where a day starts for the bars.
  const offset = -new Date().getTimezoneOffset();
  return api<Board>(`/collect/status?tz_offset=${offset}`, { signal });
}

/** What the status says in place of a past next-check time when the worker has stopped checking. */
export const STALLED = "RSS 확인이 멈췄어요. worker가 돌고 있는지 확인해 주세요.";
/** The same on a phone's one-line summary. */
export const STALLED_SHORT = "RSS 멈춤";
