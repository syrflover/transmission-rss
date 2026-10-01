import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Link, useLocation, useNavigate, useParams, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { forgetPrefix, useCached } from "@/lib/cached";
import { useMediaQuery, PHONE_QUERY } from "@/lib/media";

import { EmptyState, ScreenFrame, usePageTitle } from "../ScreenFrame";
import { btnNeutral } from "../collect/channels/styles";
import { LIST_PREFIX, loadWork, workKey, type WorkDetail } from "./api";
import { CoverDialog } from "./detail/CoverDialog";
import { EpisodeList } from "./detail/EpisodeList";
import { defaultSeason, rowId } from "./detail/model";
import { useEpisodeOrder } from "./detail/prefs";
import { SeasonTiles } from "./detail/SeasonTiles";
import { CollectCard, FilesCard } from "./detail/SideCards";
import { coverOf } from "./model";
import { Cover, FROM_LIBRARY } from "./WorkItem";

const LOAD_FAILED = "작품을 불러오지 못했어요.";

/** The two-column layout starts here; below it the cards fold under the episode list. */
const WIDE_QUERY = "(min-width: 1100px)";

/**
 * A work's page: its head, the seasons, the episodes of the chosen season with
 * whether a video and a subtitle file are there, and the cards for how the work
 * is collected and where its files are. The way back is the browser's own back
 * step when the work was opened from the library, so the list is found as it was
 * (search, filter, scroll position).
 */
export function WorkDetailScreen() {
  const { workId = "" } = useParams();
  // One page per work: choosing a season, an opened sheet or row never carries over to another work.
  return <WorkPage key={workId} workId={workId} />;
}

function WorkPage({ workId }: { workId: string }) {
  const location = useLocation();
  const navigate = useNavigate();
  const work = useCached<WorkDetail | null>(workKey(workId), (signal) => loadWork(workId, signal), LOAD_FAILED);
  const fromLibrary = (location.state as typeof FROM_LIBRARY | null)?.from === FROM_LIBRARY.from;

  const back = (event: MouseEvent<HTMLAnchorElement>) => {
    if (!fromLibrary) return;
    event.preventDefault();
    navigate(-1);
  };

  const backLink = (
    <Link
      to="/library"
      onClick={back}
      className="inline-flex min-h-9 items-center text-[13px] font-semibold text-text-secondary underline underline-offset-4 hover:text-text-primary"
    >
      라이브러리로
    </Link>
  );

  // A new cover shows in the head at once; the list reads its pages again.
  const coverChanged = (coverUrl: string | null) => {
    if (work.data && work.data.cover_url !== coverUrl) {
      work.update((w) => (w ? { ...w, cover_url: coverUrl } : w));
      forgetPrefix(LIST_PREFIX);
    }
  };

  if (work.data) {
    return <Loaded work={work.data} backLink={backLink} onCoverChanged={coverChanged} />;
  }
  return (
    <ScreenFrame title="작품">
      <div className="flex flex-col items-start gap-3">
        {work.data === null ? (
          <EmptyState>이 작품을 찾지 못했어요. 폴더가 감시 폴더에서 빠졌을 수 있어요.</EmptyState>
        ) : work.error !== null ? (
          <>
            <p role="alert" className="text-[13px] font-semibold text-urgent">
              {work.error}
            </p>
            <Button type="button" variant="ghost" className={btnNeutral} onClick={work.reload}>
              재시도
            </Button>
          </>
        ) : work.slow ? (
          <p className="text-[13px] text-text-muted">작품을 불러오는 중이에요.</p>
        ) : null}
        {backLink}
      </div>
    </ScreenFrame>
  );
}

