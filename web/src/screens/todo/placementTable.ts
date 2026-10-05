import type { ConfirmEpisode, ConfirmView, Placement, PlacementChoice } from "./placementTypes.ts";

/**
 * What the 배치 확인 table of a job (`docs/specs/subtitles.md`, 배치 확인) shows and sends, decided from the API's
 * `Placement`s and `ConfirmView` alone, so the component only draws it and a test can say what appears when.
 */

/**
 * What a person chose for a row: an episode number, `skip` (`적용하지 않음`: kept only) or `unset` (a held row nobody
 * chose for yet).
 */
export type Choice = number | "skip" | "unset";

/** The rows of the table: the placements the job asks about, held ones first, then by planned episode and name. */
export function tableRows(placements: readonly Placement[], confirm: ConfirmView): Placement[] {
  const asked = new Set(confirm.positions);
  return placements
    .filter((p) => asked.has(p.position))
    .sort(
      (a, b) =>
        Number(b.question !== null) - Number(a.question !== null) ||
        (a.episode ?? Infinity) - (b.episode ?? Infinity) ||
        a.name.localeCompare(b.name, "ko", { numeric: true }) ||
        a.position - b.position,
    );
}

/**
 * Where a row stands before the person touches it: a planned `apply` on an episode is that episode; a row the plan
 * only stores (another format, the same bytes twice) is `skip`; a held row, or one to apply with no episode, is
 * `unset` and has to be chosen.
 */
export function initialChoice(p: Placement): Choice {
  if (p.question !== null) return "unset";
  if (p.action === "apply") return p.episode ?? "unset";
  return "skip";
}

/** The choice of every row by position. */
export function initialChoices(rows: readonly Placement[]): Map<number, Choice> {
  return new Map(rows.map((p) => [p.position, initialChoice(p)]));
}

/** How many rows still have to be chosen. */
export function unsetCount(choices: ReadonlyMap<number, Choice>): number {
  let n = 0;
  for (const choice of choices.values()) if (choice === "unset") n += 1;
  return n;
}

/**
 * The body of `적용`: an episode is applied there; `skip` keeps the row's planned episode (`null` when it had none)
 * and does not apply it. `null` while a row is still `unset`.
 */
export function requestRows(
  rows: readonly Placement[],
  choices: ReadonlyMap<number, Choice>,
): PlacementChoice[] | null {
  const out: PlacementChoice[] = [];
  for (const p of rows) {
    const choice = choices.get(p.position) ?? initialChoice(p);
    if (choice === "unset") return null;
    out.push(
      choice === "skip"
        ? { position: p.position, episode: p.episode, apply: false }
        : { position: p.position, episode: choice, apply: true },
    );
  }
  return out;
}

const UNWRITTEN = "확인하기 전에는 작품 폴더에 아무것도 쓰지 않아요.";

/**
 * The sentence above the table. It says how the episodes were chosen only for rows the plan did choose: a table whose
 * rows all have to be chosen says so, and a package of fonts and attachments alone asks only to keep them.
 */
export function leadText(rows: readonly Placement[], confirm: ConfirmView): string {
  if (rows.length === 0) return `받은 폰트와 첨부를 이 작품에 보관할지 확인해 주세요. ${UNWRITTEN}`;
  const open = rows.filter((p) => initialChoice(p) === "unset").length;
  if (confirm.scope !== "whole") {
    return `이 파일들은 회차를 직접 골라야 해요. 회차를 고르거나 적용하지 않음으로 두고 적용을 눌러 주세요. ${UNWRITTEN}`;
  }
  if (open === rows.length) {
    return `파일 이름으로 회차를 정하지 못했어요. 회차를 고르거나 적용하지 않음으로 두고 적용을 누르면 보관하고 영상 옆에 붙여요. ${UNWRITTEN}`;
  }
  const rest = open > 0 ? " 정하지 못한 파일은 회차를 골라 주세요." : "";
  return `파일 이름의 번호로 회차를 정했어요.${rest} 확인하고 적용을 누르면 보관하고 영상 옆에 붙여요. ${UNWRITTEN}`;
}

/** What the `붙을 영상 이름` cell says for a choice. */
export interface VideoText {
  text: string;
  /** The file goes on an episode that has a subtitle: it waits for a replacement comparison and approval. */
  replaces: boolean;
}

export const NO_VIDEO = "영상 없음 · 영상이 생기면 붙여요";
export const STORE_ONLY = "보관만 해요";
export const REPLACES = "이미 자막이 있어 교체를 비교해요";

/** The video a choice puts the file beside: its name, why there is none, or that the file is only kept. */
export function videoText(choice: Choice, episodes: readonly ConfirmEpisode[]): VideoText {
  if (choice === "skip") return { text: STORE_ONLY, replaces: false };
  if (choice === "unset") return { text: "—", replaces: false };
  const e = episodes.find((x) => x.episode === choice);
  if (e === undefined) return { text: "—", replaces: false };
  const text = e.videos === 0 ? NO_VIDEO : e.videos > 1 ? `영상 ${e.videos}개` : (e.video ?? "영상 1개");
  return { text, replaces: e.subtitle };
}

/**
 * Why a mapped row is on the episode it is: the name says `13` and the season's numbering puts it on `1화`. `null`
 * when the name says the episode itself, the row was not mapped, or the person moved it.
 */
export function mappedNote(p: Placement, choice: Choice): string | null {
  if (p.assignment !== "mapped" || p.named === null || p.episode === null || choice !== p.episode) return null;
  if (Number(p.named) === p.episode) return null;
  return `이름 ${p.named} → ${p.episode}화 (회차 대응)`;
}

/** What `kind` is called in the summary of the rows outside the table. */
const KIND: Record<Placement["kind"], string> = {
  subtitle: "자막",
  font: "폰트",
  attachment: "첨부",
  companion: "구성 파일",
  other: "그 밖의 파일",
};

const KIND_ORDER: Placement["kind"][] = ["font", "attachment", "companion", "subtitle", "other"];

/** The rows outside the table: kept with the package (by kind) and dropped (each with why). */
export interface RestSummary {
  kept: { kind: string; names: string[] }[];
  dropped: { name: string; note: string | null }[];
}

/**
 * What the table leaves out, for a whole plan: the files kept with the package (fonts, attachments, companions) with
 * their names, and the dropped files with the reason. A plan of held rows only has none: its other rows were
 * placed before.
 */
export function restSummary(placements: readonly Placement[], confirm: ConfirmView): RestSummary {
  const none: RestSummary = { kept: [], dropped: [] };
  if (confirm.scope !== "whole") return none;
  const asked = new Set(confirm.positions);
  const rest = placements.filter((p) => !asked.has(p.position));
  return {
    kept: KIND_ORDER.map((kind) => ({
      kind: KIND[kind],
      names: rest.filter((p) => p.kind === kind && p.action !== "drop").map((p) => p.name),
    })).filter((group) => group.names.length > 0),
    dropped: rest.filter((p) => p.action === "drop").map((p) => ({ name: p.name, note: p.note })),
  };
}
