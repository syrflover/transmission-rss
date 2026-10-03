/**
 * What the `자막 올리기` section works out before anything is sent
 * (`docs/specs/subtitles.md`, 직접 찾기와 자막 올리기): which of the chosen
 * files are subtitles, fonts and ZIPs and which are left out, whether they
 * are within the limits, and whether the browser can pick a folder. No
 * browser API is called here, so it runs under `node --test`.
 *
 * The browser judges by the name only. The server judges again by what is in
 * the file, so an image named `a.ass` is sent and dropped there.
 */

/** The limits the server holds an upload to (`UPLOAD` constants in `trss-jobs/src/upload.rs`). */
export const LIMITS = {
  /** Files sent: subtitles, fonts and ZIPs. */
  files: 500,
  /** Files named in all, those left out included. */
  entries: 2000,
  /** The files' bytes in all: 1 GiB. */
  totalBytes: 1024 * 1024 * 1024,
  /** One file's bytes: 200 MiB. */
  fileBytes: 200 * 1024 * 1024,
} as const;

export type Kind = "subtitle" | "font" | "archive";

export const KIND_LABEL: Record<Kind, string> = {
  subtitle: "자막",
  font: "폰트",
  archive: "압축 파일",
};

const SUBTITLE = new Set(["ass", "ssa", "srt", "smi", "sami", "vtt", "sub", "idx", "sup", "ttml", "dfxp"]);
const FONT = new Set(["ttf", "otf", "ttc", "woff", "woff2"]);
const ARCHIVE = new Set(["zip", "rar", "7z", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "tar"]);
/** The archive extensions a number may follow for the volume of a split archive (`.7z.001`). */
const VOLUME_OF = new Set(["7z", "zip", "rar", "gz", "bz2", "xz", "tar"]);

/** The reason the server records for any file left out by its name (`SKIPPED_REASON`). */
export const SKIPPED_REASON = "이름으로 보아 자막이나 폰트가 아니라 올리지 않았어요";

/**
 * What a file name's extension says it is, or `null` for a file to leave out.
 * Archives are ZIP, RAR, 7z, gzip, bzip2, xz and tar, and the volumes of a
 * split archive (`.part1.rar`, `.r00`, `.z01`, `.7z.001`): each volume is a
 * file of its own in the package. The server tells the format by the first
 * bytes and drops a file that is none.
 */
export function kindByName(name: string): Kind | null {
  const base = name.slice(name.lastIndexOf("/") + 1).toLowerCase();
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return null;
  const stem = base.slice(0, dot);
  const ext = base.slice(dot + 1);
  if (SUBTITLE.has(ext)) return "subtitle";
  if (FONT.has(ext)) return "font";
  if (ARCHIVE.has(ext)) return "archive";
  // `.r00`, `.z01`: a volume of a split RAR or ZIP.
  if (/^[rz]\d{2,}$/.test(ext)) return "archive";
  // `.7z.001`: a volume cut from an archive.
  const inner = stem.includes(".") ? stem.slice(stem.lastIndexOf(".") + 1) : "";
  if (/^\d{3,}$/.test(ext) && VOLUME_OF.has(inner)) return "archive";
  return null;
}

/** The part of a chosen file the browser knows: a `File`, or a plain object in a test. */
export interface Chosen {
  name: string;
  size: number;
  /** Set for a file picked in a folder: `Show/Season 1/01.ass`. */
  webkitRelativePath?: string;
}

/** A chosen file as the list shows it: its path (the folder's relative one, else its name) and what its name says. */
export interface Entry<F extends Chosen = Chosen> {
  /** A number that does not change while the list does, for React. */
  key: number;
  file: F;
  /** What the server is told the file is called. */
  name: string;
  /** `null`: left out, and only its name is sent. */
  kind: Kind | null;
}

export function entriesOf<F extends Chosen>(files: readonly F[], firstKey: number): Entry<F>[] {
  return files.map((file, i) => {
    const name = file.webkitRelativePath ? file.webkitRelativePath : file.name;
    return { key: firstKey + i, file, name, kind: kindByName(file.name) };
  });
}

