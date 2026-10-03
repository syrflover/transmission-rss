import { episodeKey, numericKey } from "./episodeKey.ts";

/**
 * The pure parts of `회차 대응 정하기` (`library.md`, 자막의 회차 대응): what each choice
 * means as an offset, where the source's episodes go under it, the exceptions the user wrote
 * and the one line the group shows after the save. Nothing here reads the server; the
 * server decides the same way (`trss_jobs::mapping`), and these only show the user what a
 * choice would do before it is saved.
 */

/** The largest offset and season episode the server takes. */
export const MAX_NUMBER = 9999;

export type Choice = "same" | "continue" | "custom";

/** An exception as the user writes it: Anissia's episode text, the season episode, or `받지 않음`. */
export interface ExceptionRow {
  episode: string;
  target: string;
  skip: boolean;
}

/** An exception as the server stores it. */
export interface ExceptionValue {
  episode: string;
  target: number | null;
}

/** A positive whole episode number in Anissia's text (not `0`, `13.5` or text), compared as a number. */
export function wholeOf(text: string): number | null {
  const n = numericKey(text.trim());
  if (n === null || n.includes(".")) return null;
  const value = Number(n);
  return value > 0 ? value : null;
}

/** An integer written in a box (`-12`, `0`, `3`), or `null` for anything else. */
function integerOf(text: string): number | null {
  const t = text.trim();
  return /^[+-]?\d{1,5}$/.test(t) ? Number(t) : null;
}

/** Why `앞 시즌에 이어 셈` cannot be chosen, or `null` when it can. */
export function continueReason(previous: number | null | undefined): string | null {
  if (previous === null || previous === undefined) return "앞 시즌의 회차 수를 알 수 없어서 고를 수 없어요.";
  if (previous <= 0) return "앞 시즌이 없어서 고를 수 없어요.";
  return null;
}

/** The offset a choice means (what is added to Anissia's whole episode), or `null` while it cannot be said. */
export function offsetOf(choice: Choice, custom: string, previous: number | null | undefined): number | null {
  switch (choice) {
    case "same":
      return 0;
    case "continue":
      return continueReason(previous) === null ? -(previous as number) : null;
    case "custom": {
      const n = integerOf(custom);
      return n !== null && Math.abs(n) <= MAX_NUMBER ? n : null;
    }
  }
}

/** The choice that already means `offset`: the one a dialog opens on. */
export function choiceOf(offset: number | null, previous: number | null | undefined): Choice | null {
  if (offset === null) return null;
  if (offset === 0) return "same";
  if (continueReason(previous) === null && offset === -(previous as number)) return "continue";
  return "custom";
}

/** The rows of the dialog for the exceptions a mapping has. */
export function rowsOf(exceptions: readonly ExceptionValue[]): ExceptionRow[] {
  return exceptions.map((e) => ({
    episode: e.episode,
    target: e.target === null ? "" : String(e.target),
    skip: e.target === null,
  }));
}

/** An exception row that was left empty (the user added it and wrote nothing). */
const blank = (row: ExceptionRow) => row.episode.trim() === "" && row.target.trim() === "" && !row.skip;

/**
 * The exceptions the rows say, or the sentence for the first row that cannot be saved. An empty row is left out.
 * Two rows for one episode number (`013` and `13`) cannot be saved: the server refuses them too.
 */
export function exceptionsOf(rows: readonly ExceptionRow[]): { ok: true; exceptions: ExceptionValue[] } | { ok: false; message: string } {
  const exceptions: ExceptionValue[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    if (blank(row)) continue;
    const episode = row.episode.trim();
    if (episode === "") return { ok: false, message: "예외의 회차를 써 주세요." };
    const key = episodeKey(episode);
    if (seen.has(key)) return { ok: false, message: `회차 ‘${episode}’의 예외가 둘이에요. 같은 회차에는 예외를 하나만 둘 수 있어요.` };
    seen.add(key);
    if (row.skip) {
      exceptions.push({ episode, target: null });
      continue;
    }
    const target = integerOf(row.target);
    if (target === null || target < 1 || target > MAX_NUMBER) {
      return { ok: false, message: `회차 ‘${episode}’가 시즌의 몇 화인지 1 이상의 숫자로 써 주세요. 받지 않으려면 ‘받지 않음’을 골라 주세요.` };
    }
    exceptions.push({ episode, target });
  }
  return { ok: true, exceptions };
}

