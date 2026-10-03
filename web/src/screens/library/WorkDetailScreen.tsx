import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Link, useLocation, useNavigate, useParams, useSearchParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import { forget, forgetPrefix, useCached } from "@/lib/cached";
import { useMediaQuery, PHONE_QUERY } from "@/lib/media";

import { EmptyState, ScreenFrame, usePageTitle } from "../ScreenFrame";
import { btnNeutral } from "../collect/channels/styles";
import { KEYS } from "../collect/cache";
import {
  LIST_PREFIX,
  loadWork,
  workKey,
  type AnissiaLink,
  type SeasonInfo,
  type WorkDetail,
  type WorkSubscription,
} from "./api";
import { CandidateSection } from "./detail/CandidateSection";
import { useCandidates } from "./detail/candidates";
import { CoverDialog } from "./detail/CoverDialog";
import { EpisodeList } from "./detail/EpisodeList";
import { defaultSeason, rowId } from "./detail/model";
import { useEpisodeOrder } from "./detail/prefs";
import { SeasonAnissiaSection } from "./detail/SeasonAnissiaSection";
import { SeasonInfoSection } from "./detail/SeasonInfoSection";
import { UploadSection } from "./detail/UploadSection";
import { SeasonTiles } from "./detail/SeasonTiles";
import { HeadCreators } from "./detail/SubtitleCreators";
import { CollectCard, FilesCard, InfoCard } from "./detail/SideCards";
import { coverOf } from "./model";
import { Cover, FROM_LIBRARY } from "./WorkItem";

const LOAD_FAILED = "작품을 불러오지 못했어요.";

