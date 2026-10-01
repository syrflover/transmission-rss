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
  /** `next_at` is when the next cycle is due; `null` until a worker has recorded its interval. */
  cycle: { started_at: number; finished_at: number | null; next_at: number | null } | null;
  /** False until the collect folder is chosen; the worker adds no torrent before that. */
  collect_folder_set: boolean;
}

export function loadBoard(signal?: AbortSignal): Promise<Board> {
  // Minutes east of UTC decide where a day starts for the bars.
  const offset = -new Date().getTimezoneOffset();
  return api<Board>(`/collect/status?tz_offset=${offset}`, { signal });
}
