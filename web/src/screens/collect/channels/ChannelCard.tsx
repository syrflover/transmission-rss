import { useId, useRef, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";

import { EditIcon, LockIcon } from "../icons";
import type { Channel } from "./api";
import { ChannelEditor } from "./ChannelEditor";
import { btnNeutral } from "./styles";

const chip =
  "inline-flex items-center gap-[5px] rounded-full border border-hairline px-[9px] py-[3px] text-[11.5px] leading-tight font-semibold text-text-secondary";

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="pt-px text-[12.5px] font-semibold text-text-muted">{label}</dt>
      <dd className="m-0 min-w-0 text-[13px] leading-normal max-[480px]:mb-2">{children}</dd>
    </>
  );
}

/** The stored settings of a channel, secrets masked. */
function ChannelDetails({ channel }: { channel: Channel }) {
  const anySecret = channel.query.some((q) => q.secret);
  return (
    <dl className="m-0 grid grid-cols-[130px_minmax(0,1fr)] gap-x-3.5 gap-y-2.5 max-[480px]:grid-cols-1 max-[480px]:gap-y-0.5">
      <Row label="주소">
        <div className="flex min-w-0 flex-col gap-2">
          <span className="flex min-w-0 items-start gap-1.5">
            {anySecret && (
              <>
                <LockIcon className="mt-[3px] size-3 flex-none text-text-muted" />
                <span className="sr-only">비밀 값을 가렸어요.</span>
              </>
            )}
            <span className="min-w-0 font-mono text-xs break-all" data-testid="masked-url">
              {channel.masked_url}
            </span>
          </span>
          {channel.query.length > 0 && (
            <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0" aria-label="주소의 값">
              {channel.query.map((q) => (
                <li key={q.name} className={chip}>
                  {q.secret && <LockIcon className="size-3 flex-none" />}
                  <span>
                    {q.name} {q.secret ? "비밀" : "공개"}
                  </span>
                  {q.secret && !q.filled && <span className="font-normal text-text-muted">입력 전</span>}
                </li>
              ))}
            </ul>
          )}
        </div>
      </Row>
      <Row label="기본 저장 폴더">{channel.base_dir}</Row>
      <Row label="제외 조건">
        {channel.excludes.length > 0 ? (
          <ul className="m-0 flex list-none flex-wrap gap-1.5 p-0">
            {channel.excludes.map((x, i) => (
              <li key={`${i}-${x}`} className={`${chip} font-mono break-all`}>
                {x}
              </li>
            ))}
          </ul>
        ) : (
          "없음"
        )}
      </Row>
      <Row label="지난 회차 검색">
        {channel.past_search ? (
          <span className="font-mono text-xs break-all">{channel.past_search}</span>
        ) : (
          <>
            형식 없음
            <span className="block text-xs text-text-muted">검색할 때 검색어를 직접 넣어요.</span>
          </>
        )}
      </Row>
    </dl>
  );
}

interface ChannelCardProps {
  channel: Channel;
  onEdit: () => void;
  onUpdated: (channel: Channel) => void;
  onDeleted: (channel: Channel, removedRules: number) => void;
}

/** One channel: its settings, or its editor in the same place after 수정. */
export function ChannelCard({ channel, onEdit, onUpdated, onDeleted }: ChannelCardProps) {
  const headingId = useId();
  const [editing, setEditing] = useState(false);
  const editButton = useRef<HTMLButtonElement>(null);

  const close = () => {
    setEditing(false);
    // The button only exists again after this render.
    requestAnimationFrame(() => editButton.current?.focus());
  };

  return (
    <li
      aria-labelledby={headingId}
      className="flex min-w-0 flex-col gap-3.5 rounded-card border border-hairline-soft bg-surface-1 p-[18px] shadow-(--card-shadow) max-[480px]:p-4"
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h3 id={headingId} className="min-w-0 text-[17px] font-bold">
          {channel.host}
        </h3>
        {!editing && (
          <Button
            ref={editButton}
            type="button"
            variant="ghost"
            className={`${btnNeutral} ml-auto`}
            onClick={() => {
              onEdit();
              setEditing(true);
            }}
            aria-label={`${channel.host} 채널 수정`}
          >
            <EditIcon className="size-[15px]" />
            수정
          </Button>
        )}
      </div>
      {editing ? (
        <ChannelEditor
          channel={channel}
          onSaved={(saved) => {
            onUpdated(saved);
            close();
          }}
          onCancel={close}
          onDeleted={onDeleted}
        />
      ) : (
        <ChannelDetails channel={channel} />
      )}
    </li>
  );
}
