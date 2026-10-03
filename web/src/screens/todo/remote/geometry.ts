import type { Viewport } from "./protocol.ts";

/** The bounds the server takes for a device's size and pixel ratio. */
export const MIN_SIDE = 200;
export const MAX_SIDE = 4096;
export const MIN_DPR = 0.5;
export const MAX_DPR = 4;

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/**
 * What the device reports for the remote page: the width of the area that
 * shows it, and a height of 60% of `deviceHeight` (at least 360 and at most
 * 720 CSS pixels). A touch screen passes its screen's height, which a virtual
 * keyboard does not shorten, so raising the keyboard does not lay the remote
 * page out again; a computer passes its window's. Either way the height's
 * middle, where the server scrolls the check box to, lies in the first screen.
 */
export function deviceViewport(device: {
  areaWidth: number;
  deviceHeight: number;
  dpr: number;
  touch: boolean;
}): Viewport {
  const dpr = Number.isFinite(device.dpr) ? device.dpr : 1;
  return {
    width: Math.round(clamp(Math.floor(device.areaWidth), MIN_SIDE, MAX_SIDE)),
    height: Math.round(clamp(Math.round(device.deviceHeight * 0.6), 360, 720)),
    dpr: Math.round(clamp(dpr, MIN_DPR, MAX_DPR) * 1000) / 1000,
    touch: device.touch,
  };
}

export function sameViewport(a: Viewport, b: Viewport): boolean {
  return a.width === b.width && a.height === b.height && a.dpr === b.dpr && a.touch === b.touch;
}

/** A frame as the remote page lays out: its size in the page's CSS pixels. */
export interface FrameSize {
  width: number;
  height: number;
}

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * The point of the remote page that a pointer over the shown frame is on, in
 * the page's CSS pixels. The frame is drawn scaled to the box (its size can be
 * another device's), so each axis is scaled by its own ratio. A point outside
 * the box (a drag that left it) is held just inside the edge (half a pixel, as
 * the browser rounds a frame's size, so the server does not take it for off
 * the screen); an empty box has no point.
 */
export function toRemote(
  point: { clientX: number; clientY: number },
  box: Box,
  frame: FrameSize,
): { x: number; y: number } | null {
  if (box.width <= 0 || box.height <= 0 || frame.width <= 0 || frame.height <= 0) return null;
  const x = ((point.clientX - box.left) * frame.width) / box.width;
  const y = ((point.clientY - box.top) * frame.height) / box.height;
  return { x: clamp(x, 0, frame.width - 0.5), y: clamp(y, 0, frame.height - 0.5) };
}
