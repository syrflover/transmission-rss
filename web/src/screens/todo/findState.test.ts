import assert from "node:assert/strict";
import { test } from "node:test";

import { findShown } from "./findState.ts";

test("a find job is 직접 찾는 중 while its screen is open, and 끝내는 중 once finished", () => {
  const browsing = { state: "waiting", wait: "auth", finishing: false, kept: 0 };
  assert.equal(findShown(browsing), "finding");
  assert.equal(findShown({ ...browsing, kept: 2 }), "finding");
  assert.equal(findShown({ ...browsing, finishing: true }), "finishing");
  // Waiting for a server browser, in line, or being opened, it is named as any job.
  assert.equal(findShown({ ...browsing, wait: "subtitle" }), null);
  assert.equal(findShown({ ...browsing, state: "pending", wait: null }), null);
  assert.equal(findShown({ ...browsing, state: "running", wait: null }), null);
});

test("a find job that kept nothing ends as 받은 파일 없음, not 받음", () => {
  const ended = { state: "done", wait: null, finishing: false, kept: 0 };
  assert.equal(findShown(ended), "nothing");
  assert.equal(findShown({ ...ended, kept: 1 }), null);
});
