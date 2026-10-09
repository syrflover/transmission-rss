/**
 * The episodes of a to-do or a job in a line. Kept free of imports so the pure helper and its test run without the
 * app.
 */

/**
 * A run of episodes the server names: `1–4`, `7`, `SP` (`trss_core::episode::segments`). The screen only joins and
 * shortens them. `count` is how many episodes the run holds; `whole` is whether it is of whole numbers (`0` and `13.0`
 * included) rather than an episode of any other text.
 */
export interface EpisodeSegment {
  text: string;
  count: number;
  whole: boolean;
}

/** How many segments of the episode list the line names before it says `외 N개`. */
const SEGMENTS = 3;

export interface EpisodeLabel {
  /** `11화`, `2–3화`, `2·5화`, `1–4·7화`: what is shown bold. */
  label: string;
  /** Episodes past the shortened list (`외 3개`); 0 when the list is whole. */
  more: number;
}

/**
 * The episodes of a to-do or a job in a line, from the runs the server names (`2–3`, `7`, `SP`): joined by `·`, and a
 * long list shortened (`1–4·7화` and `외 3개`, which counts the episodes left out). No runs, no label.
 */
export function episodeLabel(segments: readonly EpisodeSegment[]): EpisodeLabel | null {
  if (segments.length === 0) return null;
  const head = segments.slice(0, SEGMENTS);
  const more = segments.slice(SEGMENTS).reduce((sum, s) => sum + s.count, 0);
  return { label: `${head.map((s) => s.text).join("·")}화`, more };
}