/** Where a source's episode goes: a season episode, nowhere on purpose (`받지 않음`), or nowhere the mapping can say. */
export type Mapped = { kind: "episode"; n: number } | { kind: "skip" } | { kind: "none" };

/** Where the mapping puts the episode text: an exception first, else the offset for a whole episode. */
export function mapOf(text: string, offset: number | null, exceptions: readonly ExceptionValue[]): Mapped {
  const key = episodeKey(text);
  const exception = exceptions.find((e) => episodeKey(e.episode) === key);
  if (exception) return exception.target === null ? { kind: "skip" } : { kind: "episode", n: exception.target };
  const whole = wholeOf(text);
  return offset === null || whole === null ? { kind: "none" } : { kind: "episode", n: whole + offset };
}

/** Anissia's episode texts of a source, each once (by number) and in order: numbers ascending, then the other texts. `0` is left out. */
export function episodesOf(texts: readonly string[]): string[] {
  const byKey = new Map<string, string>();
  for (const text of texts) {
    const key = episodeKey(text);
    if (numericKey(text) === "0") continue;
    if (!byKey.has(key)) byKey.set(key, text);
  }
  // Only the order of the list is made from a float; no episode is ever compared by it.
  return [...byKey.values()].sort((a, b) => {
    const x = numericKey(a);
    const y = numericKey(b);
    if (x !== null && y !== null) return Number(x) - Number(y);
    if (x !== null) return -1;
    if (y !== null) return 1;
    return a < b ? -1 : a > b ? 1 : 0;
  });
}

/** `13화` for a number's text (`013` too), the text itself for any other (`SP`). */
const shown = (text: string) => {
  const n = numericKey(text);
  return n === null ? text : `${n}화`;
};

/** One line of the preview: Anissia's episode, and where it goes. */
export interface Preview {
  episode: string;
  /** `1화`, `32화 (시즌 회차 수 밖)`, `받지 않음`, `시즌 밖` or `정해지지 않음`. */
  to: string;
  /** The season episode it is received as (an exception's may be past the count); `null` when it is not received. */
  n: number | null;
  /** The episode is received as an episode inside the season. */
  lands: boolean;
}

/** The label of an episode an exception puts past the season's count. */
const PAST = "시즌 회차 수 밖";

/** Where each of the source's episodes goes under an offset and exceptions; `total` is the season's episode count when known. */
export function previewOf(
  episodes: readonly string[],
  offset: number | null,
  exceptions: readonly ExceptionValue[],
  total: number | null | undefined,
): Preview[] {
  const covered = new Set(exceptions.map((e) => episodeKey(e.episode)));
  return episodesOf(episodes).map((episode): Preview => {
    const mapped = mapOf(episode, offset, exceptions);
    if (mapped.kind === "skip") return { episode, to: "받지 않음", n: null, lands: false };
    if (mapped.kind === "none") return { episode, to: "정해지지 않음", n: null, lands: false };
    const outside = mapped.n < 1 || (total != null && mapped.n > total);
    // The server receives what an exception covers whatever the count says; only the default offset is held to 1–N.
    if (covered.has(episodeKey(episode))) {
      return { episode, to: outside ? `${mapped.n}화 (${PAST})` : `${mapped.n}화`, n: mapped.n, lands: !outside };
    }
    return { episode, to: outside ? "시즌 밖" : `${mapped.n}화`, n: outside ? null : mapped.n, lands: !outside };
  });
}

/**
 * The warnings of a preview: a season episode that two or more of the source's episodes land on (`13.5 → 13` with
 * offset 0 while `13` exists). The save is not refused; the user is told.
 */
