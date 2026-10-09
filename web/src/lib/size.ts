/**
 * Bytes as `22 KB`, `1.4 MB`, `1.5 GB`; a size under 1 KB is `512 B`. A step under 10 shows one decimal (`.0`
 * dropped), a larger one is rounded. Imports nothing, so the pure helpers that use it and their tests run without
 * the app.
 */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${step(kb)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${step(mb)} MB`;
  return `${step(mb / 1024)} GB`;
}

function step(n: number): string {
  return n < 10 ? n.toFixed(1).replace(/\.0$/, "") : String(Math.round(n));
}
