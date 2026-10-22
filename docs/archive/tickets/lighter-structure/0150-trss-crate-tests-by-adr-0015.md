# 0150 `trss` 크레이트의 테스트를 테스트 나눔 ADR에 맞춰요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 테스트 정리](lighter-structure.md#테스트-정리), [테스트 나눔 ADR](../../../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0148](0148-one-instance-lock-and-cycle-stall.md)(인계와 잠금을 없앤 뒤)

## 작업

[0145](0145-merge-web-and-worker-crates.md)–[0148](0148-one-instance-lock-and-cycle-stall.md) 뒤 `trss` 크레이트에는 web의 API 테스트와 worker의 프로세스 테스트가 함께 있어요. 2026-10-21에 web의 단위 테스트는 468개(7.0초)였고 worker의 `it`은 91개(6.6초)였어요. 없어진 인계만 확인하던 테스트는 0147과 0148이 지웠어요.

[테스트 나눔 ADR](../../../adr/0015-test-a-rule-once-in-its-crate.md)은 규칙을 그 규칙을 가진 크레이트에서 한 번만 확인하게 해요. 이 티켓은 `trss` 크레이트의 테스트를 그 ADR에 맞추고 바이너리 배치를 정리해요. 다른 크레이트의 테스트 양은 줄이지 않아요(사용자 결정, 2026-10-21).

- API 테스트는 요청과 응답의 모양만 확인해요. 상태 코드, JSON 모양, 접근 경계, 오류 본문이 여기에 들어가요.
- 프로세스 테스트는 다시 시작, 단일 실행 잠금, 동시 실행, 이벤트처럼 프로세스에서 생기는 일만 확인해요.
- 기능 규칙을 API나 프로세스를 거쳐 다시 확인하던 테스트는, 기능 크레이트가 그 규칙을 확인하고 있으면 지우고, 아니면 기능 크레이트로 옮겨요.
- 메모리 최고치를 측정하는 테스트 3개는 기본 실행에 그대로 둬요(사용자 결정, 2026-10-21). 합친 프로세스가 메모리 한도 안에서 도는지를 바꿀 때마다 바로 알려고 두는 거예요.

## 완료 기준

- [x] `trss` 크레이트의 테스트는 단위 테스트 바이너리 하나와 `tests/it` 바이너리 하나에 있어요. 자기 바이너리를 갖는 것은 메모리 최고치 테스트처럼 프로세스를 따로 써야 하는 테스트뿐이고, 결과에 그 까닭을 테스트마다 적어요.
- [x] `trss` 크레이트에 남은 테스트는 모두 API의 모양이나 프로세스에서 생기는 일을 확인해요.
- [x] 지운 테스트마다 같은 규칙을 확인하는 기능 크레이트의 테스트를, 옮긴 테스트마다 옮긴 곳을 결과에 짝지어 적어요.
- [x] `remote_screen_docker`는 기본 `cargo test --workspace`에서 빌드되지 않아요. 따로 켜면 지금처럼 돌고, 그 방법이 테스트 파일의 설명과 문서에 있어요.
- [x] 메모리 최고치를 측정하는 테스트 3개는 기본 실행에 있어요.
- [x] `trss` 크레이트 테스트 바이너리들의 시간과 테스트 수를 앞뒤로 견준 값이 결과에 있어요. [0144](0144-lighter-structure-baseline.md)와 같은 명령과 빌드 상태로 측정해요.
- [x] 명세의 검증 표가 이름으로 가리키는 테스트를 옮기거나 지웠으면 같은 커밋에서 그 행을 고쳐요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-21에 `trss` 크레이트의 테스트를 단위 테스트 바이너리 하나, `tests/it` 바이너리 하나, 메모리 최고치 테스트 3개의 바이너리로 정리했어요. 규칙을 API로 다시 확인하던 테스트는 하나를 지우고 셋을 줄였어요. 기본 실행의 테스트 바이너리는 워크스페이스 전체에서 29개에서 27개가 됐어요.

### 바이너리

| 바이너리 | 앞 | 뒤 |
| --- | --- | --- |
| 단위 테스트(`src/lib.rs`) | 479개, 6.93초 | 478개, 6.91초 |
| `tests/it` | 85개, 7.14초 | 85개, 7.12초 |
| `tests/subtitle_upload_peak.rs` | 1개, 2.96초 | 1개, 2.94초 |
| `tests/artwork_serving_peak.rs` | 1개, 1.43초 | 1개, 1.43초 |
| `tests/artwork_upload_peak.rs` | 1개, 0.11초 | 1개, 0.10초 |
| `src/main.rs`의 단위 테스트 | 0개 | 빌드하지 않아요 |
| `tests/remote_screen_docker.rs` | 무시된 것 4개 | `tests/it`의 모듈, 기능을 켤 때만 빌드해요 |

