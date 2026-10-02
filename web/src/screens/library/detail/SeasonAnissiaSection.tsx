import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnDanger, btnNeutral, hintClass } from "../../collect/channels/styles";
import { setAnissiaLink, type AnissiaLink } from "../api";
import { ExternalIcon } from "../icons";
import { anissiaStatusText as statusText } from "./model";
import { SeasonAnissiaDialog } from "./SeasonAnissiaDialog";

/**
 * The Anissia anime the season is linked to: its name and a link to its page,
 * and the ways to connect, change or cut the link. While a subscription is
 * connected to the season the anime is the subscription's, and it is changed
 * there, not here.
 */
export function SeasonAnissiaSection({
  workId,
  link,
  seasonCount,
  onChanged,
}: {
  workId: string;
  link: AnissiaLink;
  seasonCount: number;
  /** Called with the link after any change (and with the current one after a conflict). */
  onChanged: (link: AnissiaLink) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { season } = link;

  // Another season's message never stays.
  useEffect(() => setError(null), [season]);

  // A change that came back as the current link: when a subscription holds the
  // season by now, the dialog goes away with the buttons, so say why.
  const changed = (next: AnissiaLink) => {
    if (next.subscription && !link.subscription) {
      setError("그 사이 이 시즌이 구독에 이어졌어요. 연결은 구독을 따라가요.");
    }
    onChanged(next);
  };

  const cut = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      changed(await setAnissiaLink(workId, season, link.version, null));
    } catch (e) {
      if (e instanceof ApiError && e.code === "conflict" && e.current) {
        const current = e.current as AnissiaLink;
        changed(current);
        if (!current.subscription) setError("다른 곳에서 먼저 바꿨어요. 지금 연결을 보여드려요.");
      } else {
        setError(e instanceof ApiError ? e.message : "연결을 끊지 못했어요.");
      }
    } finally {
      setBusy(false);
    }
  };

  const { anime, subscription } = link;
  const title = seasonCount > 1 ? `시즌 ${season} Anissia 연결` : "Anissia 연결";

  return (
    <section
      aria-labelledby={`season-anissia-${season}`}
      className="rounded-card border border-hairline-soft bg-surface-1 px-4 py-3.5 shadow-(--card-shadow) max-[720px]:px-3"
      data-testid="season-anissia"
    >
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
        <h2 id={`season-anissia-${season}`} className="m-0 text-[15px] font-bold">
          {title}
        </h2>
        {anime && (
          <span className="rounded-full bg-surface-2 px-2 py-px text-[11.5px] font-bold text-text-secondary">{statusText(anime.status)}</span>
        )}
        {subscription && (
          <span className="rounded-full border border-ok px-2 py-px text-[11.5px] font-semibold text-ok">구독 중</span>
        )}
        <span className="ml-auto flex flex-wrap items-center gap-x-1.5 gap-y-1">
          {anime && (
            <a
              href={anime.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex min-h-9 items-center gap-1 rounded-full px-2.5 text-[13px] font-semibold text-focus underline-offset-4 hover:underline max-[720px]:min-h-10"
            >
              Anissia
              <ExternalIcon className="size-3.5" />
              <span className="sr-only">에서 {anime.subject} 보기 (새 창)</span>
            </a>
          )}
          {!subscription && (
            <Button type="button" variant="ghost" className={btnNeutral} aria-haspopup="dialog" disabled={busy} onClick={() => setEditing(true)}>
              {anime ? "연결 바꾸기" : "연결하기"}
            </Button>
          )}
          {!subscription && anime && (
            <Button type="button" variant="ghost" className={btnDanger} disabled={busy} onClick={() => void cut()}>
              {busy ? "끊는 중…" : "연결 끊기"}
            </Button>
          )}
        </span>
      </div>

      {anime ? (
        <p className="m-0 mt-2.5 text-[14px] leading-snug font-semibold break-words">
          {anime.subject}
          {anime.original_subject && <span className="mt-0.5 block text-xs font-normal text-text-muted">{anime.original_subject}</span>}
        </p>
      ) : (
        <p className={cn(hintClass, "mt-2.5")}>이 시즌에 이은 Anissia 작품이 없어요.</p>
      )}

      {subscription && (
        <p className={cn(hintClass, "mt-2.5")} data-testid="season-anissia-subscribed">
          이 시즌은 구독{subscription.subject ? ` ‘${subscription.subject}’` : ""}을 따라가고 있어서 연결을 바꿀 수 없어요. 다른 작품에 잇고 싶으면 그 구독을 먼저 삭제해 주세요. 삭제해도
          시즌의 연결은 남고, 그 뒤 여기서 바꾸거나 끊을 수 있어요.{" "}
          <Link to={`/collect/rules?rule=${encodeURIComponent(subscription.rule_id)}`} className="text-focus underline underline-offset-2">
            구독 열기
          </Link>
        </p>
      )}
      {error && (
        <p role="alert" className="mt-2.5 text-[13px] leading-relaxed font-semibold text-urgent">
          {error}
        </p>
      )}

      {!subscription && <SeasonAnissiaDialog workId={workId} link={link} open={editing} onOpenChange={setEditing} onChanged={changed} />}
    </section>
  );
}
