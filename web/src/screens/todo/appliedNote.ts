/**
 * What a job's detail says of a file that was applied (`docs/specs/subtitles.md`, 보관본과 적용본). Each applied row
 * keeps its own note, which tells what was done for its episode: `기존 자막을 새 자막으로 교체했어요` for a replaced
 * subtitle, `이 회차에 같은 자막이 이미 있어 그 파일을 적용본으로 기록했어요` for one found beside the video with the
 * same bytes (0121). A job's other episodes never lend theirs. No imports, so `node --test` runs it.
 */

/** The words after `적용함` for a placement: its episode and its own note, each when it has one. */
export function appliedDetail(episode: string | null, note: string | null): string | null {
  const parts = [episode, note].filter((part): part is string => part !== null && part !== "");
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** The rows that share one note, in the order the first of them came. */
export interface NoteCluster<T> {
  /** `null` for the rows that have none. */
  note: string | null;
  rows: T[];
}

/**
 * Applied rows grouped by their note, so that twelve episodes replaced the same way say it once instead of twelve
 * times, and an episode with a note of its own stands apart. A note of no words is no note.
 */
export function clusterByNote<T extends { note: string | null }>(rows: readonly T[]): NoteCluster<T>[] {
  const clusters: NoteCluster<T>[] = [];
  for (const row of rows) {
    const note = row.note === "" ? null : row.note;
    const found = clusters.find((c) => c.note === note);
    if (found !== undefined) found.rows.push(row);
    else clusters.push({ note, rows: [row] });
  }
  return clusters;
}
