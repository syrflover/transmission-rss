import { api, ApiError } from "@/lib/api";
import { forget, forgetPrefix } from "@/lib/cached";
import type {
  AnilistPage,
  AnilistSearchBody,
  AnissiaLink as SeasonAnissiaLink,
  AnissiaLinkBody,
  AnissiaPage,
  AnissiaSearchBody,
  ApplyBody,
  ApplyMode,
  Applying,
  ArtworkState,
  CandidateMapping,
  Cleaning,
  CleanBody,
  FileCreatorBody,
  FileCreatorState,
  FormatOrder,
  LibraryWorkPage,
  MappingInput,
  MappingPreview,
  MovieLink,
  MovieSubtitleCandidates,
  NamedCreators,
  NameCreatorBody,
  PickArtworkBody,
  RevertMappingBody,
  SaveMappingBody,
  SeasonInfo,
  SeasonLinksBody,
  SeasonSearchPage,
  SortKey,
  FilterKey,
  SourceMapping,
  SubtitleCandidates,
  SubtitleFormat,
  SubtitleOrderBody,
  VersionBody,
  WorkDetail,
} from "@/lib/api-types";

import { forgetWeek } from "../schedule/api";
import type { AnissiaTarget } from "./detail/movie.ts";

/*
 * The shapes of these APIs are generated from the Rust types (`lib/api-types`, `cargo types`); the types below are the
 * ones the screens import from here. A type of this file that is not re-exported is a shape of the screen's own.
 */
export type {
  AnilistCandidate,
  AnilistPage,
  AnissiaAnime,
  AnissiaCandidate,
  AnissiaHolder,
  AnissiaPage,
  ApplyMode,
  Applying,
  ArtworkImage,
  ArtworkMode,
  ArtworkState,
  CandidateJob,
  CandidateMapping,
  CandidateRevision,
  Cleaning,
  EpisodeRange,
  EpisodeRevision,
  FailureFile,
  FileCreator,
  FileCreatorState,
  FilterKey,
  FormatOrder,
  FuzzyDate,
  ImageStatus,
  LibraryWork,
  LibraryWorkPage,
  MappingChoice,
  MappingException,
  MappingInput,
  MappingPreview,
  MappingRow,
  MovieLink,
  NamedCreators,
  ReferenceTitle,
  RevisionFailure,
  SeasonCandidate,
  SeasonEntry,
  SeasonInfo,
  SeasonSearchPage,
  SeasonSuggestion,
  SortKey,
  SourceMapping,
  StoredSubtitle,
  SubtitleCandidate,
  SubtitleCopy,
  SubtitleCoverage,
  SubtitleCreatorCopies,
  SubtitleFormat,
  UnrecognizedFile,
  WorkDetail,
  WorkEpisode,
  WorkFile,
  WorkRule,
  WorkSeason,
  WorkSubscription,
  WorkSubtitle,
  WorkSubtitles,
} from "@/lib/api-types";

export type {
  AssetKind,
  CleanAsset,
  CleanKind,
  CleanableEntry,
  CleaningEntry,
  KeptAsset,
  StorageKind,
  StorageOverview,
  StorageWork,
  WorkStorage,
} from "./storage.ts";

/**
 * The library list (`src/web/library_api.rs`): the works with the summary the
 * list shows, one page at a time. Times are Unix milliseconds, `null` when unknown.
 */

export interface WorkPageQuery {
  sort: SortKey;
  filter: FilterKey;
  /** The title text to search for; empty for none. */
  q: string;
  /** The `next` of the page before, none for the first page. */
  after?: string | null;
  /** How many works a page has; the server's default (60) when left out. */
  limit?: number;
}

export function loadWorkPage(query: WorkPageQuery, signal?: AbortSignal): Promise<LibraryWorkPage> {
  const params = new URLSearchParams({ sort: query.sort, filter: query.filter });
  if (query.q !== "") params.set("q", query.q);
  if (query.after) params.set("after", query.after);
  if (query.limit !== undefined) params.set("limit", String(query.limit));
  return api<LibraryWorkPage>(`/library/works?${params}`, { signal });
}

/** The cache keys that start with this hold the loaded pages of the list. */
export const LIST_PREFIX = "library:list:";
/** ...and these one work's page. */
export const WORK_PREFIX = "library:work:";

