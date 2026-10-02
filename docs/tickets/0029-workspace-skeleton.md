# 0029 크레이트 지도를 정하고 workspace의 바이너리와 공통 기반을 나눠요

- 상태: 진행 중 (workspace 전환과 `trss-core`·`trss-web`·`trss-worker` 분리, 크레이트 지도는 끝났어요. 로컬 compose 실행만 남았고 0030이 끝난 뒤 함께 확인해요. 아래 "결과")
- 출처: [기능별 크레이트 ADR](../adr/0011-feature-crate-workspace.md), [공통 라이브러리의 모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: 없음 (0027·0028처럼 진행 중인 변경이 있으면 먼저 끝내요. 파일을 대량으로 옮기는 동안 다른 변경과 충돌하기 때문이에요)

## 작업

동작을 바꾸지 않고 저장소를 Cargo workspace로 바꾸는 첫 단계예요. 넓은 이동이라 확장·축소 순서로 진행해요.

1. 크레이트 지도의 모양은 [ADR 0011](../adr/0011-feature-crate-workspace.md)에 정했어요(사용자 결정, 2026-10-02). 순환하는 모듈을 한 묶음으로 두는 크레이트 구성, 마이그레이션 SQL과 순서는 모두 `trss-core`, 폴더는 `crates/<이름>/`이에요.
   이 티켓에서는 모듈마다 갈 크레이트를 표로 정하고 결과에 남겨요. 그 원칙을 바꿔야 하는 배치(예: 묶음을 더 나누거나 합침)가 나오면 그때만 사용자에게 물어요.
   2026-10-02에 `crate::` 참조로 센 역방향 의존이에요(시험 코드 제외). 이동 때 다시 확인해요.
   - 기능 → worker: `anissia`·`artwork`·`seasons`(`Clock`·`CycleLock`), `archive_suggestions`·`subscriptions`·`store/channels`(`plan::ChannelPlan`), `past_search`(`feed`·`plan`·`revisions`), `episode_offset`(`revisions`·`season_link`), `store/history`·`store/revisions`·`transmission`(`revisions`), `store/channels`(`commands::episode_undo`), `artwork`(규칙 보관의 `rename_noreplace`), `store/library`(문서 링크만: `commands::rule_archive`·`live`)
   - 기능 → 웹: `schedule`(`web::schedule_api`)
   - 묶음 사이의 순환: `import` ↔ `store/channels`(`import_subscriptions`)
2. 지금의 라이브러리를 workspace의 한 크레이트로 두고, `trss-web`·`trss-worker`를 각자의 바이너리 크레이트로 옮겨요. `src/web`·`src/worker`처럼 진입부에만 쓰이는 모듈은 해당 바이너리 크레이트로 가요.
3. 공통 기반 크레이트를 떼어 내고, 지금의 라이브러리가 그것을 쓰게 해요.

Docker 이미지 빌드, 테스트 위치(`tests/`), 배포 스크립트가 새 구성으로 동작해야 해요.

## 완료 기준

| 확인 | 기대 결과 |
| --- | --- |
| 크레이트 지도 | 크레이트마다 책임·의존하는 크레이트·옮길 모듈과 역방향 의존 목록이 이 티켓의 결과에 있고, ADR 0011의 원칙과 어긋나는 배치는 사용자가 정했어요. |
| `cargo build`·`cargo test`·`cargo clippy` (workspace 전체) | 이동 전과 같은 테스트가 모두 통과하고, 빠진 테스트가 없어요(이동 전후 테스트 수를 비교해요). |
| 공통 기반 크레이트 | 웹·worker·기능 코드를 참조하지 않아요(크레이트 의존 목록으로 확인). |
| 앱 이미지 빌드와 로컬 compose 실행 | 두 바이너리가 이전과 같이 시작해 같은 DB를 읽고, worker의 수집 주기와 웹 화면이 동작해요. |
| 동작 | 마이그레이션 순서·DB 스키마·API 응답이 바뀌지 않아요. |

## 결과

### 한 일

`cargo test` 1,454개(라이브러리 1,078 + `tests/` 20개 파일 376)가 `cargo test --workspace`에서도 1,454개예요. 코드 변경은 커밋 다섯 개로 나눴어요.

1. workspace 전환: 루트 `Cargo.toml`은 `members = ["crates/*"]`인 workspace이고, 기존 패키지는 `crates/trss-legacy`(임시 이름)로 옮겼어요. `Dockerfile`은 `COPY crates ./crates`로 바뀌었어요.
2. `trss-core` 추출: 데이터베이스(`Db`·마이그레이션 실행기·**SQL 30개 전부**와 순서, 7번 코드 마이그레이션 `fold_base_dirs`), 앱 설정 저장소, 폴더 산술(`folders`), `Millis`·`Clock`·`system_clock`, `CycleLock`·`lock_path_for`, `USER_AGENT`예요. SQL은 `crates/trss-core/migrations/<기능>/`로 `git mv`했고 내용이 같아요(`git diff -M`이 30개 모두 R100). 기능이 worker를 거쳐 `Clock`·`CycleLock`을 쓰던 곳은 `trss_core`를 직접 쓰도록 고쳤어요. `db`·`settings`·`folders`·`Millis`는 옛 경로의 재수출로 남겼고, 0030이 호출부를 새 경로로 바꾸며 없애요.
3. `trss-worker` 분리: 라이브러리와 바이너리를 함께 둔 크레이트로, `Worker`·`WorkerEnv`·`run_cycle`·명령 분배와 `trss-worker` 바이너리가 들어 있어요. 기능과 웹이 참조하는 worker 코드는 지금 `trss-legacy::worker`에 남겼어요(`plan`·`feed`·`revisions`·`season_link`·`watch`·`live`·`heartbeat`·`offsets`, 명령 종류별 모듈, `CycleContext`). `CycleContext`는 `worker/context.rs`로 떼어 냈어요.
4. `trss-web` 분리: `src/web` 전체와 `trss-web` 바이너리예요. 웹의 단위 시험이 쓰는 가짜 AniList·Anissia 서버, 시험용 이미지, 옛 스키마 만들기는 `trss-legacy`·`trss-core`의 `test-support` 기능으로 열었어요.
5. `cargo fmt`로 옮긴 코드의 import를 정리했어요.

`tests/`의 통합 시험 20개 파일과 가짜 Transmission 도구(`common`)는 모두 `crates/trss-worker/tests/`로 옮겼어요. 시험 도구가 worker·웹 라우터·라이브러리를 한 번에 엮고 `worker_process.rs`가 `CARGO_BIN_EXE_trss-worker`를 쓰기 때문이에요. `trss-worker`는 시험에만 `trss-web`에 의존해요. 라이브러리 단위 시험이 쓰는 `fixtures`(`legacy_*.yml`, `progressive_444.jpg`)는 `trss-legacy/tests/fixtures`에 남겼고, `trss-web`의 시험 하나가 `legacy_commented.yml`을 크레이트 경계를 넘는 상대 경로로 읽어요.

시험 코드를 둘 바꿨어요. 둘 다 같은 내용을 새 경계에서 확인해요.
- 마이그레이션 7번 시험은 `rss::save_path`와 비교하므로 `trss-core`에 둘 수 없어서 라이브러리의 `store/collect_folder_migration_tests.rs`로 옮겼어요. `StatusStore`로 읽는 마이그레이션 시험 둘도 `store/status/tests.rs`로 옮겼어요.
- `tests/collect_folder.rs`와 `store/history/tests.rs`가 마이그레이션 SQL을 `include_str!`로 직접 불러 6개·1개를 손으로 적용하던 곳은 `database_at(경로, n)`으로 바꿨어요. `database_at`은 같은 목록의 앞 n개를 적용해요.

### 크레이트 지도

크레이트마다 책임과 의존이에요. "지금"은 이 티켓이 끝난 상태예요.

| 크레이트 | 책임 | 의존하는 크레이트 (목표) | 지금 |
| --- | --- | --- | --- |
| `trss-core` | DB·마이그레이션(SQL 전부)·앱 설정·폴더 산술·`Millis`·`Clock`·`CycleLock`·`USER_AGENT`, 명령 큐·하트비트·`rename_noreplace`(Q2), KST 날짜 계산(Q3) | 없음 | 있음. 외부 크레이트(`rusqlite`·`thiserror`·`tokio`)에만 의존 |
| `trss-transmission` | Transmission RPC 클라이언트(시간 제한·이름 변경 재시도·비밀 값 가리기). 이름 변경에 쓸 값은 호출하는 쪽이 넘겨요(Q4) | core | 없음(`trss-legacy::transmission`) |
| `trss-anissia` | Anissia HTTP 클라이언트·응답 해석과 모델(Q5)·요일·날짜 표기 해석과 방영 칸 계산(Q3) | core | 없음(`trss-legacy::anissia`) |
| `trss-anilist` | AniList 클라이언트·제목 판정·응답 모델·요청 간격 저장(Q5) | core | 없음(`trss-legacy::artwork`·`seasons`) |
| `trss-collect` | 수집 규칙·피드·계획, 채널·규칙·구독·수집 이력, 영상 수정본, 지난 회차 검색, 회차 변환, 보관 제안, 규칙 보관·회차 되돌리기 명령, `season_link`(Q1), 상태 스냅샷, 편성의 영상·자막 상태(Q3) | core, transmission, anissia, library | 없음 |
| `trss-library` | 작품 발견·감시 폴더·표지·시즌 정보와 그 저장소, 감시 폴더 읽기(`watch`·`live`) | core, anilist | 없음 |
| `trss-import` | 기존 YAML 가져오기 계획 | core, collect | 없음(`trss-legacy::import`) |
| `trss-web` | 라우터·API·화면 연결, 이번 주 편성 조립(Q3) | 모든 기능 크레이트, core | 있음. `trss-legacy`·`trss-core`에 의존 |
| `trss-worker` | 실행 루프·명령 분배·주기 | collect, library, transmission, anissia, anilist, core | 있음. `trss-legacy`·`trss-core`에 의존 |
| `trss-legacy` | (임시) 위에서 아직 옮기지 않은 모든 모듈 | core | 있음 |

`trss-library`가 `trss-collect`에 의존하지 않고 collect가 library에 의존하는 방향은 코드에서 읽은 것이에요(아래 Q1). `trss-schedule`은 두지 않기로 했어요(Q3).

### 모듈별 배치

모듈마다 목표 크레이트예요. Q 표시는 ADR 0011의 처음 목록과 어긋나서 사용자가 정한 배치(아래 "사용자가 정한 것")이고, 나머지는 원칙대로 정했어요.

| 현재 모듈 | 목표 | 메모 |
| --- | --- | --- |
| `store/db.rs`, SQL 30개 | `trss-core` | 이동 끝. 마이그레이션 순서·바이트가 같아요 |
| `store/settings` | `trss-core` | 이동 끝. 7번 코드 마이그레이션이 이 안에 있어요 |
| `folders` | `trss-core` | 이동 끝. 사용자 목록은 library에 두었지만 마이그레이션이 `fold_bases`를 써서 파일 전체를 core에 뒀어요(Q6) |
| `Millis`, `Clock`·`system_clock`, `worker/lock`, `USER_AGENT` | `trss-core` | 이동 끝 |
| `web/`, `bin/trss-web` | `trss-web` | 이동 끝 |
| `worker/mod`(`Worker`), `env`, `commands`(분배), `cycle`(`run_cycle`), `bin/trss-worker` | `trss-worker` | 이동 끝 |
| `worker/context`(`CycleContext`) | `trss-worker` | 명령·감시가 쓰는 저장소 묶음이라 0030에서 쓰는 것만 받도록 좁혀야 해요. 그동안 `trss-legacy` |
| `transmission/` | `trss-transmission` | |
| `revision` | `trss-collect` | Transmission 클라이언트는 이름 변경에 쓸 값을 호출하는 쪽에서 받아요(Q4) |
| `anissia/{mod,parse,fake}` | `trss-anissia` | `parse`가 쓰는 `store/anissia`의 `Anime`·`WEEK_UPCOMING`도 함께 옮겨요(Q5) |
| `anissia/queue` | `trss-collect` | `store/anissia`(collect)를 쓰는 갱신 작업이에요 |
| `artwork/{anilist,title,fake}`, `seasons/anilist` | `trss-anilist` | `store::seasons`의 응답 모델(`Entry`·`FuzzyDate`·`Sequel`·`Airing`)과 AniList 몫의 요청 간격(`take_request_slot`·`block_requests`)도 옮겨요(Q5) |
| `artwork/{mod,files,image,queue}`, `seasons/{mod,combine,describe,queue}` | `trss-library` | |
| `discovery`, `automatic_watch` | `trss-library` | |
| `rss`, `rule`, `config`, `episode_offset`, `past_search`, `subscriptions` | `trss-collect` | `episode_offset`이 라이브러리의 시즌 정보를 읽어서 collect→library 방향이 정해져요 |
| `archive_suggestions` | `trss-collect` | |
| `schedule/calendar` | `trss-core`, `trss-anissia` | KST 날짜 계산(`day_of`·`day_start`·`days_from_civil`·`weekday`·`week_start`·`date_text`, `subscriptions`의 `DAY_MS`·`KST_OFFSET_MS`)은 core, Anissia 표기 해석(`weekday_of_anissia`·`PartialDate`)은 anissia(Q3) |
| `schedule/slot` | `trss-anissia` | (Q3) |
| `schedule/state` | `trss-collect` | (Q3) |
| `web::schedule_api` | `trss-web` | 그대로. 이번 주 편성을 조립해요(Q3) |
| `import/` | `trss-import` | `store/channels/import*`의 타입을 쓰므로 `trss-collect`에 의존해요 |
| `worker/{plan,feed,revisions,offsets}` | `trss-collect` | |
| `worker/commands/{receive_once,receive_past,episode_undo,rule_archive,link}` | `trss-collect` | 웹이 쓰는 요청 타입·검사 함수가 같은 파일에 있어요. `work_folder::rename_noreplace`는 `trss-core`(Q2) |
| `worker/commands/watch_rescan` | `trss-library` | |
| `worker/{watch,live}` | `trss-library` | |
| `worker/heartbeat` | `trss-core` | (Q2) |
| `worker/season_link` | `trss-collect` | (Q1) |
| `store/{channels,history,revisions,search_pace,anissia}` | `trss-collect` | `store/anissia`는 `store/channels`와 서로 참조해서 같은 크레이트여야 해요 |
| `store/{library,artwork,seasons}` | `trss-library` | |
| `store/setup` | `trss-library` | 첫 실행 체크리스트이고 트리거가 `watch_folders`에 걸려요. 웹만 읽어요. 기능 사이 원칙과 부딪히지 않아서 제가 골랐어요 |
| `store/commands` | `trss-core` | (Q2) |
| `store/status` | `trss-core`, `trss-collect` | 하트비트 몫은 core, 나머지 상태 스냅샷은 collect(Q2) |

### 역방향 의존 (이동 전 코드, 시험·문서 링크 제외)

티켓의 2026-10-02 목록을 `use`·경로 참조를 읽는 스크립트로 다시 셌어요. 문서 링크와 시험만 있던 항목이 많았어요.

- 기능 → worker (코드):
  - `anissia`·`artwork`·`seasons`: `Clock`·`system_clock`·`CycleLock`. **해결**(`trss-core`).
  - `artwork/files`: `commands::rule_archive::work_folder::rename_noreplace`. 남음(Q2).
  - `archive_suggestions`·`subscriptions`: `plan::{ChannelPlan, Judgement}`. 둘 다 collect가 되고 `plan`도 collect라 지도 안에서 풀려요.
  - `past_search`: `feed::{FeedItem, FetchError, 클라이언트}`·`plan::picks`·`Clock`. 같은 이유로 풀려요(`Clock`은 해결).
- 티켓 목록에 있었지만 코드에는 없던 것: `store/channels`(`plan`·`episode_undo`)·`store/history`·`store/revisions`·`store/library`·`transmission`의 worker 참조는 문서 링크뿐이에요. `episode_offset`의 `revisions::episode_name`은 시험에만 있고, `season_link` 참조는 문서 링크예요.
- 기능 → 웹: **없어요**. `schedule`의 `web::schedule_api`는 문서 링크뿐이에요.
- 웹 → worker: `commands::{rule_archive, episode_undo, receive_once, receive_past, watch_rescan}`의 요청 타입과 검사 함수, `plan::{ChannelPlan, Judgement, PastCause, PlanEvaluation, rule_destination}`, `revisions::{episode_name, same_folder, received_again_on_retry}`. 이것 때문에 이 코드를 `trss-worker`로 옮길 수 없어서 `trss-legacy::worker`에 남겼어요. 0030에서 collect·library가 되면 풀려요.
- worker 안: 명령 종류별 모듈·`offsets`·`season_link`·`watch`·`live`가 `CycleContext`를 받아요(0030에서 좁혀요).
- 묶음 사이의 순환: 티켓이 적은 `import` ↔ `store/channels`는 지금 코드에 없어요. `import`가 `store::channels::import`의 타입을 쓰는 한 방향이고, 반대는 문서 링크뿐이에요. 대신 아래 Q1–Q5의 순환을 찾았어요.

### 사용자가 정한 것

ADR 0011의 처음 목록(`rss`·`rule`… collect, `discovery`… library 등)대로 크레이트를 나누면 크레이트 사이에 순환이 생기는 곳이 있었어요. 크레이트 의존이 `core ← {transmission, anissia, anilist} ← library ← collect ← import ← {web, worker}`가 되도록 아래를 권고했고, 사용자가 2026-10-02에 정했어요(결정은 [ADR 0011](../adr/0011-feature-crate-workspace.md)). 권고대로 배치하고 아래에서 작은 상수(`FETCH_TIMEOUT`·`MAX_IMAGE_BYTES`, 큐의 `POLL`·`LOCK_RETRY`·`RETRY_DELAYS`, `MAX_REASON_CHARS`, 시간 상수 `DAY_MS` 등)를 쓰는 쪽 크레이트로 옮긴다고 치면, `use`·경로 참조를 읽는 스크립트로 센 크레이트 그래프(웹·worker 제외)에 순환이 없어요. 각 항목 끝에 정한 것을 적었어요.

1. **Q1 collect ↔ library**: collect가 library를 쓰는 곳은 `episode_offset`(시즌 정보), `rule_archive`→`watch`, `store/channels/import`→`ensure_automatic_in`, `episode_undo`→`discovery`의 상수이고, library가 collect를 쓰는 곳은 `worker/season_link`→`store::channels`(구독·규칙)예요. 사용자 목록은 `season_link`를 library에 뒀어요. 권고: collect→library 방향으로 두고 `season_link`를 `trss-collect`로 옮겨요. 구독과 시즌을 잇는 일이라 구독 쪽 코드예요. **정함**: 권고대로 collect→library, `season_link`는 `trss-collect`.
2. **Q2 library가 collect의 저장소를 쓰는 곳**: `watch_rescan`→`store/commands`, `heartbeat`·`live/watcher`→`store/status`, `artwork/files`→`rename_noreplace`예요. 권고: `store/commands`(웹이 맡기고 worker가 집는 요청 큐)와 `store/status`(worker가 웹에 남기는 스냅샷·하트비트)는 기능 규칙이 없는 웹↔worker 인계 테이블이라 `trss-core`에 두고, `heartbeat`도 core로, `rename_noreplace`는 core의 파일 도구로 내려요. 대안은 `store/status`를 나누는 것이에요(하트비트만 core). **정함**: 명령 큐·하트비트·`rename_noreplace`는 `trss-core`, `store/status`의 나머지(상태 스냅샷)는 `trss-collect`.
3. **Q3 collect ↔ schedule**: `archive_suggestions`(collect, ADR이 보관 제안을 collect로 적었어요)가 `schedule::calendar`·`slot`을 쓰고, `schedule`은 `subscriptions`의 시간 상수와 `store::anissia::Anime`을 써요. 권고: 순수 계산인 `schedule/{calendar,slot}`을 `trss-collect`로 내리고 `trss-schedule`에는 화면용 상태(`state`)만 둬요. 대안은 `archive_suggestions`를 `trss-schedule`로 올리는 것인데 ADR의 문구와 어긋나요. **정함**: `trss-schedule`을 두지 않아요. KST 날짜 계산은 `trss-core`, Anissia 요일·날짜 표기 해석과 `slot`은 `trss-anissia`, `state`는 `trss-collect`, 이번 주 편성 조립은 `trss-web`의 `schedule_api`.
4. **Q4 `trss-transmission` ↔ `revision`**: 외부 연동 크레이트인 `transmission`이 이름 변경에 `revision::Release::without_version`을 써요(`revision`은 regex와 std만 쓰는 잎 모듈이에요). 사용자 목록은 `revision`을 collect에 뒀어요. 권고: `revision.rs`를 `trss-transmission`으로 옮겨요. collect가 이미 의존하고 다른 기능 크레이트는 안 쓰며, 릴리스 이름 판독은 토렌트 이름 규칙이에요. 대안은 `transmission`이 이름을 호출한 쪽에서 받는 것(함수 시그니처 변경)이에요. **정함**: 대안. `transmission`은 이름 변경에 쓸 값을 호출하는 쪽에서 받고, `revision`은 `trss-collect`에 둬요.
5. **Q5 AniList·Anissia 클라이언트가 자기 저장소·모델과 엉킨 곳**: `artwork/anilist.rs`는 요청 간격을 `ArtworkStore`에 두고 `artwork` 상수·`title`을 쓰며, `seasons/anilist.rs`는 `store::seasons`의 모델(`Entry`·`FuzzyDate`·`Sequel`·`Airing`)을 반환해요. 반대로 library의 두 큐가 `AnilistError`·`title::decide`·`fetch_entry`를 써서, 사용자 목록대로 `trss-anilist`를 나누면 library와 서로 참조해요. `anissia/parse.rs`도 `store/anissia`의 `Anime`을 써요. 권고: 응답 모델과 요청 간격 저장(AniList 몫의 `take_request_slot`·`block_requests`)을 클라이언트 크레이트로 가져가고, 저장소와 큐는 기능 크레이트에 둬요. 모델 타입 이동이 있으므로 사용자가 확인해야 해요. 대안은 AniList 클라이언트를 `trss-library` 안에 두는 것이에요(ADR의 `trss-anilist`가 없어져요). **정함**: 권고대로 응답 모델과 요청 간격 저장을 클라이언트 크레이트로.
6. **Q6 `folders`(사소)**: 파일 전체를 core에 뒀어요(마이그레이션이 쓰는 `fold_bases`·`prefixed`·`FoldError`와 import·규칙 보관·웹이 쓰는 `common_ancestor`·`has_parent_dir` 등이 한 파일이에요). 사용자 목록은 library예요. 권고: 그대로 core. 나누려면 마이그레이션 몫만 core에 남기고 나머지를 library나 collect로 보내요. **정함**: 그대로 core.

### 검증한 것

- `cargo test`(커밋 `89ad352`, 단일 패키지): 라이브러리 1,078 + `tests/` 376 = 1,454개 통과. `cargo test --workspace`(마지막 커밋): `trss-core` 44, `trss-legacy` 748, `trss-web` 281, `trss-worker` 단위 5와 통합 376 = 1,454개 통과, 실패 0. `cargo test -- --list`로 뽑은 이름의 마지막 구간 다중집합이 이전과 이후에 같아서 빠지거나 늘어난 시험이 없어요. 이동한 단계마다 같은 숫자였어요.
- `cargo clippy --workspace --all-targets`: 경고 0(이동 전 `89ad352`도 0). `cargo fmt --check` 통과.
- 공통 기반: `cargo tree -p trss-core -e normal`이 `rusqlite`·`thiserror`·`tokio`만 보여요. 웹·worker·기능 크레이트가 없어요.
- 동작 불변: 이동 전 빌드와 이동 후 `trss-web`을 빈 DB에 한 번씩 열어 `sqlite_master` 87개 개체의 SQL과 `user_version` 29를 비교하니 같았어요. SQL 파일은 `git diff -M`에서 30개 모두 R100이에요. API 응답은 웹 단위 시험 281개(이전과 같은 시험)와 통합 시험으로만 확인했어요.
- 앱 이미지: `docker build`(`Dockerfile`, `--locked`)가 성공했어요. 그 이미지로 `trss-worker`와 `trss-web`을 같은 DB 폴더에 띄우니 worker가 "a cycle every 300s"를 출력하고 주기 한 번을 돌았으며(채널 0개, 연결할 수 없는 Transmission을 가리켰어요), 웹은 `/`에 200, `/api/health`에 `{"status":"ok"}`, `/api/collect/status`에 JSON을 답했어요. 둘 다 `/data/trss.db`를 열었어요.
- `.github/workflows/deploy.yml`과 compose 파일은 빌드 경로를 직접 쓰지 않아서 고치지 않았어요.

### 검증하지 못한 것

- 로컬 `docker compose` 실행: 실제 Transmission·RSS 피드·감시 폴더 데이터로 수집, 웹 명령, 웹 화면을 이전과 비교하지 않았어요. 위 이미지 시험은 빈 DB와 닿지 않는 Transmission이에요. 완료 기준의 이 행은 아직 안 채웠어요.
- 배포 workflow를 실행하지 않았어요(태그 푸시에만 돌아요).
- 웹 화면을 브라우저로 열어 보지 않았어요.
- `cargo doc`의 깨진 문서 링크는 이전에도 있었어요. 옮기며 생긴 것은 고쳤지만 남은 것을 이전과 하나씩 비교하지는 않았어요.

### 남은 일

- 로컬 compose 실행이 끝나야 완료로 바꿔요. 0030이 크레이트를 다 옮긴 뒤에 함께 확인해요.
- 0030이 옮길 순서는 의존 방향대로 `trss-transmission`·`trss-anissia`·`trss-anilist` → `trss-library` → `trss-collect` → `trss-import` 순이에요. 옮기면서 `trss-legacy::worker`의 `CycleContext`를 풀고, 재수출(`db`·`settings`·`folders`·`Millis`)을 없애고, `trss-worker/tests/`의 통합 시험과 도구를 시험하는 크레이트로 나눠요. `common`은 여러 크레이트가 쓰므로 시험 도구 크레이트가 필요할 수 있어요.
- `Cargo.toml`의 `tap`은 이전부터 쓰는 곳이 없어요(이 티켓과 무관해서 두었어요).
