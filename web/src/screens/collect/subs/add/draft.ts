import type { Channel } from "../../channels/api";
import type { ScheduleEntry, SubtitleMode, TitleGroup } from "../api";

/** What the subscribe flow has collected so far. */
export interface Draft {
  /** The schedule tab being browsed, 0 (Sunday) to 8 (`신작`). */
  week: number;
  anime: ScheduleEntry | null;
  channel: Channel | null;
  work: TitleGroup | null;
  subtitles: SubtitleMode;
  /** The creator followed; kept as chosen while `subtitles` is not `follow`. */
  creator: string | null;
  directory: string;
}

/** Why a save folder cannot be used, or `null` when it can. The server checks it again. */
export function folderProblem(directory: string): string | null {
  const dir = directory.trim();
  if (dir === "") return "저장 폴더를 적어 주세요.";
  if (dir.startsWith("/") || /^[A-Za-z]:/.test(dir)) return "수집 폴더 아래의 상대 경로로 적어 주세요.";
  const parts = dir.split(/[\\/]/);
  if (parts.includes("..")) return "경로에 ‘..’은 쓸 수 없어요.";
  // `.` and `./` are the collect folder itself, not a folder below it.
  if (parts.every((part) => part === "." || part === "")) return "‘.’만 적으면 수집 폴더 자체에 받아요. 작품 폴더 이름을 적어 주세요.";
  return null;
}

/** A shared class for the options of a step: a whole-row button with a selected state. */
export const optionClass =
  "flex w-full min-w-0 flex-col gap-1 rounded-[10px] border bg-surface-2 px-3.5 py-3 text-left outline-offset-2 hover:border-text-muted focus-visible:outline-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:border-hairline-soft";
