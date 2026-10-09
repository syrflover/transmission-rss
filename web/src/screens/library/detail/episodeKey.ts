/**
 * How an episode's text is shown and compared. Kept free of imports so the pure helpers and their tests
 * run without the app.
 */

/**
 * The comparison key of text that is plainly a number (`library.md`, 자막의 회차 대응): the
 * leading zeros of the integer part and the trailing zeros of the decimal part
 * go, and a decimal part left empty loses its dot, so `013`, `13` and `13.0` are
 * `13` and `13.50` is `13.5`. No float is made, so `13.5` is never rounded.
 * `null` for any other text.
 */
export function numericKey(text: string): string | null {
  if (!/^\d+(\.\d+)?$/.test(text)) return null;
  const [whole, decimal = ""] = text.split(".");
  const integer = whole.replace(/^0+/, "") || "0";
  const fraction = decimal.replace(/0+$/, "");
  return fraction === "" ? integer : `${integer}.${fraction}`;
}

/** One key for the episodes that are the same: `1` and `01` are, other text is only itself (`SP`). */
export function episodeKey(text: string): string {
  const n = numericKey(text);
  return n === null ? `t:${text}` : `n:${n}`;
}

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
