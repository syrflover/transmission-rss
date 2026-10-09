# 0113 라이브러리, 설정, 웹 앱 공통 영역의 테스트를 나누고 검증 표를 둬요

- 상태: 완료
- 출처: [그 밖의 테스트 정리](refactoring.md#그-밖의-테스트-정리), [영역별 검증 표](refactoring.md#영역별-검증-표), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0094](0094-file-identity-in-core.md), [0095](0095-episode-text-key-in-core.md), [0096](0096-file-and-path-helpers.md), [0102](0102-request-pace-in-core.md), [0103](0103-queue-loop-in-core.md), [0104](0104-durable-file-writes-in-core.md), [0111](0111-web-and-screen-rules.md), [0112](0112-small-helpers-and-test-helpers.md)

## 작업

수집 받기 줄기([0101](0101-receive-line-tests.md))와 자막 작업 줄기([0110](0110-job-line-tests.md)) 밖의 영역도 ADR 0015대로 테스트를 나눠요.

- trss-web의 API 테스트 461개 중 70–110개가 다른 크레이트의 규칙을 다시 확인한다고 2026-10-07에 추정했어요. 0101과 0110에서 나누지 않은 것을 여기서 나눠요. 규칙의 주인 크레이트에 같은 경우가 있으면 지우고, 없으면 내려요.
- trss-web의 `folders_on_different_filesystems_are_refused`는 `/proc`를 훑어 혼자 4.9초가 걸리고, trss-collect의 같은 이름 테스트와 같은 규칙을 확인해요. 웹 쪽을 정리해요.
- 0101에서 trss-web의 보관 제안, 수집 이력, 규칙, 제목 후보, `in_place` 테스트를 나눴어요. 규칙 미리보기의 분류 테스트는 [0111](0111-web-and-screen-rules.md)에서 trss-collect `plan/preview/tests.rs`로 내렸어요.
- worker의 수집 영역 테스트 중 0101의 다섯 파일 밖에 있는 것도 같은 방식으로 나눠요. 2026-10-09 `a4bb97c`에서 `collect_folder.rs` 3개, `first_read.rs` 8개, `title_waiting.rs` 3개, `past_search.rs` 32개였어요.
- trss-anilist와 trss-anissia의 `Retry-After` 클라이언트 테스트는 [0102](0102-request-pace-in-core.md) 뒤로 trss-core `response::retry_after`의 테스트와 같은 값을 확인해요. 클라이언트에는 헤더를 읽어 넘기는 연결만 남겨요.
- [0103](0103-queue-loop-in-core.md) 뒤로 큐 루프와 잠금 파일 이름을 trss-core `queue::tests`와 `lock::tests`, `access::tests`가 확인해요. 그와 겹치는 7개를 나눠요. trss-collect의 `the_queue_holds_its_lock_and_a_second_observer_waits_for_it`, trss-library의 `the_queue_stops_on_shutdown_and_a_restart_resumes_it`, `hundreds_of_new_works_are_searched_one_at_a_time_at_the_pace`의 마지막 경로 비교, trss-collect `anissia/tests.rs`와 `anissia/captions_tests.rs`의 `the_lock_file_sits_next_to_the_database_and_apart_from_the_other_queues` 2개, worker의 `app_data_files_cover_every_lock_and_the_wake_socket`, `browser.rs`의 `one_worker_at_a_time_has_the_browser`의 경로 비교예요. 각 크레이트의 규칙(자막 목록의 읽기 주기, 표지 큐의 이어 하기, 브라우저의 잠금 쥐기)은 남겨요. 나눈 뒤 크레이트마다 남은 `lock_path_for` 감싸개를 지워요.
- worker의 `app_data_folders_cover_the_receive_area_the_artwork_and_the_subtitle_files`는 [0096](0096-file-and-path-helpers.md)에서 두 목록이 trss-core의 같은 상수를 쓰게 되어 실패할 수 없어요. 지워요.

[라이브러리와 작품](../specs/library.md), [설정과 이전](../specs/settings.md), [웹 앱 공통](../specs/web-app.md) 명세에 요구별 검증 표를 둬요.

## 완료 기준

- 결과 절에 파일마다 앞뒤 테스트 수와, 지운 것·내린 것·남긴 것의 개수가 있어요. 지운 테스트마다 같은 경우를 확인하는 남은 테스트를 찾을 수 있어요.
- workspace 테스트가 통과해요.
- 세 명세에 검증 표가 있어요. 여섯 명세 모두에 표가 있는지 확인해요.

## 결과

2026-10-10에 마쳤어요. 앞은 `4652d42`, 뒤는 `6e46ef1`(테스트를 바꾼 마지막 커밋)이에요.

### 결정

- 위 레이어의 코드에 적힌 규칙 다섯 개를 주인 크레이트로 옮겼어요. [0110](0110-job-line-tests.md)까지는 이런 규칙을 [0111](0111-web-and-screen-rules.md)로 보냈지만, 0111이 끝났고 뒤에 맡을 티켓이 없어서 이 티켓에서 옮겼어요(리드가 정함). 옮긴 함수는 모든 입력에 예전 코드와 같은 값을 돌려주고, 옮기기 전의 위 테스트가 새 함수로 먼저 통과한 뒤에 줄였어요.
- 웹에 남긴 규칙 사본 두 개는 옮기지 않았어요(리드가 정함). 아래 [남은 것](#남은-것)에 있어요.
- 지울 때는 같은 입력과 같은 기대값을 가진 남은 테스트를 열어 견줬어요. 남은 테스트에 경우나 기대값이 빠졌으면 그것을 아래 크레이트에 먼저 더하고 지웠고, 아래에 새로 쓴 테스트가 대신하는 것은 "내림"으로 셌어요.
- 검증 표의 칸과 단위는 [0101](0101-receive-line-tests.md)·0110과 같아요(사용자 결정, 2026-10-09).
- 표를 쓰며 찾은 명세와 코드의 차이는 사용자가 정했어요(2026-10-10). 명세에만 있고 만들지 않은 라이브러리 요구는 표에 `만들지 않았어요`로 적고 리팩터링 뒤 [0132](0132-library-unbuilt-parts.md)에서 만들어요. 웹 앱 표기 규칙과 다른 화면은 코드를 명세에 맞추는 [0133](0133-web-notation-rules.md), 설정의 두 결함은 [0134](0134-settings-version-and-first-run-badge.md)예요. 표에는 그때까지 `코드가 이 요구와 달라요`로 적었어요. 그 뒤 표를 마저 쓰며 찾은 같은 종류의 차이는 리드가 같은 결정에 따라 티켓에 더했고, [명세와 코드가 다른 곳](#명세와-코드가-다른-곳)에 누가 정했는지와 함께 있어요.

### 바꾼 것

규칙을 옮긴 커밋이에요. 각 커밋에 주인 크레이트의 새 테스트가 함께 있어요.

| 커밋 | 옮긴 규칙 | 새 테스트 |
| --- | --- | --- |
| `refactor(collect): decide an episode undo request once for the web and the worker (0113)` | 회차 되돌리기 요청의 판정. 웹의 `check_episode_undo`와 명령이 따로 정하던 것을 trss-collect `commands/episode_undo`의 `standing`이 정해요. | `episode_undo/standing_tests.rs` 3개 |
| `refactor(web): ask trss-anissia whether an unlisted anime's run is over in the week (0113)` | 이번 주 편성에서 Anissia가 더 싣지 않는 작품. 웹의 `week_at`이 trss-anissia `slot::run_over`를 불러요. | 없음(`slot.rs`의 기존 테스트) |
| `refactor(collect): name the season's holder in trss-collect for the web's link views (0113)` | 시즌을 잡은 구독은 규칙 ID가 가장 작은 구독이라는 것. 웹의 `link_views`가 trss-collect `season_holders`를 써요. | `season_anime_tests.rs` 1개. 규칙 ID가 무작위라 64번에 한 번 실패하던 웹 테스트를 대신해요. |
| `refactor(import): decide what the checked subscriptions of an import come to in trss-import (0113)` | 가져오기에서 체크한 구독 제안이 무엇이 되는지(`pick`, `settle`, `subscriptions`, `result`). 웹에서 trss-import `picks`로 옮겼고 Anissia에 묻는 `resolve`와 문장을 만드는 `why`는 웹에 남았어요. | `picks/tests.rs` 8개 |
| `refactor(collect): sort Transmission's torrents into the status counts in trss-collect, where the worker only calls it (0113)` | Transmission 토렌트 수를 상태별로 세는 것. worker 주기의 코드를 trss-collect `TransmissionLook::of`로 옮겼어요. | `store/status/tests.rs` 1개(일곱 상태 모두와 상태·해시가 없는 토렌트의 표) |
| `refactor(worker): name each queue's lock file with LockFile where the worker starts it, and drop the five per-crate lock_path_for wrappers (0113)` | 규칙은 아니에요. 큐마다의 `lock_path_for` 감싸개 다섯 개를 지우고, worker의 `main.rs`가 큐를 띄우는 자리에서 `LockFile::<종류>.path_for`를 불러요. | 없음 |

테스트를 나눈 커밋은 영역마다 하나이고(`test(web): …`, `test(worker): …`, `test(collect): …`, `test(library): …`, `test(anilist,anissia): …`, 모두 `(0113)`), 각 커밋의 본문에 지운 것과 내린 것이 있어요.

### 파일별 테스트 수

테스트를 뺀 파일이에요. "앞"은 `4652d42`, "뒤"는 `6e46ef1`의 `#[test]`·`#[tokio::test]` 개수예요. "더함"은 다른 레이어에서 이 파일로 온 테스트예요. "그중 줄임"은 남긴 테스트 중 위 레이어의 몫만 남기고 줄였거나 확인을 바꾼 것이고, 이름을 바꾼 것도 들어가요.

| 파일 | 앞 | 뒤 | 지움 | 내림 | 더함 | 남김 | 그중 줄임 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| trss-web `channels_api/tests.rs` | 26 | 26 | 0 | 0 | 0 | 26 | 5 |
| trss-web `commands_api/tests.rs` | 16 | 14 | 1 | 2 | 1 | 13 | 5 |
| trss-web `schedule_api/tests.rs` | 16 | 13 | 3 | 0 | 0 | 13 | 9 |
| trss-web `status_api/tests.rs` | 7 | 8 | 0 | 0 | 1 | 7 | 4 |
| trss-web `subscriptions_api/tests.rs` | 27 | 26 | 1 | 0 | 0 | 26 | 18 |
| trss-web `seasons_anissia_api/tests.rs` | 21 | 20 | 0 | 1 | 0 | 20 | 11 |
| trss-web `import_api/tests.rs` | 21 | 17 | 3 | 1 | 0 | 17 | 12 |
| trss-web `import_api/subscription_tests.rs` | 16 | 9 | 3 | 4 | 0 | 9 | 5 |
| trss-web `setup_api/tests.rs` | 11 | 5 | 3 | 3 | 0 | 5 | 3 |
| trss-web `settings_api/tests.rs` | 11 | 10 | 0 | 1 | 0 | 10 | 5 |
| trss-web `policy_api/tests.rs` | 4 | 4 | 0 | 0 | 0 | 4 | 2 |
| trss-web `seasons_api/tests.rs` | 14 | 13 | 0 | 1 | 0 | 13 | 10 |
| trss-web `artwork_api/tests.rs` | 12 | 12 | 0 | 0 | 0 | 12 | 2 |
| trss-web `library_api/tests.rs` | 8 | 8 | 1 | 0 | 1 | 7 | 3 |
| trss-web `video_check_api/tests.rs` | 1 | 1 | 0 | 0 | 0 | 1 | 1 |
| trss-worker `tests/it/collect_folder.rs` | 3 | 1 | 1 | 1 | 0 | 1 | 1 |
| trss-worker `tests/it/first_read.rs` | 8 | 0 | 0 | 8 | 0 | 0 | 0 |
| trss-worker `tests/it/title_waiting.rs` | 3 | 0 | 2 | 1 | 0 | 0 | 0 |
| trss-worker `tests/it/past_search.rs` | 36 | 1 | 8 | 27 | 0 | 1 | 1 |
| trss-worker `tests/it/library_watch.rs` | 25 | 2 | 7 | 16 | 0 | 2 | 2 |
| trss-worker `tests/it/live_watch.rs` | 14 | 5 | 0 | 9 | 0 | 5 | 1 |
| trss-worker `tests/it/season_link.rs` | 12 | 3 | 0 | 9 | 0 | 3 | 1 |
| trss-worker `tests/it/anissia_captions.rs` | 6 | 5 | 0 | 1 | 0 | 5 | 2 |
| trss-worker `tests/it/status_snapshots_from_cycle.rs` | 6 | 5 | 1 | 0 | 0 | 5 | 3 |
| trss-worker `tests/it/rules_preview_then_cycle.rs` | 3 | 0 | 2 | 1 | 0 | 0 | 0 |
| trss-worker `tests/it/channel_edit_then_cycle.rs` | 2 | 0 | 1 | 1 | 0 | 0 | 0 |
| trss-worker `tests/it/worker_process.rs` | 9 | 9 | 0 | 0 | 0 | 9 | 2 |
| trss-worker `tests/it/app_data_files.rs` | 2 | 0 | 2 | 0 | 0 | 0 | 0 |
| trss-worker `src/env.rs` | 7 | 7 | 0 | 0 | 0 | 7 | 0 |
| trss-worker `src/jobs.rs` | 4 | 4 | 0 | 0 | 0 | 4 | 1 |
| trss-worker `src/browser.rs` | 1 | 1 | 0 | 0 | 0 | 1 | 1 |
| trss-anissia `src/tests.rs` | 15 | 13 | 2 | 0 | 0 | 13 | 2 |
| trss-collect `anissia/tests.rs` | 10 | 9 | 1 | 0 | 0 | 9 | 0 |
| trss-collect `anissia/captions_tests.rs` | 23 | 22 | 1 | 0 | 0 | 22 | 1 |
| trss-library `artwork/tests.rs` | 35 | 36 | 0 | 0 | 1 | 35 | 2 |
| trss-anilist `src/lib.rs` | 2 | 3 | 0 | 0 | 1 | 2 | 0 |
| 합계 | 437 | 312 | 43 | 87 | 5 | 307 | 115 |

- "내림" 87개는 테스트 전체를 아래 크레이트로 내린 것이에요. 여러 테스트를 표 테스트 하나로 모은 곳이 있어서 받은 쪽의 수와 같지 않아요(worker `first_read.rs` 8개가 trss-collect 7개가 됐어요).
- 줄인 테스트는 상태 코드, 응답 모양, Host·Origin, 웹이 정하는 것, 그리고 worker에서는 잠금·순서·명령이 worker에 닿는 것을 남겼어요.

테스트를 받은 파일이에요. 기존 테스트에 확인을 더한 것은 세지 않았어요.

| 크레이트 | 파일 | 더함 |
| --- | --- | ---: |
| trss-collect | `cycle/first_read_tests.rs`(새 파일) 7, `season_link/tests.rs`(새 파일) 9, `revisions/past_tests.rs`(새 파일) 6, `commands/receive_past/tests.rs`(새 파일) 5, `commands/episode_undo/standing_tests.rs`(새 파일) 3 | 30 |
| trss-collect | `past_search/service.rs` 4, `commands/rule_archive/run_tests.rs` 3, `commands/anissia_captions.rs` 2, `cycle/tests.rs` 2, `past_search/run.rs` 2, `store/channels/import_tests.rs` 2, `commands/receive_once.rs` 1, `past_search/client/tests.rs` 1, `plan/preview/tests.rs` 1, `store/channels/season_anime_tests.rs` 1, `store/status/tests.rs` 1 | 20 |
| trss-library | `live/tests.rs`(새 파일) 9, `watch/scan_tests.rs`(새 파일) 6, `store/library/page.rs` 5, `store/setup/tests.rs` 3, `watch_rescan.rs` 3, `discovery.rs` 1, `seasons/tests.rs` 1, `store/library/detail.rs` 1, `store/library/tests.rs` 1, `store/seasons/tests.rs` 1 | 31 |
| trss-import | `picks/tests.rs`(새 파일) 8, `plan.rs` 2, `legacy.rs` 1 | 11 |
| trss-web | `past_search_api/tests.rs`(새 파일) 8, `watch_folders_api/tests.rs`(새 파일) 7, `commands_api/past_tests.rs`(새 파일) 3, `library_work_api/tests.rs` 1 | 19 |
| trss-core | `folder_check.rs` | 2 |
| trss-jobs | `tests/it/todo.rs` | 1 |
| 합계 | | 114 |

- 위 표의 파일에서 125개가 빠지고 아래에 114개가 늘어, workspace 테스트는 3,037개에서 3,026개가 됐어요.
- trss-library에는 감시 테스트가 함께 쓰는 바탕 `watch/fixture.rs`(테스트에서만 컴파일)를, trss-collect의 `World`에는 지난 회차 검색과 수집 폴더가 없는 주기의 도우미를, trss-web에는 가짜 트래커 `testing/past_search.rs`를 더했어요. trss-web의 trss-collect 개발 의존성이 `test-support`를 켜요.

### 지운 테스트와 남은 테스트

지운 43개와, 같은 입력과 기대값으로 그 경우를 확인하는 남은 테스트예요. 남은 테스트가 둘 이상인 행은 그 테스트들이 함께 지운 테스트의 경우를 덮어요.

| 지운 테스트 | 남은 테스트 |
| --- | --- |
| trss-web `commands_api/tests.rs` `a_retry_is_accepted_again_once_the_earlier_command_ended` | trss-core `commands/tests.rs` `a_second_open_command_for_the_same_subject_is_refused` |
| trss-web `schedule_api/tests.rs` `the_stand_in_an_import_keeps_is_not_anissias_off` | trss-collect `store/channels/import_subscriptions_tests.rs` `a_stand_in_sits_on_the_comments_weekday_or_in_the_other_tab`, trss-anissia `slot.rs` `an_anime_with_no_weekday_airs_in_no_week`, trss-collect `schedule/state.rs` `only_a_status_anissia_gave_as_off_counts`, trss-web `schedule_api/tests.rs` `a_stand_in_with_the_comments_weekday_shows_its_card_on_that_weekday` |
| trss-web `schedule_api/tests.rs` `a_card_leaves_once_the_end_date_has_passed` | trss-anissia `slot.rs` `nothing_airs_before_its_start_or_after_its_end` |
| trss-web `schedule_api/tests.rs` `an_anime_that_has_not_started_airs_on_its_start_day` | trss-anissia `slot.rs` `a_not_started_anime_airs_on_its_start_day_when_that_is_this_week`, `a_not_started_anime_airs_its_first_episode`, trss-collect `schedule/state.rs` `the_video_line_follows_the_library_then_the_download_then_the_clock` |
| trss-web `subscriptions_api/tests.rs` `a_work_without_a_connected_subscription_has_no_korean_title` | trss-web `library_work_api/tests.rs` `the_head_of_a_work_names_the_subscription_of_its_season_and_the_anissia_title` |
| trss-web `import_api/tests.rs` `a_channel_changed_after_the_review_is_refused_not_replaced` | trss-import `plan.rs` `every_conflicting_channel_needs_one_choice_that_matches_now`(웹의 `409`는 `skip_leaves_everything_and_choices_must_cover_every_conflict`에 남았어요) |
| trss-web `import_api/tests.rs` `differing_channel_folders_set_their_common_parent_and_prefix_the_rules` | trss-import `fit.rs` `with_no_collect_folder_differing_folders_share_their_common_ancestor`, `a_rule_saves_where_the_old_file_put_it`, trss-collect `store/channels/import_tests.rs` `an_import_sets_the_collect_folder_once_and_makes_it_an_automatic_watch_folder` |
| trss-web `import_api/tests.rs` `a_channel_folder_that_climbs_out_of_the_collect_folder_is_not_imported` | trss-import `fit.rs` `a_channel_folder_with_parent_components_is_reported_not_placed`와 그 이웃의 "고르는 데 끼지 않음" 테스트 |
| trss-web `import_api/subscription_tests.rs` `a_rule_saving_into_the_collect_folder_itself_is_offered_but_blocked` | trss-import `suggest/tests.rs` `a_rule_saving_into_the_collect_folder_itself_cannot_be_a_subscription`, `picks/tests.rs` `a_pick_that_cannot_be_a_subscription_is_reported_and_leaves_the_others_alone` |
| trss-web `import_api/subscription_tests.rs` `a_rule_saving_through_a_parent_folder_is_blocked_in_the_preview_and_refused_on_apply` | trss-import `suggest/tests.rs` `a_rule_saving_through_a_parent_folder_cannot_be_a_subscription`, `picks/tests.rs` `a_pick_that_cannot_be_a_subscription_is_reported_and_leaves_the_others_alone` |
| trss-web `import_api/subscription_tests.rs` `items_history_recorded_before_the_import_are_past_for_the_new_subscription` | trss-collect `plan.rs` `what_the_feed_held_at_the_first_read_is_past_for_a_subscription_only`, `store/history/tests.rs` `the_items_of_a_channels_first_read_are_marked_whatever_the_clock_does_after`, `store/channels/import_subscriptions_tests.rs` `a_replaced_rule_that_is_a_subscription_stays_as_it_is`(교체가 규칙 ID를 지키는 확인을 `6e46ef1`에서 더했어요) |
| trss-web `setup_api/tests.rs` `a_skip_can_be_taken_back_and_brings_the_checklist_back` | trss-library `store/setup/tests.rs` `taking_back_the_last_skip_brings_the_checklist_back`, `skipping_a_step_and_taking_it_back_is_per_step` |
| trss-web `setup_api/tests.rs` `a_channel_that_was_not_imported_does_not_end_the_import_step` | trss-library `store/setup/tests.rs` `a_channel_alone_does_not_finish_the_import_step_but_an_applied_import_does` |
| trss-web `setup_api/tests.rs` `unregistering_the_last_folder_does_not_bring_back_a_checklist_finished_by_the_data` | trss-library `store/setup/tests.rs` `registering_a_folder_finishes_the_folder_step_for_good`, `a_folder_that_comes_back_finishes_the_folder_step_too` |
| trss-web `library_api/tests.rs` `a_work_added_while_paging_does_not_repeat_what_was_seen` | trss-library `store/library/page.rs` `a_work_added_between_pages_is_never_a_repeat_and_one_ahead_of_the_cursor_is_not_shown` |
| trss-anissia `src/tests.rs` `a_429_without_a_retry_after_waits_a_minute_and_a_huge_one_is_cut_to_an_hour` | trss-core `response.rs` `retry_after_reads_seconds_and_falls_back_to_a_minute`, trss-anissia `src/tests.rs` `a_429_blocks_every_request_until_its_retry_after_has_passed` |
| trss-anissia `src/tests.rs` `a_search_waits_after_a_429_and_asks_again_only_when_the_wait_is_over` | trss-anissia `src/tests.rs` `a_429_blocks_every_request_until_its_retry_after_has_passed` |
| trss-worker `tests/it/collect_folder.rs` `a_retry_without_a_collect_folder_ends_with_the_reason_and_leaves_the_item` | trss-collect `commands/receive_once/ended_tests.rs`의 `Cause::NoCollectFolder` 경우 |
| trss-worker `tests/it/title_waiting.rs` `a_title_waiting_subscription_receives_nothing_and_offers_the_new_title_as_a_candidate` | trss-web `subscriptions_api/title_tests.rs` `a_new_title_becomes_one_candidate_and_a_second_episode_makes_no_second`, `a_rejected_candidate_is_gone_for_good_and_the_subscription_keeps_waiting`, trss-collect `rss/evaluate.rs` `title_waiting_rule_does_not_shadow_or_overlap` |
| trss-worker `tests/it/title_waiting.rs` `a_paused_waiting_subscription_offers_no_candidate` | trss-collect `subscriptions/candidates_tests.rs` `a_paused_or_archived_subscription_offers_nothing_and_a_resume_keeps_the_boundary` |
| trss-worker `tests/it/past_search.rs` `row_2_other_seasons_are_folded_batches_are_not_selected_and_only_the_revision_is` | trss-collect `past_search/judge/tests.rs` `another_season_a_batch_and_a_revision_are_told_apart`(빠진 회차 `13..=24` 확인을 더했어요) |
| trss-worker `tests/it/past_search.rs` `a_range_of_a_rule_with_no_season_folder_is_labelled_by_its_episodes` | trss-collect `episode_offset.rs`의 `range_label`·`place_of` 경우, trss-web `past_search_api/tests.rs` `the_range_is_labelled_by_the_folder_episodes_of_the_rules_conversion` |
| trss-worker `tests/it/past_search.rs` `row_4_a_video_of_unknown_version_with_another_crc_makes_the_result_unknown_and_unselected` | trss-collect `past_search/judge/tests.rs` `a_video_of_unknown_version_is_told_by_its_crc`, `past_search/service.rs` `a_video_of_unknown_version_is_told_by_the_crc_of_its_file_in_the_work_folder`(파일 CRC를 읽는 연결은 이 테스트에만 있어서 새로 썼어요) |
| trss-worker `tests/it/past_search.rs` `a_searched_revision_whose_download_stopped_is_received_again_with_retry` | trss-collect `revisions/retry_tests.rs` `a_stopped_revision_that_left_the_feed_is_received_again_with_retry`, `revisions/past_tests.rs` `a_revision_chosen_in_the_preview_replaces_the_video_after_it_is_received_and_checked` |
| trss-worker `tests/it/past_search.rs` `an_episode_whose_torrent_is_in_transmission_cannot_be_chosen_before_its_video_is_placed` | trss-collect `past_search/world.rs` `a_torrent_still_in_transmission_keeps_its_episode_before_the_video_is_placed`, trss-web `commands_api/past_tests.rs`의 `TheTorrentIsThere` 줄 |
| trss-worker `tests/it/past_search.rs` `a_work_folder_that_is_not_there_does_not_make_a_received_episode_gone` | trss-collect `past_search/world.rs` `a_folder_that_is_missing_or_not_read_whole_says_no_video_is_gone`, trss-web `commands_api/past_tests.rs`의 `TheFolderIsNotThere` 줄 |
| trss-worker `tests/it/past_search.rs` `a_folder_with_more_entries_than_are_looked_at_holds_a_received_episode` | trss-collect `past_search/world.rs` `a_folder_with_more_entries_than_are_looked_at_is_not_read_whole`, `a_folder_that_is_missing_or_not_read_whole_says_no_video_is_gone` |
| trss-worker `tests/it/past_search.rs` `a_batch_whose_torrent_is_removed_stays_held` | trss-collect `past_search/world.rs` `a_batch_or_an_unnumbered_item_is_never_gone` |
| trss-worker `tests/it/library_watch.rs` `files_that_do_not_fit_are_counted_on_the_work_and_hidden_ones_are_not_discovered` | trss-library `discovery.rs` `what_does_not_fit_is_counted_with_a_reason_and_hidden_things_are_skipped`(까닭 코드 여섯 개와 시즌 안의 숨은 폴더를 더했어요) |
| trss-worker `tests/it/library_watch.rs` `only_what_appears_after_the_first_check_gets_a_time_and_file_times_change_nothing` | trss-library `store/library/tests.rs` `the_first_scan_leaves_times_unknown_and_a_later_one_stamps_only_what_is_new`(네 번째 스캔을 더했어요) |
| trss-worker `tests/it/library_watch.rs` `a_deleted_episode_file_drops_out_and_a_deleted_work_folder_keeps_its_id` | trss-library `store/library/tests.rs` `gone_files_and_episodes_drop_and_a_gone_work_folder_keeps_its_id` |
| trss-worker `tests/it/library_watch.rs` `changing_the_collect_folder_swaps_its_watch_folder_and_adopts_one_registered_by_hand` | trss-library `automatic_watch.rs` `an_automatic_folder_the_settings_stopped_using_is_removed_and_a_changed_one_is_swapped`, `a_folder_registered_by_hand_at_the_same_place_is_kept_and_made_automatic` |
| trss-worker `tests/it/library_watch.rs` `archiving_into_an_existing_work_folder_merges_into_the_destinations_id` | trss-collect `commands/rule_archive/run_tests.rs` `archiving_a_work_keeps_its_id_under_the_archive_folder_and_merges_into_a_work_already_there`, trss-library `store/library/tests.rs` `following_a_move_keeps_the_id_and_a_merge_keeps_the_destinations` |
| trss-worker `tests/it/library_watch.rs` `starting_a_rule_for_a_work_in_both_folders_merges_the_archives_into_the_collect_folders_id` | trss-collect `commands/rule_archive/run_tests.rs` `starting_a_rule_for_an_archived_work_keeps_its_id_and_its_season_link_under_the_collect_folder` |
| trss-worker `tests/it/library_watch.rs` `an_empty_library_lists_no_works` | trss-web `library_api/tests.rs` `an_empty_library_answers_an_empty_page` |
| trss-worker `tests/it/status_snapshots_from_cycle.rs` `only_a_cycle_that_read_the_feed_leaves_a_read_day` | trss-collect `store/status/tests.rs` `a_read_on_the_newest_day_again_adds_nothing`, `a_day_read_twice_counts_once_and_a_failed_read_or_a_gap_counts_for_nothing`, `a_day_is_unix_ms_over_a_day` |
| trss-worker `tests/it/rules_preview_then_cycle.rs` `the_preview_of_every_rule_agrees_with_what_the_worker_did_with_the_same_items` | trss-collect `plan/preview/tests.rs` `the_preview_agrees_with_the_plan_for_every_recorded_title`(피드 A의 제목 여섯 개를 더했어요), `cycle/tests.rs` `a_cycle_judges_each_item_by_the_first_rule_that_takes_it_and_adds_it_there` |
| trss-worker `tests/it/rules_preview_then_cycle.rs` `what_the_preview_predicts_for_an_edit_is_what_the_next_cycle_does_after_it_is_saved` | trss-collect `plan/preview/tests.rs` `widening_a_phrase_shows_the_items_an_earlier_rule_takes`, `cycle/tests.rs` `a_new_matching_rule_turns_no_match_into_received_and_keeps_the_first_seen_time` |
| trss-worker `tests/it/channel_edit_then_cycle.rs` `an_exclude_added_through_the_api_is_used_by_the_next_cycle` | trss-collect `cycle/tests.rs` `a_cycle_judges_each_item_by_the_first_rule_that_takes_it_and_adds_it_there` |
| trss-worker `tests/it/app_data_files.rs` `app_data_files_cover_every_lock_and_the_wake_socket` | trss-core `lock.rs` `every_lock_file_has_its_own_name_next_to_the_database`, `access.rs` `the_checked_suffixes_hold_every_lock_file_and_the_wake_socket_once`, `wake.rs` `the_wake_socket_sits_next_to_the_database` |
| trss-worker `tests/it/app_data_files.rs` `app_data_folders_cover_the_receive_area_the_artwork_and_the_subtitle_files` | trss-core `app_data.rs` `the_write_check_names_every_folder` |
| trss-collect `anissia/tests.rs` `the_lock_file_sits_next_to_the_database_and_apart_from_the_other_queues` | trss-core `lock.rs` `every_lock_file_has_its_own_name_next_to_the_database` |
| trss-collect `anissia/captions_tests.rs` `the_lock_sits_next_to_the_database_and_apart_from_the_other_queues` | trss-core `lock.rs` `every_lock_file_has_its_own_name_next_to_the_database` |

### 명세를 고친 것

- 표를 쓰며 기록과 맞지 않는 명세 문장 네 곳을 고쳤어요(`docs(specs): correct observation records in the library and web app specs (0113)`). 라이브러리 명세의 방영 시각 표본 숫자는 기록이 없어서 [0051](../archive/tickets/3-subtitle-candidates-and-receiving/0051-airtime-episode-mapping.md)의 표본으로 바꿨고, 장치 번호가 바뀐 관찰은 표지 파일이 아니라 영상 파일의 것이라 그렇게 적었어요. 구독하지 않은 작품에 앱이 후보를 제안한다는 문장은 바로 아래의 사용자 결정(2026-10-02)과 맞지 않아 앱이 후보를 정하지 않는다고 고쳤어요. 웹 앱 명세의 접근 경계는 LAN에서 확인한 [0082](../archive/tickets/5-deployed-verification/0082-deployed-access-boundary.md)를 적었어요.
- 지우거나 옮긴 테스트를 인용하던 [RSS 수집 명세의 검증 표](../specs/collection.md#검증-표) 16곳, [자막 명세의 검증 표](../specs/subtitles.md#검증-표) 1곳, [티켓 목록](README.md)의 목표 1 요약을 지금 그 경우를 확인하는 테스트로 바꿨어요.
- [작업과 인증 명세의 검증 표](../specs/jobs.md#검증-표) 두 행은 실제 erulabo 수신의 Drive ID가 DB에 없었다고 적었지만, [0041](../archive/tickets/3-subtitle-candidates-and-receiving/0041-source-erulabo.md)은 DB 전체에서 받은 파일의 스냅샷 한 곳에서 찾았어요. 그렇게 고쳤어요. 자막 명세에서 이름에 backtick이 든 테스트 하나는 code span이 끊겨 보여서 두 겹 backtick으로 감쌌어요.

### 검증 표

검증 표는 세 명세의 끝에 있어요.

| 명세 | 요구 절 | 행 | 실제 사용에서 본 것이 있는 행 | 테스트가 있는 행 | 둘 다 없는 행 |
| --- | ---: | ---: | ---: | ---: | ---: |
| [라이브러리와 작품](../specs/library.md#검증-표) | 14 | 88 | 53 | 75 | 4 |
| [설정과 이전](../specs/settings.md#검증-표) | 14 | 111 | 25 | 43 | 65 |
| [웹 앱 공통](../specs/web-app.md#검증-표) | 6 | 48 | 22 | 27 | 17 |

- 둘 다 없는 행은 표 끝의 `확인하지 않은 요구`에 모았어요. 실제 사용에서 본 것은 날짜가 있는 관찰만 셌고, `근거만:`, `관련만:`, `참고(…):`는 세지 않았어요. 같은 방법으로 세면 작업과 인증 명세의 "실제 사용에서 본 것이 있는 행"이 0110의 기록과 같은 61행으로 나와요.
- 처음 쓴 표는 라이브러리 105 KB(두 부분 48.9 KB와 56.7 KB), 설정 67.8 KB, 웹 앱 55.5 KB였어요. 0110과 같이 테스트는 부분마다 가장 직접 겨눈 것 하나(행마다 셋까지), 관찰은 행마다 둘까지 남기고 확인하지 않은 것은 모두 남겨 라이브러리 66.1 KB, 설정 46.1 KB, 웹 앱 38.1 KB로 줄였어요. 설정 표는 명세 본문(36.8 KB)보다 커요. 요구 문장이 길고, 목표 6에서 만들 행 63개도 행마다 그 표시와 링크를 남겨서예요.
- 표를 쓴 agent가 이름만 보고 적은 테스트는 줄이는 단계에서 본문을 읽어, 그 행을 확인하지 않으면 빼거나 행을 확인하는 다른 테스트로 바꿨어요. 4·5단계가 지우거나 옮긴 테스트도 이때 지금 이름으로 바꿨어요. 실제 사용의 관찰은 인용한 티켓을 다시 읽어 날짜와 환경을 맞췄어요.
- [웹 앱 공통](../specs/web-app.md#검증-표), [라이브러리와 작품](../specs/library.md#검증-표), [RSS 수집](../specs/collection.md#검증-표), [자막](../specs/subtitles.md#검증-표), [작업과 인증](../specs/jobs.md#검증-표), [설정과 이전](../specs/settings.md#검증-표)의 여섯 명세에 모두 검증 표가 있어요.

### 명세와 코드가 다른 곳

표를 쓰며 명세에만 있고 만들지 않은 요구와 코드가 명세와 다른 곳을 찾았어요. 모두 리팩터링을 마친 뒤에 해요.

| 티켓 | 내용 | 누가 정했나 |
| --- | --- | --- |
| [0132](0132-library-unbuilt-parts.md) | 라이브러리 명세에만 있는 여섯 가지. 감시 폴더의 `연결한 작품 수`, 회차 목록의 방영 전 회차 줄, `최근 활동` 카드, 표지 자동 결정의 Anissia 힌트와 시즌별 관찰, `자막 확인 필요`의 원인 중 파일 누락·검증 실패, 극장판의 `본편` 이름이에요. | 앞의 셋은 사용자(2026-10-10), 뒤의 셋은 같은 결정에 따라 리드 |
| [0133](0133-web-notation-rules.md) | 웹 앱 공통 명세의 표기 규칙 일곱 가지. 상대 시각, ` · `로 잇기, `더 보기` 버튼, 빨간 할 일 배지, Transmission 관리 화면 링크, 올해가 아닌 날짜의 연도, `교체 승인` 배지의 물음표예요. | 앞의 다섯은 사용자(2026-10-10), 뒤의 둘은 같은 결정에 따라 리드 |
| [0134](0134-settings-version-and-first-run-badge.md) | 작품별 자막 형식 순서의 저장·삭제에 버전이 없는 것과 처음 실행 중의 할 일 배지예요. 설정 목록의 이름 두 가지(`파일 용량·정리`, `내보내기`·`가져오기`)는 그 티켓에서 사용자에게 물어요. | 사용자(2026-10-10) |
| [0135](0135-deploy-settings-item.md) | 설정의 `배포 설정`이 빈 상태만 보여주는 것이에요. | 기능을 더해야 맞출 차이는 리팩터링 뒤에 한다는 결정(2026-10-09)에 따라 리드 |

- 감시 폴더의 등록(`POST`)과 해제(`DELETE`)도 버전을 받지 않지만, 있는 값을 고치는 편집이 아니라서 [버전 계약](../specs/web-app.md#웹-명령과-상태-갱신)과 다르다고 보지 않았어요(리드가 정함).

### 실행 시간

`cargo test --locked --workspace -j 4`를 빌드가 끝난 뒤 다시 돌린 값이에요(2026-10-10, glibc, 개발 PC).

| 커밋 | 테스트 | 바이너리별 `finished in`의 합 | 가장 긴 바이너리 |
| --- | ---: | ---: | --- |
| `f10e9c9`–`100016e`(1단계 뒤) | 3,037 | 52.8초 | 10.05초 |
| `6e46ef1` | 3,026 | 43.0초 | trss-web 6.95초(452개), worker `tests/it` 6.65초(91개) |

`6e46ef1`에서 3,026개가 통과했고 실패는 0개, `#[ignore]`로 건너뛴 것은 14개예요.

worker 통합 테스트(`tests/it`)는 189개에서 91개가 됐어요. 0101과 0110이 나누지 않은 파일에서 98개가 빠졌어요.
trss-web의 `folders_on_different_filesystems_are_refused`는 `/proc`를 수집 폴더로 저장하던 두 번째 요청을 빼서, 저장 뒤의 작품 찾기가 `/proc`를 훑지 않아요. 뺀 요청은 보관 폴더 없이 저장되는 경우라 `the_archive_folder_may_be_left_empty`와 같았어요. 남은 확인은 다른 파일시스템의 보관 폴더가 400과 `다른 파일시스템` 문장으로 거절되고 설정이 저장되지 않는 것이고, 판정(`folder_check::conflict`)은 trss-core가 확인해요.

### 남은 것

- 웹 코드에 규칙의 사본이 두 곳 남았어요. 가져오기 미리보기의 `channel_view`는 지울 규칙·남길 규칙·제목 대기 구독을 저장소의 교체 코드와 따로 계산하고, 그 확인은 웹의 `import_api/tests.rs` 두 테스트에만 있어요. `policy_api`의 `within`은 `SettingsStore::put_policy`의 범위 검사를 웹의 문장을 내려고 다시 해요.
- 큐마다 잠금 파일 종류가 다른지는 테스트하지 않아요. 감싸개를 지운 뒤 worker의 `main.rs`가 큐를 띄우는 자리에서 `LockFile::Artwork`·`Seasons`·`Anissia`·`AnissiaCaptions`를 고르고, 섞이면 `main.rs`에서만 보여요. trss-core의 `LockFile::ALL`도 손으로 적은 목록이라 종류 하나를 빠뜨려도 실패하는 테스트가 없어요.
- worker 테스트의 공통 바탕 `tests/it/common/mod.rs`에 이제 쓰는 테스트가 없는 도우미가 남았을 수 있어요. `pub`이라 경고가 나지 않고, 이 티켓에서 정리하지 않았어요.
- 각 단계의 중간 커밋은 그 커밋이 바꾼 크레이트만 빌드하고 테스트했어요. workspace 전체는 단계마다 마지막 커밋에서 돌렸어요.