/** The cache key of the settings' stored size per work (`settings/storage`). */
export const STORAGE_KEY = "library:storage";

/**
 * Whoever changes which folders the library reads (adds, rescans or removes a
 * watch folder, sets the collect or archive folder) calls this: the loaded
 * pages of the list and every work's page are read again the next time.
 */
export function forgetLibrary(): void {
  forgetPrefix(LIST_PREFIX);
  forgetPrefix(WORK_PREFIX);
  forget(STORAGE_KEY);
  // The home screen's checklist ends with the first watch folder, and its cards link to works.
  forgetWeek();
}

/** Where a work opens (the work detail screen). */
export const workPath = (id: string) => `/library/${encodeURIComponent(id)}`;

// --- one work (`src/web/library_work_api.rs`) ---------------------------------------

/**
 * Asks to apply the stored subtitle `storedId` beside the episode's video; answers the job. `compare` is `true` when
 * the episode has a subtitle: the job compares and waits for the person's `교체 승인`, so the job's page is the next
 * step. A `409` is a refusal whose message is the sentence to show, a `404` says the copy is gone.
 */
export function applyStored(
  id: string,
  storedId: string,
  mode: ApplyMode = "apply",
): Promise<Applying> {
  const body: ApplyBody = { mode };
  return api<Applying>(`/library/works/${encodeURIComponent(id)}/stored/${encodeURIComponent(storedId)}/apply`, {
    method: "POST",
    body,
  });
}

const orderPath = (id: string) => `/library/works/${encodeURIComponent(id)}/subtitle-order`;

/** Sets the work's own format order; a `400` has the sentence for a bad one. */
export function putSubtitleOrder(id: string, order: SubtitleFormat[]): Promise<FormatOrder> {
  const body: SubtitleOrderBody = { format_order: order };
  return api<FormatOrder>(orderPath(id), { method: "PUT", body });
}

/** Takes the work's own order away; the answer is the global order. */
export function deleteSubtitleOrder(id: string): Promise<FormatOrder> {
  return api<FormatOrder>(orderPath(id), { method: "DELETE" });
}

/**
 * Asks the worker to delete the stored subtitle `storedId` and `assets`, the ids of the linked files the confirmation
 * listed; answers the cleanup's id. A `409` is a `conflict`: the stored file cannot be cleaned (its message is why),
 * or its files changed since (`current` is the new entry).
 */
export function cleanStored(id: string, storedId: string, assets: string[]): Promise<Cleaning> {
  const body: CleanBody = { assets };
  return api<Cleaning>(`/library/works/${encodeURIComponent(id)}/stored/${encodeURIComponent(storedId)}/clean`, {
    method: "POST",
    body,
  });
}

/** The cache key of one work's page. */
export const workKey = (id: string) => `${WORK_PREFIX}${id}`;

/** One work, or `null` when the library has no work with this ID. */
export async function loadWork(id: string, signal?: AbortSignal): Promise<WorkDetail | null> {
  try {
    return await api<WorkDetail>(`/library/works/${encodeURIComponent(id)}`, { signal });
  } catch (e) {
    if (e instanceof ApiError && e.code === "not_found") return null;
    throw e;
  }
}

// --- a work's cover (`src/web/artwork_api.rs`) ----------------------------------------

const artworkPath = (id: string) => `/library/works/${encodeURIComponent(id)}/artwork`;

export function loadArtwork(id: string, signal?: AbortSignal): Promise<ArtworkState> {
  return api<ArtworkState>(artworkPath(id), { signal });
}

export function searchAnilist(id: string, q: string, page: number, signal?: AbortSignal): Promise<AnilistPage> {
  const body: AnilistSearchBody = { q, page };
  return api<AnilistPage>(`${artworkPath(id)}/search`, { method: "POST", body, signal });
}

export function pickArtwork(id: string, version: number, anilistMediaId: number): Promise<ArtworkState> {
  const body: PickArtworkBody = { version, anilist_media_id: anilistMediaId };
  return api<ArtworkState>(`${artworkPath(id)}/pick`, { method: "POST", body });
}

export function uploadArtwork(id: string, version: number, file: File): Promise<ArtworkState> {
  return api<ArtworkState>(`${artworkPath(id)}/upload?version=${version}`, { method: "POST", body: file });
}

