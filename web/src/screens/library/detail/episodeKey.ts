/**
 * How Anissia's episode text is compared. Kept free of imports so the pure helpers and their tests
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
