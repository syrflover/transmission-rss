import { sizeText } from "../../lib/size.ts";
import { carriesTime, receivedAt } from "./received.ts";

/**
 * The stored files of a work and what can be cleaned (`GET /api/library/works/{id}` `storage`, `GET
 * /api/library/storage`, `POST .../stored/{id}/clean`), and the decisions the screens take from them: the labels, the
 * texts of sizes and dates, what the confirmation lists and sends, and how a refused clean is read. It has no `@/`
 * imports so it runs under `node --test`; `api.ts` re-exports the types.
 */

/** Why a stored file can be cleaned: `past` 지난 수정본, `awaiting_video` 영상 대기, `stored` 보관만 함, `unplaced` 회차에 붙지 않음. */
export type CleanKind = "past" | "awaiting_video" | "stored" | "unplaced";

export type AssetKind = "subtitle" | "font" | "attachment" | "companion";

/** A file a cleanup deletes now. */
export interface CleanAsset {
  id: string;
  name: string;
  kind: AssetKind;
  size: number;
}

/** A file linked to the stored copy that stays, and why. */
export interface KeptAsset {
  id: string;
  name: string;
  kind: AssetKind;
  reason: string;
}

/** A stored subtitle that can be cleaned (or says why it cannot be now). */
export interface CleanableEntry {
  id: string;
  name: string;
  season: number;
  /** `null` for `unplaced`. */
  episode: number | null;
  /** `null` is `제작자 알 수 없음`. */
  creator: string | null;
  format: "ass" | "srt" | "smi" | "other";
  size: number;
  /** Unix milliseconds. */
  stored_at: number;
  kind: CleanKind;
  /** Why it cannot be cleaned now; the entry then has no `정리`. */
  blocked: string | null;
  /** What the cleanup deletes now. */
  with: CleanAsset[];
  /** The linked files that stay. */
  kept: KeptAsset[];
}

/** A clean that was asked for and that the worker has not finished (`asked`) or could not (`held`). */
export interface CleaningEntry {
  id: string;
  name: string;
  state: "asked" | "held";
  reason: string | null;
}

export interface WorkStorage {
  /** Bytes of every file the app stored for the work. */
  total: number;
  cleanable: CleanableEntry[];
  cleaning: CleaningEntry[];
}

export type StorageKind = "subtitle" | "font" | "attachment" | "cover";

export interface StorageWork {
  id: string;
  name: string;
  total: number;
  kinds: { kind: StorageKind; count: number; size: number }[];
  /** How many stored files can be cleaned. */
  cleanable: number;
}

export interface StorageOverview {
  works: StorageWork[];
}

// --- texts ----------------------------------------------------------------------------------------------------------

export const KIND_LABEL: Record<CleanKind, string> = {
  past: "지난 수정본",
  awaiting_video: "영상 대기",
  stored: "보관만 함",
  unplaced: "회차에 붙지 않음",
};

/** The order the kinds are listed in. */
export const KIND_ORDER: readonly CleanKind[] = ["past", "awaiting_video", "stored", "unplaced"];

export const ASSET_LABEL: Record<AssetKind, string> = {
  subtitle: "자막",
  font: "폰트",
  attachment: "첨부",
  companion: "구성 파일",
};

export const STORAGE_KIND_LABEL: Record<StorageKind, string> = {
  subtitle: "자막",
  font: "폰트",
  attachment: "첨부",
  cover: "표지",
};

const STORAGE_KIND_ORDER: readonly StorageKind[] = ["subtitle", "font", "attachment", "cover"];

export const NO_CREATOR = "제작자 알 수 없음";
export const NO_EPISODE = "회차 없음";

/**
 * What the 파일 card says of the stored size: `보관한 자막·폰트·첨부 1.2 MB`. The cover is not among them, which the
 * settings' size of the work counts.
 */
export function totalText(total: number): string {
  return `보관한 자막·폰트·첨부 ${sizeText(total)}`;
}

/** `2화`, or `시즌 2 3화` when the work has several seasons; `회차 없음` without an episode. */
export function episodeText(entry: Pick<CleanableEntry, "season" | "episode">, seasons: number): string {
  if (entry.episode === null) return NO_EPISODE;
  return seasons > 1 ? `시즌 ${entry.season} ${entry.episode}화` : `${entry.episode}화`;
}

export function creatorText(creator: string | null): string {
  return creator === null || creator === "" ? NO_CREATOR : creator;
}

/** What an entry is a copy of, so two received on one day can be told apart: its episode, or its name when it has none. */
function copyKey(entry: CleanableEntry): string {
  return entry.episode === null ? `name:${entry.name}` : `s${entry.season}e${entry.episode}`;
}

