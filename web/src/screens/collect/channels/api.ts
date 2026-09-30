import { api } from "@/lib/api";

/**
 * The channels API (`src/web/channels_api.rs`). A channel never carries its
 * URL's secret values: only the masked URL, an edit URL with the secret values
 * left blank, and per query name whether it is secret and whether a value is
 * stored.
 */
export interface QueryParam {
  name: string;
  secret: boolean;
  /** A non-empty value is stored (a secret not entered yet is not filled). */
  filled: boolean;
}

export interface Channel {
  id: string;
  position: number;
  /** Sent back on save and delete so the server can refuse stale ones. */
  version: number;
  /** The URL's host, the heading of a channel without a name. */
  host: string;
  /** The name the user gave, `null` when left blank. */
  name: string | null;
  masked_url: string;
  /** The URL to start an edit from: secret values are blank. */
  edit_url: string;
  query: QueryParam[];
  excludes: string[];
  past_search: string | null;
  rule_count: number;
}

/** What the editor sends for both add and save. */
export interface ChannelDraft {
  /** Secret values that stay blank are kept by the server, matched by query name. */
  url: string;
  excludes: string[];
  /** `name -> is secret`; a name left out is secret. */
  secret: Record<string, boolean>;
  past_search: string;
  /** May be blank; the host is shown instead. */
  name: string;
}

/** What a channel is called on screen: its name, or the host when it has none. */
export function channelTitle(channel: Pick<Channel, "name" | "host">): string {
  return channel.name ?? channel.host;
}

export async function listChannels(): Promise<Channel[]> {
  const { channels } = await api<{ channels: Channel[] }>("/channels");
  return channels;
}

export function createChannel(draft: ChannelDraft): Promise<Channel> {
  return api<Channel>("/channels", { method: "POST", body: draft });
}

export function saveChannel(id: string, version: number, draft: ChannelDraft): Promise<Channel> {
  return api<Channel>(`/channels/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: { ...draft, version },
  });
}

/** `rules` is the count the confirmation named; the server refuses if it changed. */
export async function deleteChannel(id: string, version: number, rules: number): Promise<number> {
  const { removed_rules } = await api<{ removed_rules: number }>(
    `/channels/${encodeURIComponent(id)}?version=${version}&rules=${rules}`,
    { method: "DELETE" },
  );
  return removed_rules;
}
