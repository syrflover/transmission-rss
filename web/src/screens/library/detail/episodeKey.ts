/**
 * How an episode's text is shown. Kept free of imports so the pure helpers and their tests
 * run without the app.
 */

/** `12화` for an episode as the server shows it (`episode_shown`) when it starts as a number, the text itself for anything else (`SP`). */
export function episodeLabel(shown: string): string {
  return /^\d/.test(shown) ? `${shown}화` : shown;
}

/**
 * A creator's episodes in a line, from the runs the server names: the runs of whole numbers joined by `·` and
 * followed by `화` (`1–4·11–15화`), then every other text after ` · ` (`1–4화 · SP`). No runs, no text.
 */
export function segmentsText(segments: readonly { text: string; whole: boolean }[]): string {
  const whole = segments.filter((s) => s.whole).map((s) => s.text);
  const others = segments.filter((s) => !s.whole).map((s) => s.text);
  const parts: string[] = [];
  if (whole.length > 0) parts.push(`${whole.join("·")}화`);
  parts.push(...others);
  return parts.join(" · ");
}
