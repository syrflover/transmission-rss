import assert from "node:assert/strict";
import { test } from "node:test";

import { FONT_RECEIPT_LABEL, archiveReceiptText, fontReceiptText } from "./fontReceipt.ts";

test("each font receipt has its own words, and a file that is no kept font has none", () => {
  assert.equal(fontReceiptText("unchanged"), "받지 않음(바뀌지 않음)");
  assert.equal(fontReceiptText("same"), "받아서 같음");
  assert.equal(fontReceiptText("new"), "새로 받음");
  assert.equal(fontReceiptText(null), null);
  assert.equal(fontReceiptText(undefined), null);
  // Only a font not received says it was not received.
  const notReceived = Object.entries(FONT_RECEIPT_LABEL).filter(([, words]) => words.startsWith("받지 않음"));
  assert.deepEqual(
    notReceived.map(([receipt]) => receipt),
    ["unchanged"],
  );
});

test("an archive says it was received whole and counts the new files it added, none too", () => {
  assert.equal(archiveReceiptText(0), "묶음 전체를 받음 · 새로 보관한 파일 0개");
  assert.equal(archiveReceiptText(3), "묶음 전체를 받음 · 새로 보관한 파일 3개");
  assert.equal(archiveReceiptText(null), null);
  assert.equal(archiveReceiptText(undefined), null);
});
