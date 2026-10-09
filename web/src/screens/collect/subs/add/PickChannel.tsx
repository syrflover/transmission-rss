import { useId } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { KEYS } from "../../cache";
import { channelTitle, listChannels, type Channel, type ChannelList } from "../../channels/api";
import { btnNeutral } from "../../channels/styles";
import type { ScheduleEntry } from "../api";
import { optionClass } from "./draft";

/** Step 2: the RSS channel that carries the anime's releases. */
export function PickChannel({
  anime,
  selected,
  onPick,
}: {
  anime: ScheduleEntry;
  selected: Channel | null;
  onPick: (channel: Channel) => void;
}) {
  const uid = useId();
  const list = useCached<ChannelList>(KEYS.channels, listChannels, "채널을 불러오지 못했어요.");
  const channels = list.data?.channels;
  const taken = new Set(anime.subscribed_rules.map((r) => r.channel_id));

  return (
    <section aria-labelledby={`${uid}-h`} className="flex min-w-0 flex-col gap-3.5">
      <div className="flex min-w-0 flex-col gap-1">
        <h3 id={`${uid}-h`} className="text-[15px] font-bold">
          받을 채널을 골라요
        </h3>
        <p className="min-w-0 text-[13px] break-words text-text-muted">{anime.subject}</p>
      </div>

      {channels === undefined && list.error === null && list.slow && (
        <p className="text-[13px] text-text-muted">채널을 불러오는 중이에요.</p>
      )}

      {channels === undefined && list.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {list.error}
          </p>
          <Button type="button" variant="ghost" className={btnNeutral} onClick={list.reload}>
            재시도
          </Button>
        </div>
      )}

      {channels !== undefined && channels.length === 0 && (
        <p className="text-[13px] leading-normal text-text-secondary">
          아직 등록한 채널이 없어요. <Link to="/collect/channels" className="font-semibold text-focus underline underline-offset-4">채널 탭</Link>에서 RSS 채널을 추가하면 여기서 고를 수 있어요.
        </p>
      )}

      {channels !== undefined && channels.length > 0 && (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {channels.map((channel) => {
            const already = taken.has(channel.id);
            const on = selected?.id === channel.id;
            return (
              <li key={channel.id}>
                <button
                  type="button"
                  aria-pressed={on}
                  disabled={already}
                  onClick={() => onPick(channel)}
                  className={cn(optionClass, on ? "border-focus" : "border-hairline-soft")}
                >
                  <span className="min-w-0 text-[14.5px] font-semibold break-all">{channelTitle(channel)}</span>
                  <span className="min-w-0 font-mono text-xs break-all text-text-muted">{channel.masked_url}</span>
                  {already && (
                    <span className="text-xs font-semibold text-ok">이 채널에서 이미 구독 중이에요.</span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