export function collisions(previews: readonly Preview[]): string[] {
  const byEpisode = new Map<number, string[]>();
  for (const p of previews) {
    if (p.n === null) continue;
    byEpisode.set(p.n, [...(byEpisode.get(p.n) ?? []), p.episode]);
  }
  const count = (n: number) => (n === 2 ? "두" : n === 3 ? "세" : `${n}개`);
  return [...byEpisode.entries()]
    .filter(([, episodes]) => episodes.length > 1)
    .sort(([a], [b]) => a - b)
    .map(([n, episodes]) => `${n}화에 ${count(episodes.length)} 회차가 들어와요 (${episodes.join(", ")})`);
}

/** The preview as one short text: `13화 → 1화 · 14화 → 2화 … 24화 → 12화`. */
export function previewText(previews: readonly Preview[], shownCount = 4): string {
  const one = (p: Preview) => `${shown(p.episode)} → ${p.to}`;
  if (previews.length <= shownCount) return previews.map(one).join(" · ");
  const head = previews.slice(0, shownCount - 1).map(one).join(" · ");
  return `${head} … ${one(previews[previews.length - 1])}`;
}

/**
 * The episodes the default offset does not place in the season and no exception covers: a decimal or text
 * episode, or one that falls outside `1`–`total`. The dialog offers them as exceptions to write.
 */
export function misfits(
  episodes: readonly string[],
  offset: number | null,
  exceptions: readonly ExceptionValue[],
  total: number | null | undefined,
): string[] {
  const covered = new Set(exceptions.map((e) => episodeKey(e.episode)));
  return previewOf(episodes, offset, exceptions, total)
    .filter((p) => !covered.has(episodeKey(p.episode)) && (p.to === "정해지지 않음" || p.to === "시즌 밖"))
    .map((p) => p.episode);
}

/** `13.5 받지 않음` or `14 → 3화`. */
function exceptionText(e: ExceptionValue): string {
  return e.target === null ? `${e.episode} 받지 않음` : `${e.episode} → ${e.target}화`;
}

/**
 * The group's line for a mapping the user set: `직접 정함 · 같은 번호`, `직접 정함 · 13화 → 1화`, and the
 * exceptions after it (`· 예외 13.5 받지 않음`, with `외 N개` past two). The example is the first of the
 * source's episodes that the offset places inside the season and no exception covers, written by its number
 * (`013` is `13화`); with none, the offset itself.
 */
export function userLine(
  offset: number | null,
  exceptions: readonly ExceptionValue[],
  episodes: readonly string[],
  total?: number | null,
): string {
  const parts = ["직접 정함"];
  if (offset === null) {
    parts.push("대응 미정");
  } else if (offset === 0) {
    parts.push("같은 번호");
  } else {
    const covered = new Set(exceptions.map((e) => episodeKey(e.episode)));
    const example = previewOf(
      episodes.filter((episode) => !covered.has(episodeKey(episode))),
      offset,
      [],
      total,
    ).find((p) => p.lands && p.n !== null && wholeOf(p.episode) !== null);
    parts.push(example ? `${shown(example.episode)} → ${example.n}화` : `${offset > 0 ? "+" : ""}${offset}화 차이`);
  }
  if (exceptions.length > 0) {
    const names = exceptions.slice(0, 2).map(exceptionText).join(", ");
    parts.push(`예외 ${names}${exceptions.length > 2 ? ` 외 ${exceptions.length - 2}개` : ""}`);
  }
  return parts.join(" · ");
}

/** A mapping as the server sends it, as far as its line needs. */
export interface MappingFacts {
  kind: "auto" | "undecided" | "user";
  offset: number | null;
  evidence: string;
  exceptions: readonly ExceptionValue[];
}

/** A creator's mapping as one quiet line: `자동 · <근거>`, `회차 대응 미정 · <이유>` or `직접 정함 · <대응>`. */
export function mappingText(mapping: MappingFacts, episodes: readonly string[], total?: number | null): string {
  if (mapping.kind === "user") return userLine(mapping.offset, mapping.exceptions, episodes, total);
  const label = mapping.kind === "auto" ? "자동" : "회차 대응 미정";
  return `${label} · ${mapping.evidence}`;
}
