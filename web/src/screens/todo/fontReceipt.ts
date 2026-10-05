/**
 * What a job's detail says about how its fonts were received (`docs/specs/subtitles.md`, 폰트): a Google Drive font
 * whose size and `Last-Modified` did not change is not received again; any other font is received and compared with
 * the fonts kept already; an archive comes whole. Only a receipt that asked for no bytes reads as not received:
 * keeping one copy of the same bytes is not a skipped download. No imports, so `node --test` runs it.
 */

/** What a kept font's receipt came to (`font_receipt` of a placement, `crates/trss-web/src/jobs_api.rs`). */
export type FontReceipt = "unchanged" | "same" | "new";

/** The words for each, as the job detail writes them beside the font. */
export const FONT_RECEIPT_LABEL: Record<FontReceipt, string> = {
  unchanged: "받지 않음(바뀌지 않음)",
  same: "받아서 같음",
  new: "새로 받음",
};

/** The word for a font's receipt; `null` for a file that is no kept font. */
export function fontReceiptText(receipt: FontReceipt | null | undefined): string | null {
  return receipt === null || receipt === undefined ? null : FONT_RECEIPT_LABEL[receipt];
}

/**
 * What a received archive came to: received whole, and how many of its files became new stored files
 * (`묶음 전체를 받음 · 새로 보관한 파일 0개`); `null` while its files are not all settled.
 */
export function archiveReceiptText(newAssets: number | null | undefined): string | null {
  return newAssets === null || newAssets === undefined ? null : `묶음 전체를 받음 · 새로 보관한 파일 ${newAssets}개`;
}