/**
 * `receivedAt` of a stored file of the 파일 card: the date carries its time when another entry of the same episode (any
 * creator and format) was received the same day.
 */
export function receivedText(entry: CleanableEntry, all: readonly CleanableEntry[]): string {
  return receivedAt(entry.stored_at, carriesTime(entry, all, (a, b) => copyKey(a) === copyKey(b)));
}

export interface KindGroup {
  kind: CleanKind;
  label: string;
  entries: CleanableEntry[];
}

/** The entries grouped by kind in `KIND_ORDER`, empty groups left out; inside a group by season, episode, then when stored. */
export function groupByKind(entries: readonly CleanableEntry[]): KindGroup[] {
  const order = (a: CleanableEntry, b: CleanableEntry) =>
    a.season - b.season ||
    (a.episode ?? Number.MAX_SAFE_INTEGER) - (b.episode ?? Number.MAX_SAFE_INTEGER) ||
    a.stored_at - b.stored_at ||
    a.name.localeCompare(b.name);
  return KIND_ORDER.map((kind) => ({ kind, label: KIND_LABEL[kind], entries: entries.filter((e) => e.kind === kind).sort(order) })).filter(
    (group) => group.entries.length > 0,
  );
}

/** Whether the entry can be cleaned now: it is offered `정리` unless the server says why not. */
export function canClean(entry: CleanableEntry): boolean {
  return entry.blocked === null;
}

/** What a `cleaning` entry shows: `정리하는 중`, or why the worker kept the files. */
export function cleaningText(entry: CleaningEntry): string {
  if (entry.state === "asked") return "정리하는 중";
  return entry.reason ?? "정리하지 않고 보류했어요.";
}

/** A clean this page asked for: the cleanup's id (the answer's `cleanup_id`), and the stored file's id and name. */
export interface SentClean {
  id: string;
  entryId: string;
  name: string;
}

/**
 * What the page says of the clean it asked for once the worker carried it out: `null` while its row still says how it
 * goes (`정리하는 중`, or why it was held) or while the page still lists the stored file (it has not read the work
 * since); `… 정리를 마쳤어요.` once neither lists it.
 */
export function sentNotice(sent: SentClean | null, storage: WorkStorage): string | null {
  if (sent === null) return null;
  if (storage.cleaning.some((c) => c.id === sent.id) || storage.cleanable.some((e) => e.id === sent.entryId)) return null;
  return `${sent.name} 정리를 마쳤어요.`;
}

/** Whether the worker still has a clean to carry out, so the page keeps reading the work. */
export function isCleaning(storage: WorkStorage): boolean {
  return storage.cleaning.some((c) => c.state === "asked");
}

/**
 * What the page shows of the storage changing: it reads the work again behind it and the settings' list is read again
 * when this text changes.
 */
export function storageSignature(storage: WorkStorage): string {
  return [
    storage.total,
    ...storage.cleanable.map((e) => `c:${e.id}:${e.blocked ?? ""}`),
    ...storage.cleaning.map((c) => `w:${c.id}:${c.state}`),
  ].join("|");
}

// --- the confirmation -----------------------------------------------------------------------------------------------

/** The line of a file the confirmation names. */
export interface DeleteLine {
  id: string;
  name: string;
  kindLabel: string;
  sizeText: string;
}

export interface KeptLine {
  id: string;
  name: string;
  kindLabel: string;
  reason: string;
}

/** What the confirmation of one clean shows. */
export interface ConfirmView {
  entryId: string;
  /** The stored file's name, which the question names. */
  name: string;
  /** Every file the clean deletes, as the confirmation lists it. */
  deletes: DeleteLine[];
  /** The total of those, `23 KB`. */
  totalText: string;
  /** The linked files that stay, each with why. */
  kept: KeptLine[];
  /**
   * The subtitle file is not among the ones deleted and no kept line says why: another stored copy has the same
   * content, so it stays. The server names it in `kept` with its reason, and then this is `false`.
   */
  subtitleStays: boolean;
  /** `null` when it can be cleaned; else why not (the confirmation then has no `정리`). */
  blocked: string | null;
}

export function confirmView(entry: CleanableEntry): ConfirmView {
  return {
    entryId: entry.id,
    name: entry.name,
    deletes: entry.with.map((a) => ({ id: a.id, name: a.name, kindLabel: ASSET_LABEL[a.kind], sizeText: sizeText(a.size) })),
    totalText: sizeText(entry.with.reduce((sum, a) => sum + a.size, 0)),
    kept: entry.kept.map((a) => ({ id: a.id, name: a.name, kindLabel: ASSET_LABEL[a.kind], reason: a.reason })),
    subtitleStays: ![...entry.with, ...entry.kept].some((a) => a.kind === "subtitle"),
    blocked: entry.blocked,
  };
}

