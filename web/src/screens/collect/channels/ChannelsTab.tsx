import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { useCached } from "@/lib/cached";

import { EmptyState } from "../../ScreenFrame";
import { KEYS, channelsChanged } from "../cache";
import { PlusIcon } from "../icons";
import { channelTitle, listChannels, type Channel } from "./api";
import { ChannelCard } from "./ChannelCard";
import { ChannelEditor } from "./ChannelEditor";
import { CollectFolderNote } from "./CollectFolderNote";
import { btnAction, btnNeutral } from "./styles";

/** The 채널 tab: RSS channels with their masked URLs, and adding, editing and deleting them. */
export function ChannelsTab() {
  const list = useCached<Channel[]>(KEYS.channels, listChannels, "채널을 불러오지 못했어요.");
  const [adding, setAdding] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const addButton = useRef<HTMLButtonElement>(null);

  /** The cached list takes the change at once, and the copies other screens keep of it are dropped. */
  const update = (fn: (channels: Channel[]) => Channel[]) => {
    list.update(fn);
    channelsChanged();
  };
  const channels = list.data;

  return (
    <div className="flex flex-col gap-4">
      {/* Tall as the button, so the row does not change height as the button comes and goes. */}
      <div className="flex min-h-9 flex-wrap items-center justify-between gap-x-4 gap-y-2.5 max-[720px]:min-h-10">
        <p className="min-w-[220px] flex-1 text-[13px] text-text-muted">RSS 수집 채널</p>
        {!adding && channels !== undefined && (
          <Button
            ref={addButton}
            type="button"
            variant="ghost"
            className={btnAction}
            onClick={() => {
              setNotice(null);
              setAdding(true);
            }}
          >
            <PlusIcon className="size-[15px]" />
            채널 추가
          </Button>
        )}
      </div>

      <CollectFolderNote className="-mt-1" />

      <p role="status" className="text-[13px] font-semibold text-text-secondary empty:hidden">
        {notice}
      </p>

      {/* A quick answer never shows the loading line. */}
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

      {channels !== undefined && (
        <>
          {adding && (
            <section
              aria-labelledby="new-channel-heading"
              className="flex min-w-0 flex-col gap-3.5 rounded-card border border-hairline-soft bg-surface-1 p-[18px] shadow-(--card-shadow) max-[480px]:p-4"
            >
              <h3 id="new-channel-heading" className="text-[17px] font-bold">
                새 채널
              </h3>
              <ChannelEditor
                channel={null}
                onSaved={(created) => {
                  update((all) => [...all, created]);
                  setAdding(false);
                  setNotice(`${channelTitle(created)} 채널을 추가했어요.`);
                  requestAnimationFrame(() => addButton.current?.focus());
                }}
                onCancel={() => {
                  setAdding(false);
                  requestAnimationFrame(() => addButton.current?.focus());
                }}
              />
            </section>
          )}

          {channels.length === 0 && !adding ? (
            <EmptyState>
              아직 등록한 채널이 없어요. 채널을 추가하면 새 RSS 항목을 규칙과 맞춰 받아요.
            </EmptyState>
          ) : (
            <ul className="m-0 flex list-none flex-col gap-3.5 p-0">
              {channels.map((channel) => (
                <ChannelCard
                  key={channel.id}
                  channel={channel}
                  onEdit={() => setNotice(null)}
                  onUpdated={(saved) => {
                    update((all) => all.map((c) => (c.id === saved.id ? saved : c)));
                    setNotice(`${channelTitle(saved)} 채널을 저장했어요.`);
                  }}
                  onDeleted={(deleted, removedRules) => {
                    update((all) => all.filter((c) => c.id !== deleted.id));
                    setNotice(
                      removedRules > 0
                        ? `${channelTitle(deleted)} 채널과 규칙 ${removedRules}개를 삭제했어요.`
                        : `${channelTitle(deleted)} 채널을 삭제했어요.`,
                    );
                    requestAnimationFrame(() => addButton.current?.focus());
                  }}
                />
              ))}
            </ul>
          )}
        </>
      )}

      <p className="mt-1 text-[12.5px] leading-normal text-text-muted">
        주소의 비밀 값은 화면과 내보내기에서 가려요. 다시 넣으려면 채널의 수정에서 주소의 그 값 자리에 새 값을 넣고 저장해요.
      </p>
    </div>
  );
}
