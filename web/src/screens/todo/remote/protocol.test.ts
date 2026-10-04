import assert from "node:assert/strict";
import { test } from "node:test";

import { encode, MAX_MESSAGE_BYTES, parseServerMessage, socketUrl, textChunks } from "./protocol.ts";

test("the server's messages are read by their shape", () => {
  assert.deepEqual(parseServerMessage('{"type":"viewport","gen":2,"width":402,"height":666,"dpr":3,"touch":true}'), {
    type: "viewport",
    gen: 2,
    width: 402,
    height: 666,
    dpr: 3,
    touch: true,
  });
  assert.deepEqual(parseServerMessage('{"type":"frame","gen":2,"width":402,"height":666,"data":"/9j/"}'), {
    type: "frame",
    gen: 2,
    width: 402,
    height: 666,
    data: "/9j/",
  });
  assert.deepEqual(parseServerMessage('{"type":"dropped","gen":3}'), { type: "dropped", gen: 3 });
  assert.deepEqual(parseServerMessage('{"type":"nav","back":true,"forward":false,"host":"blog.example.org"}'), {
    type: "nav",
    back: true,
    forward: false,
    host: "blog.example.org",
  });
  assert.deepEqual(parseServerMessage('{"type":"nav","back":false,"forward":false,"host":null}'), {
    type: "nav",
    back: false,
    forward: false,
    host: null,
  });
  assert.deepEqual(
    parseServerMessage(
      '{"type":"tabs","tabs":[{"id":"T1","title":"","host":"blog.example.org","shown":true,"closable":false},{"id":"P1","title":"글","host":null,"shown":false,"closable":true}]}',
    ),
    {
      type: "tabs",
      tabs: [
        { id: "T1", title: "", host: "blog.example.org", shown: true, closable: false },
        { id: "P1", title: "글", host: null, shown: false, closable: true },
      ],
    },
  );
  assert.deepEqual(parseServerMessage('{"type":"tabs","tabs":[]}'), { type: "tabs", tabs: [] });
  assert.deepEqual(parseServerMessage('{"type":"page","responding":false}'), { type: "page", responding: false });
  assert.deepEqual(parseServerMessage('{"type":"page","responding":true}'), { type: "page", responding: true });
  for (const reason of ["browser", "run", "unreachable", "stuck", "replaced"]) {
    assert.deepEqual(parseServerMessage(`{"type":"ended","reason":"${reason}"}`), { type: "ended", reason });
  }
});

test("a message that is not the protocol's is ignored", () => {
  for (const text of [
    "not json",
    "null",
    "7",
    '{"type":"frame","gen":1,"width":0,"height":10,"data":""}',
    '{"type":"frame","gen":"1","width":10,"height":10,"data":""}',
    '{"type":"viewport","gen":1,"width":1,"height":1,"dpr":1}',
    '{"type":"ended","reason":"gone"}',
    '{"type":"nav","back":"yes","forward":false,"host":null}',
    '{"type":"nav","back":true,"forward":false}',
    '{"type":"tabs","tabs":"none"}',
    '{"type":"tabs","tabs":[{"id":"","title":"","host":null,"shown":true,"closable":true}]}',
    '{"type":"tabs","tabs":[{"id":"T1","title":"","host":null,"shown":true}]}',
    '{"type":"page","responding":"no"}',
    '{"type":"page"}',
    '{"type":"unknown"}',
  ]) {
    assert.equal(parseServerMessage(text), null, text);
  }
});

test("inputs are encoded as the server's protocol names them", () => {
  const text = encode({ type: "mouse", gen: 3, event: "mousePressed", x: 10.5, y: 20, button: "left", buttons: 1, clickCount: 1, modifiers: 0 });
  assert.deepEqual(JSON.parse(text ?? ""), {
    type: "mouse",
    gen: 3,
    event: "mousePressed",
    x: 10.5,
    y: 20,
    button: "left",
    buttons: 1,
    clickCount: 1,
    modifiers: 0,
  });
  assert.deepEqual(JSON.parse(encode({ type: "reload" }) ?? ""), { type: "reload" });
  assert.deepEqual(JSON.parse(encode({ type: "back" }) ?? ""), { type: "back" });
  assert.deepEqual(JSON.parse(encode({ type: "forward" }) ?? ""), { type: "forward" });
  assert.deepEqual(JSON.parse(encode({ type: "viewport", width: 402, height: 666, dpr: 3, touch: true }) ?? ""), {
    type: "viewport",
    width: 402,
    height: 666,
    dpr: 3,
    touch: true,
  });
});

test("a message over the socket's cap is not encoded", () => {
  assert.equal(encode({ type: "text", gen: 1, text: "가".repeat(MAX_MESSAGE_BYTES / 3 + 1) }), null);
  assert.notEqual(encode({ type: "text", gen: 1, text: "가".repeat(1000) }), null);
});

test("long text goes in pieces that never cut a character", () => {
  assert.deepEqual(textChunks("abcde", 2), ["ab", "cd", "e"]);
  assert.deepEqual(textChunks("", 2), []);
  const emoji = "😀😀😀";
  assert.deepEqual(textChunks(emoji, 2), ["😀😀", "😀"]);
  assert.equal(textChunks("가".repeat(2500)).every((c) => [...c].length <= 1000), true);
});

test("the socket is on the page's own origin, secure when the page is", () => {
  assert.equal(
    socketUrl({ protocol: "http:", host: "trss.lan:8080" }, "j 1", "run/1", 1700000000123),
    "ws://trss.lan:8080/api/subtitle-jobs/j%201/screen/socket?run=run%2F1&bound=1700000000123",
  );
  assert.equal(socketUrl({ protocol: "https:", host: "trss.example" }, "j1", "r1", null), "wss://trss.example/api/subtitle-jobs/j1/screen/socket?run=r1");
});
