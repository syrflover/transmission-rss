/**
 * Reading the query part of a URL the user is typing. The server decides what
 * is stored (`src/web/channels_api.rs`); this only mirrors how it splits names
 * so the screen can show one toggle per name.
 */

export interface QueryPair {
  name: string;
  value: string;
}

function decode(part: string): string {
  const spaced = part.replace(/\+/g, " ");
  try {
    return decodeURIComponent(spaced);
  } catch {
    return spaced;
  }
}

/** The `name=value` pairs of the URL's query in order, names percent-decoded. */
export function queryPairs(url: string): QueryPair[] {
  const withoutFragment = url.split("#")[0];
  const at = withoutFragment.indexOf("?");
  if (at < 0) return [];
  const pairs: QueryPair[] = [];
  for (const segment of withoutFragment.slice(at + 1).split("&")) {
    if (segment === "") continue;
    const eq = segment.indexOf("=");
    pairs.push(
      eq < 0
        ? { name: decode(segment), value: "" }
        : { name: decode(segment.slice(0, eq)), value: segment.slice(eq + 1) },
    );
  }
  return pairs;
}

/** Distinct query names in order of first appearance. */
export function queryNames(url: string): string[] {
  const names: string[] = [];
  for (const { name } of queryPairs(url)) {
    if (!names.includes(name)) names.push(name);
  }
  return names;
}
