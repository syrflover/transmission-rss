/**
 * How an episode's text is shown and compared. Kept free of imports so the pure helpers and their tests
 * (`mapping.ts`) run without the app.
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

/** An episode as shown: leading zeros of a whole number go (`01` is `1`); anything else stays as written. */
export function shownEpisode(episode: string): string {
  return /^\d+$/.test(episode) ? episode.replace(/^0+(?=\d)/, "") : episode;
}

/** `12화` for a number, the text itself for anything else (`SP`). */
export function episodeLabel(episode: string): string {
  const shown = shownEpisode(episode);
  return /^\d/.test(shown) ? `${shown}화` : shown;
}
