import { api } from "@/lib/api";
import type { Channel, ChannelDraft, ChannelList, ChannelRemoved, ChannelUpdate } from "@/lib/api-types";

/** What a channel is called on screen: its name, or the host when it has none. */
export function channelTitle(channel: Pick<Channel, "name" | "host">): string {
  return channel.name ?? channel.host;
}

export function listChannels(signal?: AbortSignal): Promise<ChannelList> {
  return api<ChannelList>("/channels", { signal });
}

export function createChannel(draft: ChannelDraft): Promise<Channel> {
  return api<Channel>("/channels", { method: "POST", body: draft });
}

export function saveChannel(id: string, version: number, draft: ChannelDraft): Promise<Channel> {
  const update: ChannelUpdate = { ...draft, version };
  return api<Channel>(`/channels/${encodeURIComponent(id)}`, {
    method: "PUT",
    body: update,
  });
}

/** `rules` is the count the confirmation named; the server refuses if it changed. */
export async function deleteChannel(id: string, version: number, rules: number): Promise<number> {
  const { removed_rules } = await api<ChannelRemoved>(
    `/channels/${encodeURIComponent(id)}?version=${version}&rules=${rules}`,
    { method: "DELETE" },
  );
  return removed_rules;
}