/** `clear`: no cover and no automatic search; `auto`: back to automatic with a new search; `repair`: the chosen AniList entry's image again. */
export function changeArtwork(id: string, version: number, action: "clear" | "auto" | "repair"): Promise<ArtworkState> {
  const body: VersionBody = { version };
  return api<ArtworkState>(`${artworkPath(id)}/${action}`, { method: "POST", body });
}

/** The cover URL the list and the detail show for a state. */
export function coverUrlOf(state: ArtworkState): string | null {
  return state.image?.url ?? null;
}

// --- a season's info (`src/web/seasons_api.rs`) ---------------------------------------

const seasonPath = (id: string, season: number) => `/library/works/${encodeURIComponent(id)}/seasons/${season}`;

export function loadSeasonInfo(id: string, season: number, signal?: AbortSignal): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/info`, { signal });
}

export function searchSeason(id: string, season: number, q: string, page: number, signal?: AbortSignal): Promise<SeasonSearchPage> {
  const body: AnilistSearchBody = { q, page };
  return api<SeasonSearchPage>(`${seasonPath(id, season)}/search`, { method: "POST", body, signal });
}

/** Makes `anilistIds` the season's entries, in this order (empty unlinks). */
export function setSeasonLinks(id: string, season: number, version: number, anilistIds: number[]): Promise<SeasonInfo> {
  const body: SeasonLinksBody = { version, anilist_ids: anilistIds };
  return api<SeasonInfo>(`${seasonPath(id, season)}/links`, { method: "POST", body });
}

/** Unlinks the first season and asks for a new automatic search. */
export function restartSeasonAuto(id: string, season: number, version: number): Promise<SeasonInfo> {
  const body: VersionBody = { version };
  return api<SeasonInfo>(`${seasonPath(id, season)}/auto`, { method: "POST", body });
}

/** Asks AniList again for the season's entries, finished ones too. */
export function refreshSeason(id: string, season: number): Promise<SeasonInfo> {
  return api<SeasonInfo>(`${seasonPath(id, season)}/refresh`, { method: "POST", body: {} });
}

// --- a season's Anissia link (`src/web/seasons_anissia_api.rs`) -----------------------

/**
 * A link as the screen holds it: a season's (`AnissiaLink` of the API), or a movie's, which names its `movie` (the
 * movie's episode number, an episode of season 0; `season` is 0, and no subscription holds a movie).
 */
export type AnissiaLink = SeasonAnissiaLink & { movie?: number };

/** Where a picked anime came from: the server checks it against that very list. */
export type AnissiaSource = { week: number } | { q: string; page: number };

/** A link answer as the screen holds it: a season's as it is, a movie's as a link of season 0 that names its `movie`. */
export function anissiaLinkOf(answer: SeasonAnissiaLink | MovieLink): AnissiaLink {
  if (!("episode" in answer)) return answer;
  return {
    season: 0,
    movie: answer.episode,
    version: answer.version,
    anime: answer.anime,
    subscription: null,
    reference_titles: answer.reference_titles,
  };
}

/** `…/seasons/{season}/anissia`, or `…/movies/{episode}/anissia` for a movie. */
const anissiaPath = (id: string, target: AnissiaTarget) =>
  target.kind === "season"
    ? `${seasonPath(id, target.season)}/anissia`
    : `/library/works/${encodeURIComponent(id)}/movies/${target.episode}/anissia`;

/** A target as a cache key part: `3` for a season, `m1` for a movie. */
const targetKey = (target: AnissiaTarget) => (target.kind === "season" ? String(target.season) : `m${target.episode}`);

/** The movie's link, read now. */
export async function loadMovieLink(id: string, episode: number, signal?: AbortSignal): Promise<AnissiaLink> {
  return anissiaLinkOf(await api<MovieLink>(anissiaPath(id, { kind: "movie", episode }), { signal }));
}

/** The cache key of a movie's link. */
export const movieLinkKey = (id: string, episode: number) => `library:movie-link:${id}:${episode}`;

/** Searches Anissia's full list (finished anime included) for the user's `q`, which is required. */
export function searchAnissia(id: string, target: AnissiaTarget, q: string, page: number, signal?: AbortSignal): Promise<AnissiaPage> {
  const body: AnissiaSearchBody = { q, page };
  return api<AnissiaPage>(`${anissiaPath(id, target)}/search`, { method: "POST", body, signal });
}

/** Links the season or the movie to `animeNo` (`null` cuts the link). */
export async function setAnissiaLink(
  id: string,
  target: AnissiaTarget,
  version: number,
  animeNo: number | null,
  source?: AnissiaSource,
): Promise<AnissiaLink> {
  const body: AnissiaLinkBody = { version, anime_no: animeNo, ...source };
  return anissiaLinkOf(await api<SeasonAnissiaLink | MovieLink>(`${anissiaPath(id, target)}/link`, { method: "POST", body }));
}

/** The link a `409` of a link change carries (`current`), whichever target it was for. */
export function currentLink(current: unknown): AnissiaLink {
  return anissiaLinkOf(current as SeasonAnissiaLink | MovieLink);
}

// --- a season's subtitle candidates (`src/web/seasons_anissia_api.rs`) -----------------

/**
 * A season's candidates as the screen holds them. A movie's (`MovieSubtitleCandidates`) is held as a season 0's that
 * names its `movie` (the movie's episode, an episode of season 0), with no `mappings` and no episode counts.
 */
export type CandidateList = SubtitleCandidates & { movie?: number };

const mappingPath = (id: string, season: number, sourceId: string) =>
  `${anissiaPath(id, { kind: "season", season })}/sources/${encodeURIComponent(sourceId)}/mapping`;

/** Saves the user's mapping of a source: the default offset and the exceptions as one unit, from `version`. */
export function saveMapping(
  id: string,
  season: number,
  sourceId: string,
  body: SaveMappingBody,
): Promise<CandidateMapping> {
  return api<CandidateMapping>(mappingPath(id, season, sourceId), { method: "PUT", body });
}

/** What the dialog would show for `input`, without saving anything. */
export function previewMapping(
  id: string,
  season: number,
  sourceId: string,
  input: MappingInput,
  signal?: AbortSignal,
): Promise<MappingPreview> {
  return api<MappingPreview>(`${mappingPath(id, season, sourceId)}/preview`, { method: "POST", body: input, signal });
}

/** `자동으로 되돌리기`: drops the user's mapping so the app decides again. */
export function revertMapping(id: string, season: number, sourceId: string, version: number): Promise<SourceMapping> {
  const body: RevertMappingBody = { version };
  return api<SourceMapping>(`${mappingPath(id, season, sourceId)}/revert`, { method: "POST", body });
}

/** The cache key of one season's (or movie's) candidates (the anime is in it, so a new link never shows the old one's list). */
export const candidatesKey = (id: string, target: AnissiaTarget, animeNo: number | null) =>
  `library:candidates:${id}:${targetKey(target)}:${animeNo ?? "none"}`;

export async function loadCandidates(id: string, target: AnissiaTarget, signal?: AbortSignal): Promise<CandidateList> {
  if (target.kind === "season") return api<CandidateList>(`${anissiaPath(id, target)}/candidates`, { signal });
  const { episode, ...list } = await api<MovieSubtitleCandidates>(`${anissiaPath(id, target)}/candidates`, { signal });
  return { ...list, season: 0, movie: episode, mappings: [], previous_episodes: null, season_episodes: null };
}

/** The command `새로고침` sends: reads the anime's subtitle lines now. */
export const ANISSIA_CAPTIONS_KIND = "anissia_captions";

// --- the creator of a season's subtitle files (`src/web/subtitle_creator_api.rs`) -----

/** Names the creator `creator` (one of the season's Anissia anime) for every subtitle file of the season that has none. */
export function nameUnknownCreators(id: string, season: number, creator: string): Promise<NamedCreators> {
  const body: NameCreatorBody = { creator };
  return api<NamedCreators>(`${seasonPath(id, season)}/subtitle-creators`, { method: "POST", body });
}

/** Changes one subtitle file's creator from `version` (`null` creator: back to `제작자 알 수 없음`). */
export function setFileCreator(
  id: string,
  season: number,
  file: { path: string; version: number },
  creator: string | null,
): Promise<FileCreatorState> {
  const body: FileCreatorBody = { path: file.path, version: file.version, creator };
  return api<FileCreatorState>(`${seasonPath(id, season)}/subtitle-creators/file`, { method: "PUT", body });
}
