import assert from "node:assert/strict";
import { test } from "node:test";

import {
  buttonName,
  composedText,
  draggedButton,
  inputEventBodies,
  keyDownBody,
  keyUpBody,
  modifiersOf,
  nextClickCount,
  tapKey,
  wheelPixels,
  type KeyLike,
} from "./keys.ts";

const key = (key: string, code: string, keyCode: number, more: Partial<KeyLike> = {}): KeyLike => ({
  key,
  code,
  keyCode,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...more,
});

test("modifiers are the DevTools bit mask", () => {
  assert.equal(modifiersOf({ altKey: true, ctrlKey: false, metaKey: false, shiftKey: false }), 1);
  assert.equal(modifiersOf({ altKey: false, ctrlKey: true, metaKey: false, shiftKey: true }), 10);
  assert.equal(modifiersOf({ altKey: true, ctrlKey: true, metaKey: true, shiftKey: true }), 15);
});

test("a printable key types its text, Enter types a return", () => {
  assert.deepEqual(keyDownBody(key("a", "KeyA", 65)), { type: "key", event: "keyDown", key: "a", code: "KeyA", keyCode: 65, modifiers: 0, text: "a" });
  assert.equal(keyDownBody(key("A", "KeyA", 65, { shiftKey: true }))?.text, "A");
  assert.equal(keyDownBody(key(" ", "Space", 32))?.text, " ");
  assert.deepEqual(keyDownBody(key("Enter", "Enter", 13)), { type: "key", event: "keyDown", key: "Enter", code: "Enter", keyCode: 13, modifiers: 0, text: "\r" });
});

test("Backspace, Tab, the arrows and Escape are raw key presses without text", () => {
  for (const [k, code, kc] of [["Backspace", "Backspace", 8], ["Tab", "Tab", 9], ["ArrowLeft", "ArrowLeft", 37], ["Escape", "Escape", 27], ["Delete", "Delete", 46]] as const) {
    const body = keyDownBody(key(k, code, kc));
    assert.equal(body?.event, "rawKeyDown", k);
    assert.equal(body?.text, undefined, k);
    assert.equal(body?.keyCode, kc, k);
  }
});

test("a key release repeats the press without text", () => {
  assert.deepEqual(keyUpBody(key("a", "KeyA", 65)), { type: "key", event: "keyUp", key: "a", code: "KeyA", keyCode: 65, modifiers: 0 });
  assert.equal(keyUpBody(key("Shift", "ShiftLeft", 16, { shiftKey: true })), null);
});

test("modifier keys alone, composing keys and the browser's shortcuts are not sent as keys", () => {
  assert.equal(keyDownBody(key("Shift", "ShiftLeft", 16, { shiftKey: true })), null);
  assert.equal(keyDownBody(key("Process", "KeyK", 229)), null, "a Korean input method");
  assert.equal(keyDownBody(key("Unidentified", "", 229)), null, "a phone keyboard");
  assert.equal(keyDownBody(key("a", "KeyA", 65, { isComposing: true })), null);
  assert.equal(keyDownBody(key("r", "KeyR", 82, { ctrlKey: true })), null, "reload the app's page, not the remote one");
  assert.equal(keyDownBody(key("v", "KeyV", 86, { metaKey: true })), null);
});

test("AltGr still types its character", () => {
  assert.equal(keyDownBody(key("@", "KeyQ", 81, { ctrlKey: true, altKey: true }))?.text, "@");
});

test("an input method's text goes as text, once its composition ends", () => {
  assert.deepEqual(composedText("한"), [{ type: "text", text: "한" }]);
  assert.deepEqual(composedText(""), []);
  assert.deepEqual(composedText(null), []);
  assert.deepEqual(inputEventBodies({ inputType: "insertText", data: "ab", isComposing: false }), [{ type: "text", text: "ab" }]);
  assert.deepEqual(inputEventBodies({ inputType: "insertCompositionText", data: "ㅎ", isComposing: true }), []);
  assert.deepEqual(inputEventBodies({ inputType: "insertText", data: "ㅎ", isComposing: true }), []);
});

test("Enter and the deletes of a phone keyboard are key presses", () => {
  const names = (type: string) => inputEventBodies({ inputType: type, data: null, isComposing: false }).map((b) => (b.type === "key" ? `${b.event}:${b.key}` : b.type));
  assert.deepEqual(names("insertLineBreak"), ["keyDown:Enter", "keyUp:Enter"]);
  assert.deepEqual(names("deleteContentBackward"), ["rawKeyDown:Backspace", "keyUp:Backspace"]);
  assert.deepEqual(names("deleteContentForward"), ["rawKeyDown:Delete", "keyUp:Delete"]);
  assert.deepEqual(names("historyUndo"), []);
  assert.equal(tapKey("Enter")[0].type, "key");
});

test("pointer buttons are named for the mouse input", () => {
  assert.deepEqual([0, 1, 2, 3, 4, -1].map(buttonName), ["left", "middle", "right", "back", "forward", "none"]);
  assert.deepEqual([1, 2, 4, 3, 0].map(draggedButton), ["left", "right", "middle", "left", "none"]);
});

test("presses close in time and place make a double and a triple click, and no more", () => {
  const at = (t: number, x = 10, y = 10) => ({ at: t, x, y });
  assert.equal(nextClickCount(null, at(0)), 1);
  const first = { at: 0, x: 10, y: 10, count: 1 };
  assert.equal(nextClickCount(first, at(300)), 2);
  assert.equal(nextClickCount({ ...first, count: 2 }, at(300)), 3);
  assert.equal(nextClickCount({ ...first, count: 3 }, at(300)), 3);
  assert.equal(nextClickCount(first, at(900)), 1, "too late");
  assert.equal(nextClickCount(first, at(300, 40, 10)), 1, "too far");
});

test("a wheel's lines and pages become pixels", () => {
  assert.deepEqual(wheelPixels({ deltaX: 0, deltaY: 100, deltaMode: 0 }, 600), { dx: 0, dy: 100 });
  assert.deepEqual(wheelPixels({ deltaX: 1, deltaY: 3, deltaMode: 1 }, 600), { dx: 16, dy: 48 });
  assert.deepEqual(wheelPixels({ deltaX: 0, deltaY: 1, deltaMode: 2 }, 600), { dx: 0, dy: 600 });
});