function Loaded({
  work,
  backLink,
  onCoverChanged,
}: {
  work: WorkDetail;
  backLink: React.ReactNode;
  onCoverChanged: (coverUrl: string | null) => void;
}) {
  const cover = coverOf(work.name);
  const [coverOpen, setCoverOpen] = useState(false);
  usePageTitle(cover.title);
  const phone = useMediaQuery(PHONE_QUERY);
  const wide = useMediaQuery(WIDE_QUERY);
  const [order, setOrder] = useEpisodeOrder();
  const [params] = useSearchParams();
  const { key: locationKey } = useLocation();

  const [picked, setPicked] = useState<number | null>(null);
  const seasonOf = (number: number | null) => work.seasons.find((s) => s.number === number);
  const wantSeason = params.get("season");
  const wantEpisode = params.get("episode");
  const linkedSeason = wantSeason !== null && /^\d+$/.test(wantSeason) ? Number(wantSeason) : null;
  const season = seasonOf(picked) ?? seasonOf(defaultSeason(work.seasons));

  // An address that names a season (and an episode) chooses it, again each time the address changes.
  const hasLinked = seasonOf(linkedSeason) !== undefined;
  useEffect(() => {
    if (linkedSeason !== null && hasLinked) setPicked(linkedSeason);
  }, [linkedSeason, hasLinked, locationKey]);

  // ...and brings the episode's row into view and takes focus there, once per arrival.
  const arrived = useRef<string | null>(null);
  const settled = season !== undefined && (linkedSeason === null || season.number === linkedSeason);
  useEffect(() => {
    if (wantEpisode === null || !settled || arrived.current === locationKey) return;
    arrived.current = locationKey;
    const row = document.getElementById(rowId(season.number, wantEpisode));
    if (!row) return;
    row.focus({ preventScroll: true });
    row.scrollIntoView({ block: "center", behavior: "instant" });
  }, [wantEpisode, settled, season, locationKey]);

  const hasSubtitles = season?.episodes.some((e) => e.subtitle.length > 0) ?? false;

  return (
    <section className="mx-auto pb-4 max-[1099px]:max-w-[900px]">
      <div className="pt-4">{backLink}</div>

      <div className="flex items-start gap-[22px] pt-3 pb-5 max-[720px]:gap-3.5">
        <button
          type="button"
          aria-label="표지 크게 보기와 바꾸기"
          aria-haspopup="dialog"
          onClick={() => setCoverOpen(true)}
          className="flex-none cursor-zoom-in rounded-lg focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-focus"
        >
          <Cover
            work={cover}
            imageUrl={work.cover_url}
            className="w-28 aspect-[2/3] max-[720px]:w-20"
            letterClass="text-5xl max-[720px]:text-3xl"
          />
        </button>
        <CoverDialog
          workId={work.id}
          work={cover}
          coverUrl={work.cover_url}
          open={coverOpen}
          onOpenChange={setCoverOpen}
          onChanged={onCoverChanged}
        />
        <div className="min-w-0 flex-1">
          <h1 className="text-[26px] leading-[1.28] font-bold tracking-[-0.005em] max-[720px]:text-xl">{cover.title}</h1>
          {hasSubtitles && (
            <p className="mt-3 flex flex-wrap items-baseline gap-x-2 gap-y-1 text-[13px]">
              <span className="font-semibold text-text-muted">자막 제작자</span>
              <span className="font-medium text-text-secondary">제작자 알 수 없음</span>
            </p>
          )}
        </div>
      </div>

      {work.missing && (
        <p className="mb-5 text-[13px] leading-relaxed text-text-secondary">
          <span className="font-semibold text-focus">폴더 없음</span> 작품 폴더를 찾지 못했어요. 지워졌거나 옮겨졌거나 마운트가 풀렸을
          수 있어요. 아래는 마지막으로 확인한 기록이에요.
        </p>
      )}

      <div className={wide ? "grid grid-cols-[minmax(0,1fr)_380px] items-start gap-8" : "flex flex-col gap-7"}>
        <div className="flex min-w-0 flex-col gap-7">
          {work.seasons.length > 1 && season && (
            <section aria-labelledby="seasons-title">
              <h2 id="seasons-title" className="mb-2.5 flex h-[19px] items-center text-[13px] font-bold text-text-muted">
                시즌
              </h2>
              <SeasonTiles
                seasons={work.seasons}
                missing={work.missing}
                selected={season.number}
                onSelect={setPicked}
                phone={phone}
              />
            </section>
          )}
          {season ? (
            <EpisodeList
              key={season.number}
              season={season}
              seasonCount={work.seasons.length}
              missing={work.missing}
              order={order}
              onOrder={setOrder}
            />
          ) : (
            <EmptyState>아직 회차로 읽은 파일이 없어요.</EmptyState>
          )}
        </div>

        <div className={wide ? (work.seasons.length > 1 ? "mt-[29px] flex flex-col gap-3" : "flex flex-col gap-3") : "flex flex-col gap-3"}>
          <CollectCard work={work} collapsible={!wide} />
          <FilesCard work={work} collapsible={!wide} />
        </div>
      </div>
    </section>
  );
}