/** How often the page reads a cover that is still being received. */
const COVER_POLL_MS = 3000;

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

  // A cover that follows a season's link keeps its old image until the worker has received the new one, even
  // after the cover view or the link dialog is closed: read the cover now and then until it is there.
  const coverPending = work.data?.cover_pending ?? false;
  const coverUrl = useRef<string | null>(null);
  coverUrl.current = work.data?.cover_url ?? null;
  const updateWork = work.update;
  useEffect(() => {
    if (!coverPending) return;
    const controller = new AbortController();
    const timer = setInterval(() => {
      loadWork(workId, controller.signal).then(
        (fresh) => {
          if (!fresh) return;
          if (fresh.cover_url !== coverUrl.current) forgetPrefix(LIST_PREFIX);
          updateWork((w) => (w ? { ...w, cover_url: fresh.cover_url, cover_pending: fresh.cover_pending } : w));
        },
        () => {},
      );
    }, COVER_POLL_MS);
    return () => {
      clearInterval(timer);
      controller.abort();
    };
  }, [workId, coverPending, updateWork]);

  // A season's info after a change (or after the app linked it) shows at once; the list reads its pages again.
  const infoChanged = (info: SeasonInfo) => {
    work.update((w) => {
      if (!w) return w;
      const seasons = w.seasons.map((s) => (s.number === info.season ? { ...s, info } : s));
      // The head's original title is the work's first season's first entry's.
      const native = info.can_auto ? (info.entries[0]?.native ?? null) : w.native_title;
      return { ...w, seasons, native_title: native };
    });
    forgetPrefix(LIST_PREFIX);
    // The episodes' air days follow the entries, so the page is read again behind what it shows.
    work.reload();
  };

  // A season's Anissia link after a change shows at once. Nothing else of the page follows it.
  const anissiaChanged = (link: AnissiaLink) => {
    work.update((w) => {
      if (!w) return w;
      return { ...w, seasons: w.seasons.map((s) => (s.number === link.season ? { ...s, anissia: link } : s)) };
    });
  };

  // A `다시 받기` of a failed replacement ended: the episode rows show what the worker did.
  const retried = async () => {
    const fresh = await loadWork(workId);
    if (fresh) work.update(fresh);
  };

  // The creator changed (here or in the rule): the page, the rule list and the subscriptions read it again.
  const subscriptionEdited = () => {
    forget(KEYS.subscriptions);
    forget(KEYS.rules);
    work.reload();
  };

  // Subtitle files got a creator: the page reads them again.
  const creatorsChanged = () => {
    work.reload();
  };

  if (work.data) {
    return (
      <Loaded
        work={work.data}
        backLink={backLink}
        onCoverChanged={coverChanged}
        onInfoChanged={infoChanged}
        onAnissiaChanged={anissiaChanged}
        onSubscriptionChanged={subscriptionEdited}
        onCreatorsChanged={creatorsChanged}
        onRetried={retried}
      />
    );
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
  onInfoChanged,
  onAnissiaChanged,
  onSubscriptionChanged,
  onCreatorsChanged,
  onRetried,
}: {
  work: WorkDetail;
  backLink: React.ReactNode;
  onCoverChanged: (coverUrl: string | null) => void;
  onInfoChanged: (info: SeasonInfo) => void;
  onAnissiaChanged: (link: AnissiaLink) => void;
  onSubscriptionChanged: () => void;
  onCreatorsChanged: () => void;
  onRetried: () => Promise<void>;
}) {
  const cover = coverOf(work.name);
  // The Anissia title takes the place of the folder's name once a subscription is connected.
  const title = work.korean_title ?? cover.title;
  const [coverOpen, setCoverOpen] = useState(false);
  usePageTitle(title);
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
  // The subscription of the chosen season: one that collects before a paused or archived one.
  const forSeason = work.subscriptions.filter((s) => s.season === season?.number);
  const subscription: WorkSubscription | undefined =
    forSeason.find((s) => s.rule_state === "active") ??
    forSeason.find((s) => s.rule_state === "paused") ??
    forSeason[0];

  // The chosen season's subtitle candidates: read only for a season linked to an Anissia anime, and shared by the
  // `자막 후보` section and the episode rows, so both name the same candidates.
  const candidates = useCandidates(work.id, season?.number ?? 0, season?.anissia.anime?.anime_no ?? null);
  const subscribedCreator = subscription?.creator ?? null;
  // A creator named for a subtitle file changes the revision candidates too, so both are read again.
  const reloadCandidates = candidates.reload;
  const creatorNamed = () => {
    onCreatorsChanged();
    void reloadCandidates();
  };
  // Its creator can be chosen from the candidates while it receives subtitles and its rule collects.
  const follow =
    subscription && subscription.rule_state === "active" && subscription.subtitles !== "none"
      ? { ruleId: subscription.rule_id, ruleVersion: subscription.rule_version, creator: subscription.creator }
      : null;

  // An address that names the candidates (`자막 구독`'s `제작자 지정`) brings that section into view, once per arrival.
  const wantSection = params.get("section");
  const arrivedSection = useRef<string | null>(null);
  useEffect(() => {
    if (wantSection !== "candidates" || !settled || arrivedSection.current === locationKey) return;
    arrivedSection.current = locationKey;
    const heading = document.getElementById(`candidates-title-${season.number}`);
    if (!heading) return;
    heading.focus({ preventScroll: true });
    heading.scrollIntoView({ block: "start", behavior: "instant" });
  }, [wantSection, settled, season, locationKey]);

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
          <h1 className="text-[26px] leading-[1.28] font-bold tracking-[-0.005em] max-[720px]:text-xl">{title}</h1>
          {work.native_title && (
            <p lang="ja" className="mt-1 text-[14px] leading-snug font-medium break-words text-text-secondary">
              {work.native_title}
            </p>
          )}
          {season && (subscription || hasSubtitles) && (
            <HeadCreators
              key={`creators-${season.number}`}
              workId={work.id}
              season={season}
              subscription={subscription}
              onSubscriptionChanged={onSubscriptionChanged}
              onNamed={creatorNamed}
            />
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
          {season && (
            <SeasonInfoSection
              key={`info-${season.number}`}
              workId={work.id}
              workName={work.name}
              info={season.info}
              seasonCount={work.seasons.length}
              onChanged={onInfoChanged}
            />
          )}
          {season && (
            <SeasonAnissiaSection
              key={`anissia-${season.number}`}
              workId={work.id}
              link={season.anissia}
              seasonCount={work.seasons.length}
              onChanged={onAnissiaChanged}
            />
          )}
          {season && (
            <CandidateSection
              key={`candidates-${season.number}`}
              workId={work.id}
              link={season.anissia}
              seasonCount={work.seasons.length}
              onAnissiaChanged={onAnissiaChanged}
              episodes={season.episodes}
              order={order}
              subscribed={subscribedCreator}
              candidates={candidates}
              follow={follow}
              onFollowChanged={onSubscriptionChanged}
              openSource={params.get("source")}
            />
          )}
          {season && (
            <UploadSection
              key={`upload-${season.number}`}
              workId={work.id}
              season={season.number}
              seasonCount={work.seasons.length}
              candidates={candidates.data ?? null}
            />
          )}
          {season ? (
            <EpisodeList
              key={`episodes-${season.number}`}
              workId={work.id}
              animeNo={season.anissia.anime?.anime_no ?? null}
              onCreatorChanged={creatorNamed}
              season={season}
              seasonCount={work.seasons.length}
              missing={work.missing}
              order={order}
              onOrder={setOrder}
              onRetried={onRetried}
              candidates={
                candidates.data
                  ? { list: candidates.data, workId: work.id, subscribed: subscribedCreator, onMade: candidates.taken }
                  : undefined
              }
            />
          ) : (
            <EmptyState>아직 회차로 읽은 파일이 없어요.</EmptyState>
          )}
        </div>

        <div className={wide ? (work.seasons.length > 1 ? "mt-[29px] flex flex-col gap-3" : "flex flex-col gap-3") : "flex flex-col gap-3"}>
          {season && <InfoCard info={season.info} collapsible={!wide} />}
          <CollectCard work={work} collapsible={!wide} />
          <FilesCard work={work} collapsible={!wide} />
        </div>
      </div>
    </section>
  );
}
