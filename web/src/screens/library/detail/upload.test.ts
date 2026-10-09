import assert from "node:assert/strict";
import { test } from "node:test";

import {
  canPickFolder,
  countsText,
  entriesOf,
  kindByName,
  planOf,
  signatureOf,
  type Chosen,
  type UploadLimits,
} from "./upload.ts";

/** What the server tells the work detail it holds an upload to. */
const LIMITS: UploadLimits = { files: 500, entries: 2000, total_bytes: 1024 * 1024 * 1024, file_bytes: 200 * 1024 * 1024 };

const file = (name: string, size = 100, webkitRelativePath?: string): Chosen => ({ name, size, webkitRelativePath });

test("a name's extension says subtitle, font, archive or nothing", () => {
  assert.equal(kindByName("01.ass"), "subtitle");
  assert.equal(kindByName("01.KOR.SRT"), "subtitle");
  assert.equal(kindByName("a/b/01.smi"), "subtitle");
  assert.equal(kindByName("Font.TTF"), "font");
  assert.equal(kindByName("font.woff2"), "font");
  assert.equal(kindByName("subs.zip"), "archive");
  assert.equal(kindByName("readme.txt"), null);
  assert.equal(kindByName("cover.jpg"), null);
  assert.equal(kindByName("ass"), null);
  assert.equal(kindByName(".ass"), null);
  assert.equal(kindByName("subs.7z"), "archive");
});

test("the list says what is kept and what is dropped before anything is sent", () => {
  const plan = planOf(entriesOf([file("01.ass"), file("02.ass"), file("a.ttf", 2048), file("readme.txt"), file("x.zip", 10)], 0), LIMITS);
  assert.deepEqual(plan.counts, { subtitle: 2, font: 1, archive: 1 });
  assert.deepEqual(
    plan.skip.map((e) => e.name),
    ["readme.txt"],
  );
  assert.equal(plan.send.length, 4);
  assert.equal(plan.totalBytes, 100 + 100 + 2048 + 10);
  assert.deepEqual(plan.problems, []);
  assert.equal(countsText(plan.counts), "자막 2개 · 폰트 1개 · 압축 파일 1개");
});

test("a folder's files are named by their path in it", () => {
  const [entry] = entriesOf([file("01.ass", 10, "Show/Season 1/01.ass")], 5);
  assert.equal(entry.name, "Show/Season 1/01.ass");
  assert.equal(entry.key, 5);
  assert.equal(entry.kind, "subtitle");
});

test("nothing to keep leaves nothing to send", () => {
  const plan = planOf(entriesOf([file("a.txt"), file("b.png")], 0), LIMITS);
  assert.equal(plan.send.length, 0);
  assert.equal(plan.skip.length, 2);
});

test("limits are told before sending", () => {
  const many = entriesOf(Array.from({ length: LIMITS.files + 1 }, (_, i) => file(`${i}.ass`, 1)), 0);
  assert.equal(planOf(many, LIMITS).problems.length, 1);
  assert.match(planOf(many, LIMITS).problems[0], /500개/);

  const big = planOf(entriesOf([file("big.ttf", LIMITS.file_bytes + 1)], 0), LIMITS);
  assert.equal(big.problems.length, 1);
  assert.match(big.problems[0], /200MB/);

  const sum = planOf(entriesOf(Array.from({ length: 6 }, (_, i) => file(`${i}.zip`, LIMITS.file_bytes)), 0), LIMITS);
  assert.equal(sum.problems.length, 1);
  assert.match(sum.problems[0], /1GB/);

  const named = planOf(entriesOf(Array.from({ length: LIMITS.entries + 1 }, (_, i) => file(`${i}.txt`, 1)), 0), LIMITS);
  assert.equal(named.send.length, 0);
  assert.equal(named.problems.length, 1);

  const fine = planOf(entriesOf([file("a.ass", LIMITS.file_bytes)], 0), LIMITS);
  assert.deepEqual(fine.problems, []);
});

test("a phone is offered files and ZIPs, not a folder", () => {
  assert.equal(canPickFolder({ hasDirectoryInput: true, touchOnly: false }), true);
  assert.equal(canPickFolder({ hasDirectoryInput: true, touchOnly: true }), false);
  assert.equal(canPickFolder({ hasDirectoryInput: false, touchOnly: false }), false);
  assert.equal(canPickFolder({ hasDirectoryInput: false, touchOnly: true }), false);
});

test("the same files and creator make the same signature, a changed list another", () => {
  const a = entriesOf([file("01.ass")], 0);
  const b = entriesOf([file("01.ass")], 9);
  assert.equal(signatureOf("", a, []), signatureOf("", b, []));
  assert.notEqual(signatureOf("x", a, []), signatureOf("", a, []));
  assert.notEqual(signatureOf("", a, []), signatureOf("", entriesOf([file("01.ass", 101)], 0), []));
});

test("the names left out are part of the signature, as they are of the server's digest", () => {
  const send = entriesOf([file("01.ass")], 0);
  const none = signatureOf("", send, []);
  const one = signatureOf("", send, entriesOf([file("readme.txt")], 5));
  const other = signatureOf("", send, entriesOf([file("notes.txt")], 5));
  assert.notEqual(none, one);
  assert.notEqual(one, other);
  assert.equal(one, signatureOf("", send, entriesOf([file("readme.txt")], 40)));
});

test("every archive format and the volumes of a split archive are sent, not left out", () => {
  for (const name of [
    "a.zip",
    "a.rar",
    "Show/subs.7Z",
    "x.tar.gz",
    "x.tgz",
    "y.xz",
    "z.bz2",
    "z.tar",
    "pack.part1.rar",
    "pack.r00",
    "pack.R12",
    "pack.z01",
    "pack.7z.001",
    "pack.zip.002",
  ]) {
    assert.equal(kindByName(name), "archive", name);
  }
  for (const name of ["readme.txt", "cover.jpg", "rar", ".rar", "a.rar.txt", "a.001", "a.r1", "a.rxx", "a.7z.01", "a.zst"]) {
    assert.equal(kindByName(name), null, name);
  }
  const plan = planOf(entriesOf([file("a.rar", 10), file("b.7z.001", 20), file("c.txt")], 0), LIMITS);
  assert.deepEqual(plan.counts, { subtitle: 0, font: 0, archive: 2 });
  assert.deepEqual(
    plan.skip.map((e) => e.name),
    ["c.txt"],
  );
});
