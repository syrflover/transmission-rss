import type { InputBody, MouseButton } from "./protocol.ts";

type KeyBody = Extract<InputBody, { type: "key" }>;

/** The modifier bit mask of the DevTools input commands: alt 1, ctrl 2, meta 4, shift 8. */
export function modifiersOf(e: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

export interface KeyLike {
  key: string;
  code: string;
  keyCode: number;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
}

const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock", "NumLock", "ScrollLock", "Fn", "OS"]);
/** Keys a keyboard with an input method sends while it composes; their text comes from the input events. */
const COMPOSING_KEYS = new Set(["Process", "Unidentified", "Dead"]);

const isPrintable = (key: string) => [...key].length === 1;

/** Whether the key belongs to the page (its own shortcuts, a browser's), not to the remote page. */
function isShortcut(e: KeyLike): boolean {
  if (!e.ctrlKey && !e.metaKey) return false;
  // AltGr reaches a page as ctrl+alt and still types its character.
  return !(e.ctrlKey && e.altKey && !e.metaKey && isPrintable(e.key));
}

/**
 * The key press to send for a `keydown`, or `null` when the key is not for
 * the remote page: a composing keyboard (its text arrives as input events), a
 * modifier alone, or a shortcut of the browser (Ctrl or Cmd with a key; paste
 * goes through the paste event).
 *
 * A printable key and Enter carry their text so the page types it; the other
 * keys (Backspace, Tab, the arrows) are raw key presses.
 */
export function keyDownBody(e: KeyLike): KeyBody | null {
  if (e.isComposing || e.keyCode === 229 || COMPOSING_KEYS.has(e.key) || MODIFIER_KEYS.has(e.key)) return null;
  if (isShortcut(e)) return null;
  const base = { type: "key" as const, key: e.key, code: e.code, keyCode: e.keyCode, modifiers: modifiersOf(e) };
  if (e.key === "Enter") return { ...base, event: "keyDown", text: "\r" };
  if (isPrintable(e.key)) return { ...base, event: "keyDown", text: e.key };
  return { ...base, event: "rawKeyDown" };
}

/** The key release that goes with {@link keyDownBody}'s press. */
export function keyUpBody(e: KeyLike): KeyBody | null {
  const down = keyDownBody(e);
  return down === null ? null : { type: "key", event: "keyUp", key: down.key, code: down.code, keyCode: down.keyCode, modifiers: down.modifiers };
}

/** A key pressed and released, for the keys an input method reports as input events instead (Enter, Backspace, Delete). */
export function tapKey(key: "Enter" | "Backspace" | "Delete"): InputBody[] {
  const code = { Enter: 13, Backspace: 8, Delete: 46 }[key];
  const press: KeyBody = { type: "key", event: key === "Enter" ? "keyDown" : "rawKeyDown", key, code: key, keyCode: code, modifiers: 0 };
  if (key === "Enter") press.text = "\r";
  return [press, { type: "key", event: "keyUp", key, code: key, keyCode: code, modifiers: 0 }];
}

/**
 * What an input event of the hidden text field means for the remote page:
 * typed text (a keyboard that does not report keys), a line break, or a
 * delete. Text being composed is left until the composition ends
 * ({@link composedText}).
 */
export function inputEventBodies(e: { inputType: string; data: string | null; isComposing: boolean }): InputBody[] {
  if (e.isComposing) return [];
  switch (e.inputType) {
    case "insertText":
      return e.data !== null && e.data !== "" ? [{ type: "text", text: e.data }] : [];
    case "insertLineBreak":
    case "insertParagraph":
      return tapKey("Enter");
    case "deleteContentBackward":
      return tapKey("Backspace");
    case "deleteContentForward":
      return tapKey("Delete");
    default:
      return [];
  }
}

/** The text a finished composition leaves (a Korean syllable, a word of a phone keyboard). */
export function composedText(data: string | null | undefined): InputBody[] {
  return data !== null && data !== undefined && data !== "" ? [{ type: "text", text: data }] : [];
}

/** The mouse button a pointer button number is (`PointerEvent.button`). */
export function buttonName(button: number): MouseButton {
  switch (button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    case 3:
      return "back";
    case 4:
      return "forward";
    default:
      return "none";
  }
}

/** The button a move with `buttons` held drags with (`PointerEvent.buttons`: left 1, right 2, middle 4). */
export function draggedButton(buttons: number): MouseButton {
  if (buttons & 1) return "left";
  if (buttons & 4) return "middle";
  if (buttons & 2) return "right";
  return "none";
}

/** A press that follows the last one closely in time and place is the next click of a multiple click (at most three). */
export function nextClickCount(
  last: { at: number; x: number; y: number; count: number } | null,
  now: { at: number; x: number; y: number },
): number {
  if (last === null) return 1;
  const near = Math.abs(now.x - last.x) <= 5 && Math.abs(now.y - last.y) <= 5;
  return near && now.at - last.at <= 500 ? Math.min(last.count + 1, 3) : 1;
}

/** A wheel's movement in pixels, whatever unit the browser reports it in (`WheelEvent.deltaMode`: pixel 0, line 1, page 2). */
export function wheelPixels(e: { deltaX: number; deltaY: number; deltaMode: number }, pageHeight: number): { dx: number; dy: number } {
  const factor = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? pageHeight : 1;
  return { dx: e.deltaX * factor, dy: e.deltaY * factor };
}
