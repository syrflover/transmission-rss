import assert from "node:assert/strict";
import { test } from "node:test";

import { sizeText } from "./size.ts";

test("a size under 1 KB is told in bytes, then KB and MB with one decimal under 10", () => {
  assert.equal(sizeText(0), "0 B");
  assert.equal(sizeText(512), "512 B");
  assert.equal(sizeText(1023), "1023 B");
  assert.equal(sizeText(1024), "1 KB");
  assert.equal(sizeText(1536), "1.5 KB");
  assert.equal(sizeText(22 * 1024), "22 KB");
  assert.equal(sizeText(1024 * 1024), "1 MB");
  assert.equal(sizeText(1234567), "1.2 MB");
  assert.equal(sizeText(200 * 1024 * 1024), "200 MB");
});