앞은 [0149](0149-no-worker-in-texts.md)의 코드, 뒤는 이 티켓의 코드로 2026-10-21에 [0144](0144-lighter-structure-baseline.md)와 같은 명령(`cargo test --locked --workspace -j 4`를 빌드 뒤 다시 실행)으로 측정했어요. 워크스페이스 전체는 앞이 바이너리 29개, 통과 3,055개, 건너뛴 것 14개, 시간 합 43.7초였고, 뒤가 바이너리 27개, 통과 3,054개, 건너뛴 것 10개, 시간 합 43.1초였어요. 건너뛴 것이 4개 준 것은 기본 실행이 `remote_screen_docker`를 빌드하지 않아서예요. 원본 로그는 이 PC의 `dev/local/measure/0149/`와 `dev/local/measure/0150/`에 있어요. 빌드 시간은 측정하지 않았고, [0161](0161-remeasure-and-crate-decision.md)이 다시 측정해요.

자기 바이너리를 갖는 테스트는 메모리 최고치 테스트 3개뿐이에요. 셋 다 프로세스 전체의 값을 봐서, 같은 프로세스에서 다른 테스트가 메모리를 쓰면 값이 틀려요.

- `subtitle_upload_peak`: 자막 올리기의 본문을 읽는 동안 `/proc/self`의 상주 메모리를 측정해요.
- `artwork_serving_peak`: 표지 요청이 몰릴 때 전역 할당자가 세는, 할당하고 아직 돌려주지 않은 바이트의 최고치를 측정해요.
- `artwork_upload_peak`: 표지 올리기의 본문을 읽는 동안 `/proc/self`의 상주 메모리를 측정해요.