export interface Plan<F extends Chosen = Chosen> {
  /** The files to send, in the order they were chosen. */
  send: Entry<F>[];
  /** The files left out, whose names are sent. */
  skip: Entry<F>[];
  counts: Record<Kind, number>;
  totalBytes: number;
  /** Why the upload cannot be sent, as sentences; empty when it can. */
  problems: string[];
}

export function bytesText(bytes: number): string {
  const unit = (n: number, name: string) => `${n < 10 ? n.toFixed(1).replace(/\.0$/, "") : Math.round(n)}${name}`;
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return unit(bytes / 1024, "KB");
  if (bytes < 1024 * 1024 * 1024) return unit(bytes / (1024 * 1024), "MB");
  return unit(bytes / (1024 * 1024 * 1024), "GB");
}

export function planOf<F extends Chosen>(entries: readonly Entry<F>[]): Plan<F> {
  const send = entries.filter((e) => e.kind !== null);
  const skip = entries.filter((e) => e.kind === null);
  const counts: Record<Kind, number> = { subtitle: 0, font: 0, archive: 0 };
  let totalBytes = 0;
  for (const e of send) {
    counts[e.kind as Kind] += 1;
    totalBytes += e.file.size;
  }
  const problems: string[] = [];
  if (send.length > LIMITS.files) {
    problems.push(`한 번에 파일 ${LIMITS.files}개까지 올릴 수 있어요. 지금 ${send.length}개예요. 나눠서 올려 주세요.`);
  }
  if (entries.length > LIMITS.entries) {
    problems.push(
      `한 번에 고를 수 있는 파일은 올리지 않는 파일까지 모두 ${LIMITS.entries}개예요. 지금 ${entries.length}개예요.`,
    );
  }
  const big = send.filter((e) => e.file.size > LIMITS.fileBytes);
  if (big.length > 0) {
    problems.push(`파일 하나는 ${bytesText(LIMITS.fileBytes)}까지 올릴 수 있어요. ${big[0].name}${big.length > 1 ? ` 외 ${big.length - 1}개` : ""}이(가) 넘어요.`);
  }
  if (totalBytes > LIMITS.totalBytes) {
    problems.push(`한 번에 모두 합쳐 ${bytesText(LIMITS.totalBytes)}까지 올릴 수 있어요. 지금 ${bytesText(totalBytes)}예요. 나눠서 올려 주세요.`);
  }
  return { send, skip, counts, totalBytes, problems };
}

/** `자막 2개 · 폰트 1개`: the kinds that are there. */
export function countsText(counts: Record<Kind, number>): string {
  return (["subtitle", "font", "archive"] as const)
    .filter((kind) => counts[kind] > 0)
    .map((kind) => `${KIND_LABEL[kind]} ${counts[kind]}개`)
    .join(" · ");
}

/** What the browser tells the picker about itself. */
export interface Env {
  /** Whether an `<input>` has the `webkitdirectory` property. */
  hasDirectoryInput: boolean;
  /** Whether the main pointer is a finger and there is no hovering (a phone or a tablet). */
  touchOnly: boolean;
}

/** The media query that matches a device whose main pointer is a finger and which cannot hover. */
export const TOUCH_QUERY = "(hover: none) and (pointer: coarse)";

/**
 * Whether to offer `폴더 고르기`. A phone's browser has the property but opens
 * its file picker, which cannot pick a folder, so a touch-only device is left
 * with files and ZIPs.
 */
export function canPickFolder(env: Env): boolean {
  return env.hasDirectoryInput && !env.touchOnly;
}

/**
 * What makes two uploads the same: the creator, the files sent with their
 * sizes and the names left out, in order. The server's digest covers all of
 * them, so one more or fewer left-out name is another upload for the same ID.
 */
export function signatureOf(creator: string, send: readonly Entry[], skip: readonly Entry[]): string {
  return JSON.stringify([
    creator,
    send.map((e) => [e.name, e.file.size, e.file.name, e.kind]),
    skip.map((e) => e.name),
  ]);
}
