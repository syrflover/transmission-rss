import assert from "node:assert/strict";
import { test } from "node:test";

import { deviceViewport, sameViewport, toRemote } from "./geometry.ts";
import type { ScreenLimits } from "./protocol.ts";

/** What the server tells the device its socket takes. */
const LIMITS: ScreenLimits = {
  min_side: 200,
  max_side: 4096,
  min_dpr: 0.5,
  max_dpr: 4,
  max_message_bytes: 64 * 1024,
  max_prompt_text: 2000,
};

test("a phone reports its area's width, a height from its screen's, its pixel ratio and touch", () => {
  const v = deviceViewport({ areaWidth: 358.6, deviceHeight: 844, dpr: 3, touch: true }, LIMITS);
  assert.deepEqual(v, { width: 358, height: 506, dpr: 3, touch: true });
});

test("the height stays within 360 and 720 whatever the device", () => {
  assert.equal(deviceViewport({ areaWidth: 800, deviceHeight: 300, dpr: 1, touch: false }, LIMITS).height, 360);
  assert.equal(deviceViewport({ areaWidth: 800, deviceHeight: 2160, dpr: 1, touch: false }, LIMITS).height, 720);
});

test("sizes and ratios outside what the server takes are held to its bounds", () => {
  const small = deviceViewport({ areaWidth: 120, deviceHeight: 700, dpr: 0.1, touch: false }, LIMITS);
  assert.equal(small.width, 200);
  assert.equal(small.dpr, 0.5);
  const big = deviceViewport({ areaWidth: 9000, deviceHeight: 700, dpr: 9, touch: false }, LIMITS);
  assert.equal(big.width, 4096);
  assert.equal(big.dpr, 4);
  assert.equal(deviceViewport({ areaWidth: 500, deviceHeight: 700, dpr: Number.NaN, touch: false }, LIMITS).dpr, 1);
});

test("viewports are the same only when every part is", () => {
  const a = { width: 400, height: 600, dpr: 2, touch: true };
  assert.ok(sameViewport(a, { ...a }));
  assert.ok(!sameViewport(a, { ...a, width: 401 }));
  assert.ok(!sameViewport(a, { ...a, touch: false }));
});

test("a point in the box maps to the frame's own pixels when the frame is shown at its size", () => {
  const box = { left: 100, top: 50, width: 400, height: 600 };
  assert.deepEqual(toRemote({ clientX: 110, clientY: 80 }, box, { width: 400, height: 600 }), { x: 10, y: 30 });
});

test("a frame of another size than the box is scaled on each axis", () => {
  // Another device's 402x666 frame shown in a 804x1332 box: every pixel counts twice.
  const box = { left: 0, top: 0, width: 804, height: 1332 };
  assert.deepEqual(toRemote({ clientX: 804, clientY: 666 }, box, { width: 402, height: 666 }), { x: 401.5, y: 333 });
  // A box of a different aspect: the ratios differ and each axis keeps its own.
  const wide = { left: 10, top: 10, width: 200, height: 100 };
  assert.deepEqual(toRemote({ clientX: 110, clientY: 60 }, wide, { width: 400, height: 400 }), { x: 200, y: 200 });
});

test("a point outside the box (a drag that left it) is held just inside the frame", () => {
  const box = { left: 0, top: 0, width: 400, height: 600 };
  const frame = { width: 400, height: 600 };
  assert.deepEqual(toRemote({ clientX: -30, clientY: 9000 }, box, frame), { x: 0, y: 599.5 });
  assert.deepEqual(toRemote({ clientX: 400, clientY: 0 }, box, frame), { x: 399.5, y: 0 });
});

test("an empty box or frame has no point", () => {
  assert.equal(toRemote({ clientX: 1, clientY: 1 }, { left: 0, top: 0, width: 0, height: 10 }, { width: 10, height: 10 }), null);
  assert.equal(toRemote({ clientX: 1, clientY: 1 }, { left: 0, top: 0, width: 10, height: 10 }, { width: 0, height: 10 }), null);
});
