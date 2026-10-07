/**
 * The creator a season's subtitle files show (`library.md`, 머리와 시즌): the head's names, the files `제작자 지정`
 * counts, and the creator beside a file. A copy trss applied beside a video is by the creator of its stored copy
 * (`applied`, which the server reads from the applied relation); any other file is by the creator the user named, or
 * `제작자 알 수 없음`. It imports nothing, not even the API's types (whose module has `@/` imports), so it runs and is
 * type-checked under `node --test`.
 */

export const UNKNOWN = "제작자 알 수 없음";

/** The part of a subtitle file (`WorkSubtitle`) these read. */
export interface CreatorFile {
  /** The creator the user named; `null` is `제작자 알 수 없음`, and always so for an applied copy. */
  creator: { name: string } | null;
  /** Set for a copy trss applied beside a video: `creator` is its stored copy's (`null`: unknown). */
  applied: { creator: string | null } | null;
}

/**
 * The season's subscription as the head shows it: its line (`subtitleChoice`: the creator it follows, `제작자 미정`,
 * `받지 않음`) and the creator it follows, which the file creators then leave out.
 */
export interface HeadSubscription {
  text: string;
  followed: string | null;
}

/** The creator a file shows: its stored copy's for an applied copy, else the one the user named; `null` is unknown. */
export function creatorOf(file: CreatorFile): string | null {
  return file.applied ? file.applied.creator : (file.creator?.name ?? null);
}

/** A file whose creator the user may name or change: one a person put there, not a copy trss applied. */
export function isNameable(file: Pick<CreatorFile, "applied">): boolean {
  return file.applied === null;
}

/** The files `제작자 지정` names: the nameable ones that have no creator. */
export function unknownFiles<T extends CreatorFile>(files: readonly T[]): T[] {
  return files.filter((f) => isNameable(f) && f.creator === null);
}

/** The creators of the season's subtitle files, once each, by name. */
export function namedCreators(files: readonly CreatorFile[]): string[] {
  const names = new Set<string>();
  for (const file of files) {
    const name = creatorOf(file);
    if (name !== null) names.add(name);
  }
  return [...names].sort((a, b) => a.localeCompare(b, "ko"));
}

/**
 * The names the head shows for a season: the subscription's line, every creator of a file, and `제작자 알 수 없음`
 * while some file has none (an applied copy of a stored copy with no creator included).
 */
export function headNames(files: readonly CreatorFile[], subscription: HeadSubscription | undefined): string[] {
  const named = namedCreators(files);
  const names: string[] = [];
  if (subscription && subscription.text !== "") names.push(subscription.text);
  const followed = subscription?.followed ?? null;
  names.push(...named.filter((name) => name !== followed));
  if (files.some((f) => creatorOf(f) === null)) names.push(UNKNOWN);
  return names;
}