/** The body of the clean request: the ids of the files the confirmation showed, in its order. */
export function cleanBody(entry: CleanableEntry): { assets: string[] } {
  return { assets: entry.with.map((a) => a.id) };
}

/** The server's sentence for a clean whose files changed since the page read them (409 `conflict`). */
export const CHANGED_MESSAGE = "정리할 파일이 바뀌었어요. 다시 확인해 주세요.";

/** What the confirmation says when it shows the new list after the files changed. */
export const CHANGED_NOTICE = "정리할 파일이 바뀌어서 목록을 다시 불러왔어요. 지금 목록이 맞는지 확인해 주세요.";

/** What the confirmation says when the stored file is gone (404). */
export const GONE_NOTICE = "이 보관본은 이미 없어요.";

/** How a refused clean goes on. */
export type CleanFailure =
  /** The files changed: show this entry in the confirmation, with `CHANGED_NOTICE`. */
  | { kind: "changed"; entry: CleanableEntry }
  /** The files changed but the server sent no new list (or one this page cannot read): read the work again. */
  | { kind: "reload"; message: string }
  /** The stored file is gone: read the work again and close the confirmation. */
  | { kind: "gone"; message: string }
  /** The stored file cannot be cleaned now (a 409 whose message is why): read the work again and close the confirmation. */
  | { kind: "refused"; message: string }
  /** Any other error (the server could not be reached, say): show the sentence and keep the confirmation to try again. */
  | { kind: "message"; message: string };

/** The parts of an `ApiError` this reads (`@/lib/api`), so the module does not import it. */
export interface FailureLike {
  code: string;
  message: string;
  current?: unknown;
}

function isAsset(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const a = value as Record<string, unknown>;
  return typeof a.id === "string" && typeof a.name === "string" && typeof a.kind === "string";
}

/** The `current` of a 409 when it is a cleanable entry this page can show. */
export function entryOf(value: unknown): CleanableEntry | null {
  if (typeof value !== "object" || value === null) return null;
  const e = value as Record<string, unknown>;
  const ok =
    typeof e.id === "string" &&
    typeof e.name === "string" &&
    Array.isArray(e.with) &&
    e.with.every(isAsset) &&
    Array.isArray(e.kept) &&
    e.kept.every(isAsset);
  return ok ? (value as CleanableEntry) : null;
}

/** Reads the error of a clean request. A 409 with a new list is `changed`; any other 409 (`message` is the reason) is `refused`. */
export function cleanFailure(error: FailureLike): CleanFailure {
  if (error.code === "not_found") return { kind: "gone", message: GONE_NOTICE };
  if (error.code === "conflict") {
    const entry = entryOf(error.current);
    if (entry) return { kind: "changed", entry };
    return error.message === CHANGED_MESSAGE ? { kind: "reload", message: error.message } : { kind: "refused", message: error.message };
  }
  return { kind: "message", message: error.message };
}

// --- the settings list ----------------------------------------------------------------------------------------------

/** The address of a work's 파일 card; the work page opens the card and scrolls to it. */
export const FILES_HASH = "files";
export const filesPath = (workPath: string) => `${workPath}#${FILES_HASH}`;

/** `자막 12개 · 340 KB`, the kinds in a fixed order, those with no file left out. */
export function kindTexts(kinds: StorageWork["kinds"]): { kind: StorageKind; text: string }[] {
  return STORAGE_KIND_ORDER.flatMap((kind) => {
    const found = kinds.find((k) => k.kind === kind && k.count > 0);
    return found ? [{ kind, text: `${STORAGE_KIND_LABEL[kind]} ${found.count}개 · ${sizeText(found.size)}` }] : [];
  });
}

/** `정리할 파일 3개`, or `null` when there is none. */
export function cleanableText(count: number): string | null {
  return count > 0 ? `정리할 파일 ${count}개` : null;
}

/** What the settings list shows without opening the item: `작품 5개 · 12 MB`, or `null` while no work has a file. */
export function storageSummary(overview: StorageOverview): string | null {
  if (overview.works.length === 0) return null;
  const total = overview.works.reduce((sum, w) => sum + w.total, 0);
  return `작품 ${overview.works.length}개 · ${sizeText(total)}`;
}

/** How many files can be cleaned over every work. */
export function cleanableCount(overview: StorageOverview): number {
  return overview.works.reduce((sum, w) => sum + w.cleanable, 0);
}
