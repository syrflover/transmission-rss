import type { ConfirmEpisode, ConfirmView, Placement, PlacementChoice, Relocation } from "./placementTypes.ts";
import { videoText } from "./placementTable.ts";

/**
 * What a relocation job (재배치, `trss_jobs::place::relocate`) shows: for each stored subtitle it moves, the applied
 * copies it takes off their old episodes and the episode it applies the subtitle on, decided from the API's
 * `placements` and `relocations` alone, so the component only draws them and a test can say what appears when.
 */

/** One line of a move: what it says, the path or reason beside it, and whether it needs a look. */
export interface MoveLine {
  text: string;
  detail: string | null;
  /** The new episode has a subtitle: applying there waits for a replacement's approval. */
  replaces: boolean;
  urgent: boolean;
}

/** One stored subtitle a relocation moves. */
export interface Move {
  key: string;
  /** The subtitle's name as its job planned it; a copy's file name when the job has no row for it. */
  name: string;
  /** The copies taken off their old episodes (`2화 적용본 제거`). */
  off: MoveLine[];
  /** Where it is applied (`3화 적용`), or that it is there already. */
  on: MoveLine;
}

/** Why a removal has no row: the stored subtitle is applied on its new episode already. */
export const ALREADY_THERE = "새 회차에는 이미 적용돼 있어요";

const line = (text: string, detail: string | null, urgent = false): MoveLine => ({
  text,
  detail,
  replaces: false,
  urgent,
});

/** What a removal says, before the confirmation and after. */
export function removalLine(r: Relocation): MoveLine {
  const copy = `${r.episode}화 적용본`;
  const why = r.reason !== null && r.reason !== "" ? `${r.path} · ${r.reason}` : r.path;
  switch (r.state) {
    case "planned":
      return line(`${copy} 제거`, r.path);
    case "intended":
    case "set_aside":
      return line(`${copy} 지우는 중`, r.path);
    case "done":
      return line(`${copy} 지움`, r.path);
    case "kept":
      return line(`${copy} 그대로 둠`, why);
    case "held":
      return line(`${copy} 보류`, why, true);
  }
}

/**
 * What a row's application says: before the confirmation (`episodes` given) the episode and the video it goes beside,
 * after it what came of it.
 */
export function applyLine(p: Placement, episodes: readonly ConfirmEpisode[] | null): MoveLine {
  const on = p.episode !== null ? `${p.episode}화` : "회차";
  if (episodes !== null) {
    const video = p.episode !== null ? videoText(p.episode, episodes) : { text: "—", replaces: false };
    return { text: `${on} 적용`, detail: video.text, replaces: video.replaces, urgent: false };
  }
  if (p.outcome === null) {
    return p.question !== null ? line(`${on} 회차 확인 필요`, p.question) : line(`${on} 적용 예정`, null);
  }
  switch (p.outcome) {
    case "applied":
      return line(`${on}에 적용함`, p.note);
    case "no_video":
      return line(`${on} 영상 대기`, p.note);
    case "held":
      return line(`${on} 보류`, p.note, true);
    case "failed":
      return line(`${on} 적용 실패`, p.note, true);
    case "stored":
    case "existing":
    case "dropped":
      return line(`${on} 적용하지 않음`, p.note);
  }
}

const fileName = (path: string) => path.slice(path.lastIndexOf("/") + 1);

/**
 * The moves of a relocation, by the episode they apply on, then by name: each row with the removals that name it,
 * and each removal with no row on its own. `episodes` is the 배치 확인's while the job waits for it, else `null`.
 */
export function moves(
  placements: readonly Placement[],
  relocations: readonly Relocation[],
  episodes: readonly ConfirmEpisode[] | null,
): Move[] {
  const rows = [...placements].sort(
    (a, b) =>
      (a.episode ?? Infinity) - (b.episode ?? Infinity) ||
      a.name.localeCompare(b.name, "ko", { numeric: true }) ||
      a.position - b.position,
  );
  const named: Move[] = rows.map((p) => ({
    key: `row:${p.position}`,
    name: p.name,
    off: relocations.filter((r) => r.position === p.position).map(removalLine),
    on: applyLine(p, episodes),
  }));
  const alone: Move[] = relocations
    .filter((r) => r.position === null || !placements.some((p) => p.position === r.position))
    .map((r) => ({ key: `off:${r.id}`, name: fileName(r.path), off: [removalLine(r)], on: line(ALREADY_THERE, null) }));
  return [...named, ...alone];
}

/** The body of `적용` for a relocation: every row on its planned episode, applied, and every planned removal. */
export function relocationRequest(
  placements: readonly Placement[],
  confirm: ConfirmView,
  relocations: readonly Relocation[],
): { rows: PlacementChoice[]; removals: string[] } {
  const asked = new Set(confirm.positions);
  return {
    rows: placements
      .filter((p) => asked.has(p.position))
      .map((p) => ({ position: p.position, episode: p.episode, apply: true })),
    removals: relocations.filter((r) => r.state === "planned").map((r) => r.id),
  };
}

/** The sentence above a relocation's table. */
export function relocationLead(relocations: readonly Relocation[]): string {
  const planned = relocations.filter((r) => r.state === "planned");
  const n = planned.length;
  // Every new episode has the subtitle already: the plan only takes copies off.
  const what = planned.every((r) => r.position === null)
    ? `회차 대응이 바뀌어 옛 회차의 적용본 ${n}개를 지워요. 새 회차에는 이미 적용돼 있어요. 적용을 누르면 지워요. `
    : `회차 대응이 바뀌어 적용본 ${n}개를 새 회차로 옮겨요. 적용을 누르면 옛 회차의 적용본을 지우고 새 회차에 붙여요. `;
  return what + "확인하기 전에는 옛 회차의 적용본을 그대로 둬요.";
}
