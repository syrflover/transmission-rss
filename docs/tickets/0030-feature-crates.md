# 0030 기능을 크레이트로 옮기고 단일 라이브러리를 없애요

- 상태: 진행 중 (크레이트 이동과 단일 라이브러리 제거는 끝났어요. 로컬 compose 실행만 남았고 0029의 같은 확인과 함께 해요. 아래 "결과")
- 출처: [기능별 크레이트 ADR](../adr/0011-feature-crate-workspace.md), [공통 라이브러리의 모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: [0029](0029-workspace-skeleton.md)(확정한 크레이트 지도와 공통 기반 크레이트. 둘 다 2026-10-02에 갖췄고, 0029에 남은 로컬 compose 실행은 이 티켓의 같은 확인과 함께 해요)

## 작업

0029에서 확정한 지도대로 외부 연동과 기능을 아래에서 위 순서로 한 묶음씩 크레이트로 옮기고, 묶음마다 workspace 전체가 빌드·테스트되는 상태로 커밋해요.
기능의 조회·갱신 코드(`store/<기능>`)는 그 기능의 크레이트로 함께 옮기고, 그 안의 스키마 SQL은 `trss-core`의 마이그레이션 목록에 둬요.
옮기면서 드러난 역방향 의존은 기능 인터페이스를 통하도록 고치되, 동작은 바꾸지 않아요.
모든 묶음을 옮기면 남은 단일 라이브러리 크레이트를 없애요.

## 완료 기준

| 확인 | 기대 결과 |
| --- | --- |
| 크레이트 의존 그래프(`cargo tree` 등) | 0029의 지도와 같고, 기능 크레이트가 웹·worker 크레이트를 참조하지 않으며, 순환이 없어요. |
| 0029에서 찾은 역방향 의존 | 모두 없어졌거나, 남긴 것과 까닭이 결과에 있어요. |
| workspace 전체 테스트 | 이동 전과 같은 테스트가 모두 통과하고 테스트 수가 줄지 않았어요. |
| 앱 이미지와 로컬 compose 실행 | 수집 주기, 웹 명령(한 번 받기·규칙 보관), 감시 폴더 반영, 표지·시즌 정보 큐가 이전과 같이 동작해요. |
| 단일 라이브러리 | 남아 있지 않아요. |

## 결과

### 한 일

`cargo test --workspace`가 1,455개 통과하고 실패는 0이에요. 이전은 1,454개였고, 늘어난 하나는 `trss-anissia`에 더한 `anissia_counts_its_week_from_sunday`(`weekday_of_anissia`가 Anissia의 일요일 시작 요일 번호를 월요일 시작으로 바꾸는지 보는 시험)예요. 나머지는 이름이 같은 시험이 그대로 옮겨졌어요.

코드 변경은 커밋 열 개로 나눴고 커밋마다 workspace 전체가 빌드되고 시험이 통과해요. 아래는 의존이 낮은 쪽에서 높은 쪽으로 간 순서예요.

1. `refactor(core)`: 명령 큐(`store/commands`), 하트비트, KST 날짜 계산, `rename_noreplace`를 `trss-core`로 옮겼어요. 하트비트는 `store/status`에서 떼어 `HeartbeatStore`(워커가 기록하고 웹이 읽어요)로 만들었어요.
2. `refactor(legacy)`: 0029가 남긴 `db`·`settings`·`folders`·`Millis` 재수출을 없애고 호출부를 `trss-core` 경로로 바꿨어요.
3. `refactor(transmission)`: Transmission 클라이언트를 `trss-transmission`으로 옮겼어요.
4. `refactor(anissia)`: Anissia 클라이언트·응답 모델(`Anime`)·요청 간격 저장·요일/날짜 표기 해석·방영 칸 계산을 `trss-anissia`로 옮겼어요.
5. `refactor(anilist)`: AniList 클라이언트·제목 판정·시즌 항목 질의·응답 모델(`Entry`·`FuzzyDate`·`Sequel`·`Airing`)·요청 간격 저장을 `trss-anilist`로 옮겼어요.
6. `refactor(library)`: 작품 발견, 자동 감시 폴더, 표지, 시즌 정보, 첫 실행 체크리스트와 `store/{library,artwork,seasons,setup}`, 감시 폴더 읽기(`watch`·`live`)와 `watch_rescan` 명령을 `trss-library`로 옮겼어요.
7. `refactor(collect)`: 채널·규칙·구독, 피드·계획, 명령(한 번 받기·지난 회차 받기·회차 되돌리기·규칙 보관·`link`), 수정본, 회차 변환(`offsets`·`episode_offset`), `season_link`, 지난 회차 검색, 구독, 보관 제안, Anissia 갱신 큐와 `store/{channels,history,revisions,search_pace,anissia,status}`를 `trss-collect`로 옮겼어요.
8. `refactor(import)`: `trss-legacy`에는 가져오기 모듈만 남아서 `trss-import`로 이름을 바꾸고 모듈을 크레이트 뿌리로 올렸어요. `trss-legacy`는 없어요. 안 쓰던 `tap` 의존성도 함께 없앴어요.
9. `test`: 한 크레이트의 코드만 쓰는 통합 시험 네 개를 그 크레이트로 옮겼어요.
10. `docs`: 코드 문서 링크를 고쳤어요.

마이그레이션 SQL은 옮기거나 고치지 않았어요(`master`와 비교해 `*.sql`에 변경이 없어요).

### 모듈이 간 곳

| 크레이트 | 옮겨 온 것 |
| --- | --- |
| `trss-core` | 명령 큐, `HeartbeatStore`, KST 날짜 계산(`calendar`), `rename_noreplace`(`files`) |
| `trss-transmission` | `transmission/` 전체. 이름 변경은 호출하는 쪽이 넘긴 함수로 토렌트 이름을 구해요(아래 "배치표와 다른 점") |
| `trss-anissia` | `Anissia` 클라이언트와 응답 해석(`parse`), `Anime`, 요청 간격 저장 `RequestPace`, `calendar`(`weekday_of_anissia`·`PartialDate`), `slot`. 가짜 서버는 `test-support` 기능이에요 |
| `trss-anilist` | `Anilist` 클라이언트, `title`, `season`(시즌 항목 질의), `Entry` 같은 응답 모델, 요청 간격 저장 `RequestPace`, `MAX_IMAGE_BYTES`·`FETCH_TIMEOUT`. 가짜 서버는 `test-support` 기능이에요 |
| `trss-library` | `artwork`, `seasons`, `discovery`, `automatic_watch`, `store/{library,artwork,seasons,setup}`, `watch`, `live`, `watch_rescan` |
| `trss-collect` | `anissia`(갱신 큐), `archive_suggestions`, `commands/*`, `config`, `context`, `episode_offset`, `feed`, `offsets`, `past_search`, `plan`, `revision`, `revisions`, `rss`, `rule`, `schedule/state`, `season_link`, `store/{anissia,channels,history,revisions,search_pace,status}`, `subscriptions` |
| `trss-import` | 가져오기 계획(`legacy`·`fit`·`plan`·`comments`·`suggest`) |
| `trss-web`, `trss-worker` | 그대로. 의존만 기능 크레이트로 바뀌었어요 |

기능 크레이트 안에서는 이전의 `store/<기능>/` 구조를 그대로 따라서 경로 이름만 바뀌었어요. 예를 들어 `crate::store::channels`는 `trss_collect::store::channels`예요.

### 최종 크레이트 의존

`cargo tree -e normal --depth 1`로 읽은 크레이트 사이의 의존이에요(외부 크레이트 제외).

| 크레이트 | 의존하는 크레이트 |
| --- | --- |
| `trss-core` | 없음 |
| `trss-transmission` | 없음 |
| `trss-anissia` | core |
| `trss-anilist` | core |
| `trss-library` | core, anilist |
| `trss-collect` | core, transmission, anissia, anilist, library |
| `trss-import` | core, collect |
| `trss-web` | core, anissia, anilist, library, collect, import |
| `trss-worker` | core, transmission, anissia, anilist, library, collect |

기능 크레이트가 `trss-web`·`trss-worker`를 참조하는 곳은 없어요. 시험 쪽(`dev-dependencies`)에서는 `trss-worker`만 `trss-web`을 쓰고, 나머지는 위 의존에 `test-support` 기능만 더해요. 순환은 없어요(Cargo가 순환을 받아들이지 않아요).

목표 그래프(`core ← {transmission, anissia, anilist} ← library ← collect ← import ← {web, worker}`)와 다른 점은 둘이에요. `trss-transmission`은 `trss-core`를 쓸 곳이 없어서 의존하지 않아요. `trss-worker`는 `trss-import`를 쓰지 않아서 의존하지 않아요(가져오기는 웹만 해요).

### 0029의 역방향 의존은 어떻게 됐나

- 기능 → worker, `Clock`·`system_clock`·`CycleLock`: 0029에서 `trss-core`로 풀렸어요.
- 기능 → worker, `artwork/files`의 `rename_noreplace`: `trss-core`의 `files`로 내렸어요(Q2).
- 기능 → worker, `archive_suggestions`·`subscriptions`의 `plan`, `past_search`의 `feed`·`plan`: 모두 `trss-collect` 안이 되어 크레이트 경계가 없어졌어요.
- 웹 → worker(명령의 요청 타입과 검사 함수, `plan`·`revisions`): 그 코드가 `trss-collect`(`watch_rescan`은 `trss-library`)로 가서 웹이 기능 크레이트를 쓰는 평범한 의존이 됐어요. `trss-legacy::worker`는 없어요.
- worker 안, `CycleContext`: 없앴어요. 감시 폴더를 읽는 코드는 `trss-library`의 `WatchContext`(라이브러리 저장소·설정·하트비트·스캔 캐시·inotify 감시)만 받아요. 수집 코드는 `trss-collect`의 `CollectContext`(채널·설정·이력·수정본·시즌·라이브러리 저장소, inotify 감시, Transmission 주소와 클라이언트, 피드 클라이언트, 이름 변경·폴더 이동 정책, 가림 도구)를 받아요. Transmission 세션 설정과 감시 폴더 읽기 상태(`WatchContext`)는 `Worker`가 들고 있어요. 규칙 보관이 쓰는 감시 알림은 `watch::follow_move(library, live, ...)`처럼 쓰는 것만 받아요. **남은 것**: 수집 쪽 명령(한 번 받기·지난 회차 받기·회차 되돌리기·규칙 보관)과 `offsets`·`season_link`·`revisions`는 여전히 하나의 `CollectContext`를 받아요. 함수마다 쓰는 저장소만 받도록 더 좁히려면 명령끼리 서로 부르는 호출(`receive_past`가 `receive_once::execute_with`를 부르는 것 등)마다 문맥을 나눠야 해서 이 티켓에서는 하지 않았어요. 필요하면 0031에서 명령 실행 기반을 다루며 좁혀요.
- 묶음 사이의 순환 `import` ↔ `store/channels`: 코드에는 없었고, `trss-import`가 `trss-collect`를 쓰는 한 방향뿐이에요.
- 기능 → 웹(`schedule`의 `web::schedule_api`): 문서 링크뿐이었고, 링크를 고쳤어요.

### 배치표와 다른 점

- **Transmission 이름 변경(Q4)**: 이름 변경은 토렌트 이름을 Transmission에서 읽은 뒤에 쓸 값을 구해서, 호출하는 쪽이 값 하나를 미리 넘길 수 없어요. 그래서 `rename_torrent`·`rename_with_retries`가 `NameForTrname`(이름을 받아 trname에 넘길 문자열을 돌려주는 함수 포인터)을 받고, 워커의 `cycle`이 `Release::without_version`을 쓰는 함수를 넘겨요. `trss-transmission`은 `revision`을 모르고, 동작은 같아요.
- **Anissia 요청 간격(Q5 확장)**: 티켓에는 AniList 몫만 있었지만 Anissia도 같은 구조(`anissia_pace` 표)였고 클라이언트가 `store/anissia`를 써서, 같은 이유로 `trss-anissia`의 `RequestPace`로 옮겼어요. AniList는 `trss-anilist`의 `RequestPace`예요. `Anilist::new`는 이제 `Db`를 받고 `AnilistError::Store`는 `DbError`예요.
- **Anissia 갱신 큐**: 클라이언트(`Anissia`)에 붙어 있던 큐 메서드는 `store/anissia`가 `trss-collect`에 있어서 클라이언트에 둘 수 없어요. 큐를 `trss_collect::anissia::AnissiaQueue { anissia, store }`로 따로 세웠어요.
- **작은 상수**: `MAX_IMAGE_BYTES`·`FETCH_TIMEOUT`은 `trss-anilist`가 정의하고 `trss-library`의 `artwork`가 다시 내보내요. 큐의 `POLL`·`LOCK_RETRY`는 `trss-library`의 `artwork::queue`에서 `pub`으로 두고 `trss-collect`의 Anissia 큐가 불러 써요(collect가 library를 쓰는 방향이라 순환이 아니에요). 쓰는 쪽으로 옮기지는 않았어요.
- **`MovePolicy`**: `worker/mod`의 재수출 대신 `trss_collect::commands::rule_archive::work_folder::MovePolicy`로 써요.
- **`pub(crate)`**: 크레이트 경계를 넘게 된 `Rule.directory`만 `pub`으로 열었어요. 나머지는 컴파일러가 찾는 대로 맞췄고 공개 면은 늘리지 않았어요.
- **`store/status`**: 하트비트는 `trss-core`의 `HeartbeatStore`(새 타입 `WorkerHeartbeat`)가 맡고, 나머지 상태 스냅샷은 `trss-collect`의 `StatusStore`예요. 웹은 하트비트를 `AppState.heartbeat`로 읽어요.
- **통합 시험**: `trss-worker/tests`의 20개 파일 중 한 크레이트의 코드만 쓰는 네 개를 옮겼어요. `artwork_header_check`는 `trss-library`로, `revision_crc_peak`는 `trss-collect`로, `artwork_serving_peak`·`artwork_upload_peak`는 `trss-web`으로 갔어요(할당량을 세는 시험이라 파일 하나가 실행 파일 하나인 구성을 그대로 지켜요). 나머지 16개는 `Worker`로 주기와 명령을 돌리고 `common`의 가짜 Transmission·피드 서버·웹 라우터를 함께 써요. 가짜 Transmission만 따로 떼어 작은 시험 지원 크레이트로 만들어도 그 시험들은 `Worker`의 분배와 주기를 부르니 `trss-worker`에 남아요. 그래서 도구도 시험과 함께 `trss-worker/tests/common`에 두었어요.
- **고정 파일**: `progressive_444.jpg`는 `trss-library/tests/fixtures`로, `legacy_*.yml`은 `trss-import/tests/fixtures`에 있어요. `trss-web`의 시험 하나가 `legacy_commented.yml`을 상대 경로로 읽는 것은 0029와 같아요.

### 검증한 것

- 시험 수(`cargo test --workspace`, 단위 + 그 크레이트의 `tests/`): 이전(0029 끝)은 `trss-core` 44, `trss-legacy` 748, `trss-web` 281, `trss-worker` 단위 5 + 통합 376으로 1,454개예요. 이후는 `trss-core` 65, `trss-transmission` 7, `trss-anissia` 33, `trss-anilist` 7, `trss-library` 206(단위 205 + 통합 1), `trss-collect` 430(단위 429 + 통합 1), `trss-import` 47, `trss-web` 283(단위 281 + 통합 2), `trss-worker` 377(단위 5 + 통합 372)로 1,455개예요. 실패 0.
- 시험 이름: `cargo test --workspace -- --list`의 마지막 경로 구간 다중집합을 이전과 비교하니 `anissia_counts_its_week_from_sunday` 하나만 늘고 빠진 이름은 없어요. AniList 요청 간격 시험 `requests_take_turns_and_a_block_holds_them_all`은 이름을 그대로 두고 `trss-anilist`로 옮겼어요.
- `cargo clippy --workspace --all-targets`: 경고 0. `cargo fmt --check` 통과.
- 의존: 위 표. 기능 크레이트가 `trss-web`·`trss-worker`를 참조하지 않아요.
- 동작 불변: 이전 빌드와 새 `trss-web`(release)을 빈 DB에 한 번씩 열어 `sqlite_master`의 87개 개체 SQL과 `user_version` 29를 비교하니 같았어요. SQL 파일은 바뀌지 않았어요. API 응답은 웹 단위 시험 281개와 통합 시험으로만 확인했어요.
- 앱 이미지: `docker build`(`Dockerfile`, `--locked`)가 성공했어요. 그 이미지로 `trss-worker`와 `trss-web`을 같은 빈 DB 폴더에 띄우니 worker가 "a cycle every 300s"를 출력하고 주기를 한 번 돌았으며(채널 0개, 닿지 않는 Transmission), 웹은 `/`에 200, `/api/health`에 `{"status":"ok",...}`, `/api/collect/status`에 JSON을 답했어요.
- 다른 문서의 파일 경로: `docs/` 아래 27개 문서가 인용한 옛 경로(`src/web/...`, `src/store/...`, `tests/...` 등)를 이동 대응표(워크스페이스 전환 직전 커밋 `89ad352`와 현재의 이름 변경 비교)로 새 경로로 고쳤어요. 이동 전부터 없던 파일(`src/main.rs`, `tests/legacy_comparison.rs` 등)은 그대로 뒀어요. 0029의 결과는 그때의 상태를 적은 기록이라 고치지 않았어요.

### 검증하지 못한 것

- 로컬 `docker compose` 실행: 실제 Transmission·RSS 피드·감시 폴더 데이터로 수집 주기, 웹 명령(한 번 받기·규칙 보관), 감시 폴더 반영, 표지·시즌 정보 큐를 이전과 비교하지 않았어요. 위 이미지 시험은 빈 DB와 닿지 않는 Transmission이에요. 완료 기준의 이 행과 0029의 같은 행은 사용자와 함께 확인해야 해서 아직 채우지 않았고, 그래서 상태가 `진행 중`이에요.
- 배포 workflow(태그 푸시에만 돌아요)와 웹 화면을 브라우저로 여는 것은 하지 않았어요.
- `cargo doc`: 옮기며 깨진 링크 몇 개는 고쳤지만 남은 경고(비공개 항목 링크 등)를 이전과 하나씩 비교하지는 않았어요.
