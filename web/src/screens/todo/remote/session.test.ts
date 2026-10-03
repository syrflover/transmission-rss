import assert from "node:assert/strict";
import { test } from "node:test";

import { inputGen, NEW_SESSION, receive, sentViewport } from "./session.ts";
import type { ServerMessage } from "./protocol.ts";

const phone = { width: 402, height: 666, dpr: 3, touch: true };
const desk = { width: 1272, height: 753, dpr: 1, touch: false };
const size = (gen: number, v = phone): ServerMessage => ({ type: "viewport", gen, ...v });
const frame = (gen: number, width = 402, height = 666): ServerMessage => ({ type: "frame", gen, width, height, data: "" });

test("no input is sent before a frame is on screen", () => {
  assert.equal(inputGen(NEW_SESSION), null);
  assert.equal(inputGen(receive(NEW_SESSION, size(1))), null);
});

test("an input carries the generation of the frame it was made on", () => {
  let s = receive(NEW_SESSION, size(4));
  s = receive(s, frame(4));
  assert.equal(inputGen(s), 4);
});

test("while this device's size is sent and not confirmed, inputs are dropped", () => {
  let s = receive(receive(NEW_SESSION, size(1, desk)), frame(1, 1272, 753));
  assert.equal(inputGen(s), 1);
  s = sentViewport(s, phone);
  assert.equal(inputGen(s), null, "the old frame is of the old size");
  s = receive(s, size(2));
  assert.equal(inputGen(s), null, "confirmed, but the frame on screen is still of generation 1");
  s = receive(s, frame(2));
  assert.equal(inputGen(s), 2);
});

test("a frame that comes before the confirmation still waits for it", () => {
  let s = receive(receive(NEW_SESSION, size(1, desk)), frame(1, 1272, 753));
  s = sentViewport(s, phone);
  s = receive(s, frame(2));
  assert.equal(inputGen(s), null);
  s = receive(s, size(2));
  assert.equal(inputGen(s), 2);
});

test("another device's size does not confirm this one's, but makes the frame on screen old", () => {
  let s = receive(receive(NEW_SESSION, size(1)), frame(1));
  s = sentViewport(s, phone);
  s = receive(s, size(2, desk));
  assert.equal(inputGen(s), null, "this device's own size is still unconfirmed");
  s = receive(s, size(3, phone));
  assert.equal(inputGen(s), null, "the frame on screen is of generation 1, the page is at 3");
  s = receive(s, frame(3));
  assert.equal(inputGen(s), 3);
});

test("a size change by another device makes the frames on screen old until a new frame comes", () => {
  let s = receive(receive(NEW_SESSION, size(1)), frame(1));
  s = receive(s, size(2, desk));
  assert.equal(inputGen(s), null);
  s = receive(s, frame(2, 1272, 753));
  assert.equal(inputGen(s), 2);
});

test("a late frame of an older generation does not make inputs valid again", () => {
  let s = receive(receive(NEW_SESSION, size(1)), frame(1));
  s = receive(s, size(2, desk));
  s = receive(s, frame(1));
  assert.equal(inputGen(s), null);
});

test("a dropped notice moves the page's generation on", () => {
  let s = receive(receive(NEW_SESSION, size(1)), frame(1));
  s = receive(s, { type: "dropped", gen: 5 });
  assert.equal(inputGen(s), null);
  s = receive(s, frame(5));
  assert.equal(inputGen(s), 5);
});