`remote_screen_docker`는 환경 변수나 현재 폴더 같은 프로세스 전역 상태를 바꾸지 않고 컨테이너 이름도 프로세스 번호와 테스트마다 달라서, 자기 프로세스가 필요하지 않아요. 그래서 `tests/it`의 모듈로 옮기고 `trss`의 `remote-screen-docker` 기능으로 가렸어요. 실행하는 방법은 테스트 파일의 설명과 readme의 [Tests](../../../../readme.md#tests)에 있어요. 바이너리의 `src/main.rs`에는 테스트가 없어서 `test = false`로 빌드하지 않아요.

### 테스트 나누기

`trss-web`과 `trss-worker`의 테스트는 [0101](../refactoring-between-5-and-6/0101-receive-line-tests.md), [0110](../refactoring-between-5-and-6/0110-job-line-tests.md), [0113](../refactoring-between-5-and-6/0113-remaining-area-tests.md)이 테스트 나눔 ADR대로 모두 나눴어요. 그래서 0113을 마친 `cc91ce55` 뒤에 생기거나 바뀐 테스트만 다시 봤어요.

- `cc91ce55`와 지금의 테스트 이름을 견주니 새 이름이 30개였어요. 11개는 0147과 0148이 프로세스 일(명령 흐름, 단일 실행 잠금, 주기의 차례)을 확인하려고 더하거나 바꾼 것이고, 19개는 0.7.0 뒤 기능 티켓이 web에 더한 것이에요. 19개 중 하나는 이름만 바뀐 것이에요.
- `cc91ce55` 뒤에 몸통이 바뀐 web 테스트를 diff로 찾아 읽었어요. 경로·크레이트 이름·깨우기만 바뀐 것은 뺐어요.
- worker 쪽은 그 뒤 0139가 테스트의 구조체에 필드 하나를 더한 것뿐이었어요.

`stora:explore`가 각 테스트의 몸통과 그 테스트가 부르는 처리기를 읽어 모양, 규칙, 섞임으로 나눴고, 지우거나 줄이기 전에 가리킨 기능 크레이트 테스트를 읽어 같은 경우인지 확인했어요. 지우거나 줄인 테스트와 같은 규칙을 확인하는 테스트는 아래와 같아요.

| 테스트 | 한 일 | 같은 규칙을 확인하는 테스트 |
| --- | --- | --- |
| `trss` `web/seasons_api/tests.rs` `a_movie_only_works_season_0_is_its_first_season_for_the_search_the_mark_and_the_original_title` | 지움 | `trss-library` `store/seasons/movie_tests.rs`의 `the_first_season_of_a_movie_only_work_is_season_0_and_a_season_from_1_on_comes_first`, `a_movie_only_work_asks_for_its_automatic_search_once_and_a_season_0_folder_does_not`, `only_a_movie_only_works_season_0_can_ask_again_and_its_entry_is_what_the_cover_follows`. 응답의 `can_auto`·`pending`·`native_title`은 같은 파일의 `a_season_without_a_link_is_unknown_and_never_filled_from_another_season`과 `the_original_title_is_the_first_seasons_first_entrys_and_a_new_work_waits_for_its_search`가, `movie_only`는 `web/seasons_anissia_api/tests.rs`의 `a_movie_is_linked_by_its_own_version_and_the_work_detail_shows_it`가 확인해요. |
| `trss` `web/seasons_anissia_api/tests.rs` `a_candidate_carries_the_key_rank_and_form_of_its_episode` | 한 회차의 여러 표기와 두 제작자의 열 줄을, 세 제작자의 `0`·`05`·`SP` 세 줄로 줄였어요. 형태(`zero`·`number`·`text`)는 web이 정해서 셋 다 남겼어요. | `trss-core` `episode.rs`의 `zero_and_the_episode_a_number_is`, `ranks_follow_the_order_of_the_keys_and_share_one_per_episode`, `the_database_keeps_the_key_as_n_and_the_number_or_t_and_the_text` |
| `trss` `web/save_folder_api/tests.rs` `a_suggestion_names_the_folder_its_library_work_and_why` | 작품을 고른 제안이 `Show/Season 03`인지 보던 것을 뺐어요. `work` 질의와 `archived`는 남겼어요. | `trss-collect` `save_folder/tests.rs` `without_a_marker_the_last_seasons_last_entry_decides_same_or_next`의 연결 없는 마지막 시즌 경우 |
| `trss` `web/status_api/tests.rs` `a_cycle_without_an_end_past_its_bound_is_stalled` | `a_cycle_the_rule_takes_for_stalled_is_reported_stalled`로 바꾸고, 10분과 50분 1 ms의 경계 대신 두 시간 된 주기 하나가 `stalled`로 나오는지만 봐요. | `trss-collect` `store/history/tests.rs` `an_unfinished_cycle_runs_until_the_larger_of_half_an_hour_and_ten_intervals_and_then_is_stalled` |

옮긴 테스트는 없어요. 기능 크레이트가 확인하지 않는 규칙을 web 테스트만 확인하던 경우가 없었어요.

아래는 규칙의 결과를 보지만 그 결정이 기능 크레이트가 아니라 web 코드에 있어서 남겼어요. 코드를 기능 크레이트로 옮기는 일은 이 티켓의 범위가 아니에요(구현 결정, 2026-10-21).

- `web/subtitle_creator_api/tests.rs` `a_movie_only_works_files_are_named_from_the_anime_its_movie_is_linked_to`: 극장판만 있는 작품의 시즌 0이 시즌 연결 대신 극장판의 Anissia 연결로 제작자를 찾는 것은 `subtitle_creator_api.rs`의 `season_anime`이 정해요.
- `web/schedule_api/tests.rs` `a_cycle_that_runs_holds_the_downloading_look_until_it_outlives_its_bound`: 도는 중인 주기를 `영상 받는 중` 판정에 잇는 일은 `schedule_api.rs`의 `downloading_hashes`가 해요.
- 배치 확인의 `named_is_episode`와 자막 후보의 `episode_form`도 web이 계산해요.

분류하면서 web 테스트만 닿는 기능 코드를 두 곳 찾았어요. 극장판 작업의 `movie`가 자막 작업 요청 글에 들어가는 것(`trss-jobs` `NewJob::pick`, `NewUpload`)과 `LibraryStore::titles()`예요. 각각 `web/jobs_api/tests.rs`의 극장판 고르기 테스트, `web/subtitle_upload_api/tests.rs`의 극장판 올리기 테스트, `web/policy_api/tests.rs`의 작품 목록 테스트가 응답의 모양으로 확인하고 있어서 그대로 뒀어요.

### 따로 켜는 원격 화면 테스트

2026-10-21에 이 PC에서 `dev/compose.sh up`이 지은 브라우저 이미지(`ghcr.io/syrflover/trss-browser:local`)로, 기능을 켜고 무시된 테스트 4개를 돌렸어요. `tests/it`으로 옮기기 전과 뒤 모두 4개가 통과했고, 옮긴 뒤에는 29.3초였어요. `cargo clippy -p trss --all-targets`는 기능을 켤 때와 끌 때 모두 경고가 없어요.

### 문서

[라이브러리 명세](../../../requirements/library.md)의 극장판 AniList 연결 검증 행에서 지운 테스트를, [웹 앱 공통 명세](../../../requirements/web-app.md)의 수집 상태 검증 행에서 이름을 바꾼 테스트를 고쳤어요. readme의 로컬 개발 환경 절에 [Tests](../../../../readme.md#tests)를 더했어요.
