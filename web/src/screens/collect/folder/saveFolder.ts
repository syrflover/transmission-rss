import type { Anime } from "../../../lib/api-types/Anime";

/**
 * A save folder taken as a work and a season, `<work>/Season NN`
 * (`docs/specs/collection.md`, 방영작 구독). The subscribe, title and rule
 * screens share the two fields; the rule screens can also take a path whole.
 */

/** A save folder read as its work folder and its season. */
export interface WorkSeason {
  work: string;
  season: number;
}

/** The highest season the season field takes. */
export const MAX_SEASON = 99;

/**
 * `<work>/Season NN` read back into its two fields: only the exact form the
 * fields write, so a folder in another form (`Season 1`, a deeper path, the
 * collect folder itself) keeps its own text and opens as a path.
 */
export function parseDirectory(directory: string): WorkSeason | null {
  const found = /^([^/\\]+)\/Season (\d{2})$/.exec(directory);
  if (found === null) return null;
  const work = found[1];
  if (work !== work.trim() || work === "." || work === "..") return null;
  return { work, season: Number(found[2]) };
}

/** The folder the two fields make. */
export function joinDirectory(work: string, season: number): string {
  return `${work.trim()}/Season ${String(season).padStart(2, "0")}`;
}

/** The season typed, or `null` when it is no whole number from 0 to {@link MAX_SEASON}. */
export function parseSeason(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,2}$/.test(trimmed)) return null;
  const season = Number(trimmed);
  return season <= MAX_SEASON ? season : null;
}

/** Why the work field cannot be saved, or `null` when it can. The server checks the folder again. */
export function workProblem(work: string): string | null {
  const trimmed = work.trim();
  if (trimmed === "") return "작품 폴더 이름을 적어 주세요.";
  if (/[\\/]/.test(trimmed)) return "작품 칸에는 폴더 이름 하나만 적어 주세요. ‘/’는 쓸 수 없어요.";
  if (trimmed === "." || trimmed === "..") return "‘.’이나 ‘..’은 작품 폴더 이름으로 쓸 수 없어요.";
  return null;
}

export const SEASON_PROBLEM = `시즌은 0에서 ${MAX_SEASON} 사이의 정수로 적어 주세요.`;

/** The folder the fields make, or why they make none yet. */
export function folderOf(work: string, seasonText: string): { directory: string; problem: string | null } {
  const problem = workProblem(work);
  if (problem !== null) return { directory: "", problem };
  const season = parseSeason(seasonText);
  if (season === null) return { directory: "", problem: SEASON_PROBLEM };
  return { directory: joinDirectory(work, season), problem: null };
}

/** How typed text finds a work: width, case and runs of white space do not count. */
export function searchKey(text: string): string {
  return text.normalize("NFKC").toLowerCase().replace(/\s+/g, " ").trim();
}

/** The works whose folder name or title has the typed text in it, at most `limit`. */
export function matchingWorks<T extends { name: string; title: string }>(works: readonly T[], typed: string, limit = 8): T[] {
  const key = searchKey(typed);
  if (key === "") return [];
  return works.filter((w) => searchKey(w.name).includes(key) || searchKey(w.title).includes(key)).slice(0, limit);
}

/** What a suggestion is asked from: a release's work, or a work folder picked or typed. */
export type Asked = { release: string } | { work: string };

/** The anime a folder is for, as far as the suggestion reads it (`start_date` places the airing; without it, today). */
export type AnimeOfFolder = Pick<Anime, "anime_no" | "start_date">;

export function suggestionPath(asked: Asked, anime: AnimeOfFolder | null): string {
  const params = new URLSearchParams();
  if ("release" in asked) params.set("release", asked.release);
  else params.set("work", asked.work);
  if (anime !== null) {
    params.set("anime_no", String(anime.anime_no));
    if (anime.start_date !== null) params.set("start", anime.start_date);
  }
  return `/save-folder/suggestion?${params}`;
}
