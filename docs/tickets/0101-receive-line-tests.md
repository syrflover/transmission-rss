# 0101 수집 받기 줄기의 테스트를 나누고 RSS 수집 명세에 검증 표를 둬요

- 상태: 완료
- 출처: [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [영역별 검증 표](refactoring.md#영역별-검증-표), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0097](0097-shared-transmission-fake.md)(가짜 서버), [0098](0098-release-name-reading.md)(이름 읽기), [0099](0099-one-receive-path.md)(받기 경로), [0100](0100-worker-lock-release.md)(잠금 해제)

## 작업

수집 받기 줄기의 리팩터링을 마쳤으니 ADR 0015대로 테스트를 나눠요.
2026-10-07 worker의 해당 테스트는 한 번 받기 78개, 영상 수정본 86개, 수집 주기 41개, 영상 회차 변환 44개, 보관 폴더 이동 30개였어요.

- 규칙만 확인하는 테스트는 trss-collect로 내려요. 위층에서만 확인하던 규칙은 지우지 않고 내려요.
- trss-collect에 같은 경우의 테스트가 이미 있으면 위층 것을 지워요.
- worker에는 잠금, 주기와 명령의 순서, 동시 실행, 자식 프로세스, 중단 뒤 이어 하기, 웹에서 worker까지 이어지는 명령만 남겨요.
- 구조 조사는 그대로 겹치는 테스트를 30–60개, 내릴 규칙 테스트를 약 100개로 추정했어요. 실제로 나눌 때 테스트마다 몸통을 견줘 정해요.
- trss-web의 수집 API 테스트 중 수집 규칙을 다시 확인하는 것도 같이 나눠요. 웹에는 요청과 응답의 모양, 상태 코드, Host·Origin 검사만 남겨요.

[RSS 수집 명세](../specs/collection.md)에 요구별 검증 표를 둬요. 요구마다 실제 사용에서 관찰한 것(날짜와 티켓), 테스트로 확인한 것(테스트 이름이나 파일), 확인하지 않은 것을 나눠 적어요.

## 완료 기준

- 결과 절에 파일마다 앞뒤 테스트 수와, 지운 것·내린 것·남긴 것의 개수가 있어요.
- 지운 테스트마다 같은 경우를 확인하는 남은 테스트를 결과 절의 표에서 찾을 수 있어요.
- workspace 테스트가 통과하고, trss-worker와 trss-collect 테스트 바이너리의 앞뒤 실행 시간이 결과 절에 있어요.
- RSS 수집 명세에 검증 표가 있고, 확인하지 않은 요구가 표에 드러나요.

## 결과

### 결정

- 검증 표는 명세의 항목(글머리표)과 입력·결과 표의 줄마다 한 줄로 쓰고, 영역별 표를 명세 끝의 `검증 표` 절에 둬요(사용자 결정, 2026-10-09).
- 실행 중인 worker task를 중단하거나 worker를 죽인 뒤 이어 하는 테스트는, 무엇을 확인하든 worker에 남겨요. ADR 0015의 "중단 뒤 이어 하기"예요(리드가 정함). 그래서 영상 회차 변환의 되돌리기 테스트 6개가 worker에 남았어요.
- 이름표로 이어 하기는 방식마다 worker 테스트 하나를 남기고, 이어 받을지 정하는 규칙은 trss-collect로 내렸어요(리드가 정함).
- 이미 있는 토렌트에 항목 라벨을 붙이는 `add_item`과 피드에서 빠진 토렌트를 정리하는 `remove_stale`의 테스트는 그 코드가 있는 trss-transmission으로 내렸어요(ADR 0015, 리드가 정함).
- 웹의 "이미 놓였는지" 규칙(`in_place`)은 이 티켓에서 trss-web에 두고, 라우터를 거치지 않고 `Evidence::held`를 바로 부르는 테스트로 바꿨어요. 규칙을 trss-collect로 옮기는 것은 [0111](0111-web-and-screen-rules.md)의 후보로 남겨요(리드가 정함).
- 규칙 미리보기의 분류 테스트(trss-web `rules_api` 8개, `subscriptions_api` 1개, `title_tests`의 미리보기 부분)는 `build_preview`를 trss-collect로 옮기는 [0111](0111-web-and-screen-rules.md)에서 함께 내려요(리드가 정함).
- 내린 테스트는 trss-collect의 공유 테스트 환경 `test_world.rs`에서 돌아요. 가짜 Transmission, 파일로 둔 앱 DB, 피드 서버, 미디어 폴더를 갖고, worker의 `run_cycle` 순서를 따르는 `cycle()`과 `다시 받기`·되돌리기 명령을 돌리는 도우미가 있어요. `cycle()`은 테스트 코드에만 있고, 세션 설정, 상태 판 기록, 명령과 함께 쓰는 토렌트 차례는 따르지 않아요(리드가 정함).

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `test(collect): test a revision's decision at the cycle in trss-collect` | 공유 테스트 환경을 만들고 수정본 판정을 내렸어요. |
| `test(collect): test advancing a video replacement in trss-collect` | 수정본 대체의 단계 진행을 내렸어요. |
| `test(collect): test 다시 받기 of a video revision in trss-collect` | 수정본의 `다시 받기`를 내렸어요. |
| `test(collect): test the offset a cycle decides for a new season in trss-collect` | 회차 변환 값 정하기를 내렸어요. 경우 16가지를 표 테스트 하나로 묶었어요. |
| `test(collect): test the episode undo of a new season in trss-collect` | 회차 변환 되돌리기를 내렸어요. |
| `test(collect): test the first past item, the restarted cour and worth_offering in trss-collect` | 지난 항목의 첫 받기, 다시 1화부터 세는 분할 방영, `worth_offering`을 내렸어요. |
| `test(collect): test the rules of 다시 받기 in trss-collect, not through the worker` | `다시 받기`의 규칙을 내렸어요. |
| `test(collect): test what a cycle decides about an item in trss-collect, not through the worker` | 수집 주기의 항목 판단을 내렸어요. |
| `test(transmission): test the add of a torrent already there and the cleanup in trss-transmission, not through the worker` | `add_item`과 `remove_stale`를 trss-transmission으로 내렸어요. |
| `test(collect): test the archive move of a work folder in trss-collect, not through the worker` | 작품 폴더의 보관 이동을 내렸어요. |
| `test(web): leave the rules of the collect API tests to trss-collect, keep the web's part` | trss-web의 보관 제안, 수집 이력, 규칙, 제목 후보 테스트를 나눴어요. |
| `test(web): test the in-place rule on Evidence::held, not through the router` | `in_place` 규칙 테스트를 `Evidence::held`를 바로 부르는 테스트로 바꿨어요. |
| `docs(collect): add the verification table to the RSS collection spec`(이 결과를 담은 커밋) | [RSS 수집 명세의 검증 표](../specs/collection.md#검증-표)와 이 결과예요. |

제품 코드는 테스트 모듈 선언과 테스트에 필요한 공개 범위 말고는 바꾸지 않았어요. 내린 테스트 중 trss-collect 코드에서 실패한 것은 없었어요.

검증 표는 요구 절 11개에 맞춘 표 11개, 120행이에요. 테스트가 있는 행은 113행, 실제 사용에서 본 것이 있는 행은 37행이고, 둘 다 없는 7행은 표 끝의 `확인하지 않은 요구`에 모았어요. 처음 쓴 표는 150 KB로 명세 본문(72 KB)의 두 배였어요. 그래서 테스트는 요구를 가장 직접 겨눈 것만 남기고 확인하지 않은 것은 모두 남겨 88 KB로 줄였어요.

### 파일별 테스트 수

테스트를 뺀 파일이에요. "앞"은 `31202ad`, "뒤"는 `a4bb97c`의 `#[test]`·`#[tokio::test]` 개수예요. "더함"은 이 티켓에서 그 파일에 새로 쓴 테스트예요.

| 파일 | 앞 | 뒤 | 지움 | 내림 | 남김 | 더함 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| trss-worker `tests/it/video_revisions.rs` | 86 | 1 | 1 | 84 | 1 | 0 |
| trss-worker `tests/it/episode_offset.rs` | 45 | 6 | 0 | 39 | 6 | 0 |
| trss-worker `tests/it/receive_once.rs` | 79 | 21 | 13 | 46 | 20 | 1 |
| trss-worker `tests/it/worker_cycle.rs` | 47 | 19 | 4 | 24 | 19 | 0 |
| trss-worker `tests/it/archive_move.rs` | 40 | 13 | 10 | 18 | 12 | 1 |
| trss-worker 다섯 파일 합계 | 297 | 60 | 28 | 211 | 58 | 2 |
| trss-web `archive_api/tests.rs` | 15 | 6 | 9 | 0 | 6 | 0 |
| trss-web `history_api/tests.rs` | 8 | 7 | 0 | 1 | 7 | 0 |
| trss-web `rules_api/tests.rs` | 34 | 39 | 1 | 1 | 32 | 7 |
| trss-web `subscriptions_api/title_tests.rs` | 14 | 11 | 3 | 0 | 11 | 0 |
| trss-web `in_place/tests.rs` | 18 | 15 | 1 | 1 | 16 | 4 |

- worker의 "남김" 중 6개는 프로세스에 속한 부분만 남기고 줄였어요(`receive_once.rs` 3개, `worker_cycle.rs` 2개, `archive_move.rs` 1개). 줄이기 전에 규칙 부분을 trss-collect에 같은 입력으로 썼어요.
- worker의 "더함"은 `receive_once`의 결과를 적지 못한 명령이 다음 확인에서 다시 실행되는 테스트와, 보관 폴더로 옮겨지기를 기다리는 항목을 수집 주기 보고에 세는 테스트예요. 둘 다 내린 테스트가 함께 확인하던 프로세스 부분이에요.
- `in_place/tests.rs`의 "남김" 16개 중 12개는 라우터를 거치던 테스트로, 같은 경우를 `Evidence::held`를 바로 부르는 테스트 7개로 묶어 다시 썼어요. 그래서 이 파일의 "남김"과 "더함"을 더하면 "뒤"보다 5개 많아요. "더함" 4개는 수정본 `다시 받기` 쪽 웹 테스트예요.
- 조사의 추정(그대로 겹치는 테스트 30–60개, 내릴 테스트 약 100개)과 달리, worker 다섯 파일에서 지운 것은 28개, 내린 것은 211개였어요. 조사에서 지울 후보였던 테스트 중 몸통을 견주어 남는 테스트가 확인하지 않는 입력이나 결과가 있으면 내렸어요.

테스트를 받은 파일이에요.

| 크레이트 | 파일 | 더함 |
| --- | --- | ---: |
| trss-collect | `revisions/` 아래 `deciding_tests.rs` 8, `replacing_tests.rs` 20, `several_tests.rs` 8, `looks_tests.rs` 15, `away_tests.rs` 7, `retry_tests.rs` 21 | 79 |
| trss-collect | `store/revisions/tests.rs` 1, `store/history/tests.rs` 2 | 3 |
| trss-collect | `offsets/tests.rs` 3, `episode_offset.rs` 2 | 5 |
| trss-collect | `commands/episode_undo/` 아래 `renaming_tests.rs` 9, `waiting_tests.rs` 3, `rows_tests.rs` 4 | 16 |
| trss-collect | `commands/receive_once/` 아래 `retry_tests.rs` 6, `unanswered_tests.rs` 5, `ended_tests.rs` 6, `link_tests.rs` 1, `past_tests.rs` 1, `name_tests.rs` 2, `offset_tests.rs` 1 | 22 |
| trss-collect | `receive/tests.rs` 4, `cycle/tests.rs` 13 | 17 |
| trss-collect | `commands/rule_archive/` 아래 `run_tests.rs` 7, `work_folder/tests.rs` 8 | 15 |
| trss-transmission | `item_tests.rs` | 3 |
| trss-web | `commands_api/tests.rs` 5, `library_work_api/tests.rs` 3, `subscriptions_api/tests.rs` 1, `todo_api/tests.rs` 4 | 13 |

### 지운 테스트와 남은 테스트

남은 테스트의 경로는 크레이트의 `src/`(worker는 `tests/it/`) 기준이에요. "줄"은 표 테스트의 한 경우예요.

| 지운 테스트 | 남은 테스트 | 같은 경우인 까닭 |
| --- | --- | --- |
| trss-worker `video_revisions.rs` `a_lower_revision_skipped_for_another_reason_stays_skipped` | trss-collect `store/revisions/tests.rs` `a_revision_skipped_for_a_higher_one_comes_back_when_that_one_fails` | 둘 다 다른 까닭으로 건너뛴 줄(`overtaken_by`가 NULL)을 두고 더 높은 수정본을 실패시켜, 그 줄이 `Skipped`로 남고 돌아오지 않는 것을 확인해요. worker 테스트는 그 둘레에 주기를 돌렸을 뿐이에요. |
| trss-worker `receive_once.rs` `the_same_command_id_delivered_twice_adds_one_torrent_and_returns_the_result` | trss-web `commands_api/tests.rs` `the_same_command_delivered_twice_is_stored_once_and_answered_alike`, trss-core `commands/tests.rs` `the_same_id_with_the_same_content_returns_the_stored_command`, trss-worker `receive_once.rs` `two_workers_never_run_one_command_twice`, `a_command_accepted_before_a_restart_runs_once_after_it` | 같은 명령 id를 두 번 보내면 한 번 저장되고 같게 답해요(202 뒤 200). 명령이 한 번만 실행되는 것은 남은 worker 테스트가 확인해요. |
| trss-worker `receive_once.rs` `the_same_command_id_with_another_item_is_refused` | trss-web `commands_api/tests.rs` `the_same_id_for_another_item_is_refused`, trss-core `commands/tests.rs` `the_same_id_with_other_content_is_refused_and_changes_nothing` | 같은 id에 다른 내용이면 409이고, 저장된 명령은 처음 것 그대로예요. |
| trss-worker `receive_once.rs` `a_request_naming_a_folder_is_refused_and_nothing_is_accepted` | trss-web `commands_api/tests.rs` `a_request_that_names_a_folder_is_refused_and_nothing_is_stored` | 같은 폴더를 적은 요청이 400 `invalid`를 받고 아무것도 저장되지 않아요. |
| trss-worker `receive_once.rs` `after_a_lost_answer_the_command_is_looked_up_by_the_same_id` | trss-web `commands_api/tests.rs` `a_command_is_accepted_pending_and_can_be_read_back`, `an_unknown_command_id_is_not_found`, trss-worker `receive_once.rs` `a_failed_item_is_added_again_by_the_worker_and_the_screen_reads_the_end` | 명령을 id로 다시 읽는 세 경우(대기, 끝남, 모르는 id)가 같아요. 끝난 명령을 읽는 것은 남은 worker 테스트에 있어요. |
| trss-worker `receive_once.rs` `items_no_rule_picked_and_items_transmission_holds_are_not_retried` | trss-web `commands_api/tests.rs` `an_item_that_cannot_be_retried_is_refused_with_its_reason_and_nothing_is_stored` | `no_match` 항목과 `received` 항목을 같은 문장의 400으로 거절하고 아무것도 저장하지 않아요. |
| trss-worker `receive_once.rs` `an_item_that_failed_without_any_rule_is_not_said_to_belong_to_another_rule` | trss-collect `commands/receive_once.rs` `only_an_item_a_rule_picked_and_failed_to_add_with_an_active_rule_can_be_retried`, `commands/receive_once/ended_tests.rs` `an_accepted_retry_found_ineligible_ends_at_once_and_leaves_the_item_as_it_was`의 `Cause::NoRuleRecorded` 줄 | 규칙이 기록되지 않은 `add_failed` 항목을 `OtherRule`이 아닌 `NoRule`로 판정하고, 명령은 곧바로 끝나며 항목은 그대로예요. |
| trss-worker `receive_once.rs` `a_retry_with_no_collect_folder_ends_at_once_and_leaves_the_item_as_it_was` | trss-collect `commands/receive_once/ended_tests.rs` `an_accepted_retry_found_ineligible_ends_at_once_and_leaves_the_item_as_it_was`의 `Cause::NoCollectFolder` 줄, trss-worker `collect_folder.rs` `a_retry_without_a_collect_folder_ends_with_the_reason_and_leaves_the_item` | `수집 폴더`가 없으면 명령이 그 까닭으로 `failed`로 끝나고, 아무것도 더하지 않으며 항목은 그대로예요. |
| trss-worker `receive_once.rs` `an_item_first_seen_while_a_rule_was_paused_is_left_to_the_user_when_it_resumes` | trss-collect `commands/receive_once/past_tests.rs` `a_cycle_leaves_the_past_to_the_user_and_the_user_receives_it_with_the_rule`의 `Made::Resumed` 줄, `plan.rs` `a_rule_holds_back_what_came_before_the_later_of_its_subscription_and_its_resume` | 규칙이 멈춘 동안 처음 본 항목을 주기는 `no_match`로 두고, 사용자의 받기가 그 규칙으로 받아요. |
| trss-worker `receive_once.rs` `a_rule_that_was_never_paused_takes_the_recorded_items_as_before` | trss-collect `commands/receive_once/past_tests.rs` 같은 테스트의 `Made::PlainRule` 줄 | 항목이 기록된 뒤 만든 일반 규칙이 다음 주기에 두 항목을 그 규칙의 `received`로 받아요. |
| trss-worker `receive_once.rs` `a_rerun_after_a_cycle_renamed_the_torrent_leaves_its_name` | trss-collect `receive/tests.rs` `a_command_run_again_names_a_file_from_the_name_it_recorded_or_found`의 "the name recorded, a cycle renamed the file meanwhile" 줄, `a_rename_whose_answer_was_lost_does_not_convert_the_episode_twice` | 이름이 기록되어 있고 그사이 주기가 파일 이름을 바꿨으면 이름 바꾸기를 다시 부르지 않아요. |
| trss-worker `receive_once.rs` `a_rerun_renames_from_the_recorded_name_a_file_the_earlier_start_did_not` | trss-collect `receive/tests.rs` 같은 테스트의 "the name recorded, the earlier start did not rename" 줄 | 앞선 실행이 바꾸지 못한 파일을 기록된 이름에서 `Show S01E16.mkv`로 바꿔요. |
| trss-worker `receive_once.rs` `a_start_after_one_that_added_nothing_converts_a_release_in_another_seasons_form` | trss-collect `receive/tests.rs` 같은 테스트의 "no name recorded, nothing added before" 줄 | 기록된 이름이 없고 앞서 더한 것이 없을 때(`added_before`가 거짓) 폴더에 있는 다른 시즌 형식의 릴리스를 바꾸는 같은 입력이에요. |
| trss-worker `receive_once.rs` `a_torrent_whose_hash_was_never_learned_stays_while_its_item_is_in_a_feed` | trss-collect `cycle/tests.rs` `a_torrent_labelled_for_an_item_stays_while_the_item_is_in_a_feed_and_goes_after` | 이력에 해시가 없고 항목 라벨을 단 봇 토렌트가, 항목이 피드에 있는 동안 남고 빠진 뒤 지워져요. 남은 테스트가 이미 같은 몸통이었어요. |
| trss-worker `worker_cycle.rs` `a_named_torrent_with_a_three_digit_episode_is_not_renamed` | trss-collect `receive/tests.rs` `a_trname_name_is_told_apart_from_a_release_name` | 규칙 폴더의 `Slime S04E105.mkv`를 이미 바꾼 이름으로 읽는 같은 `looks_renamed` 입력이에요. |
| trss-worker `worker_cycle.rs` `nothing_is_removed_when_every_feed_was_over_the_size_cap` | trss-worker `worker_cycle.rs` `nothing_is_removed_when_no_feed_could_be_read`, trss-collect `feed.rs` `a_body_over_the_cap_is_refused_as_too_large`, `a_declared_length_over_the_cap_is_refused_before_reading` | 지우지 않는 조건은 읽은 채널이 0개인 것(`channels_read == 0`)이고 읽지 못한 까닭만 달라요. 크기 상한을 넘는 피드가 읽기 실패가 되는 것은 `feed.rs`가 확인해요. |
| trss-worker `worker_cycle.rs` `nothing_is_removed_when_every_feed_failed_even_for_unknown_origin` | trss-worker `worker_cycle.rs` `nothing_is_removed_when_no_feed_could_be_read` | 같은 조건과 같은 확인(`channels_read == 0`, 지우기 호출 없음)이고, 남은 테스트는 피드 서버가 내려간 경우예요. |
| trss-worker `worker_cycle.rs` `archived_and_title_waiting_rules_receive_nothing_and_ids_map_back` | trss-collect `plan.rs` `archived_rules_never_apply`, `a_rule_without_a_match_phrase_matches_nothing_and_does_not_shadow`, `evaluation_numbers_map_back_to_stored_rule_ids` | 보관한 규칙, 제목을 기다리는 규칙, 규칙 번호의 대응을 plan에서 바로 확인해요. |
| trss-worker `archive_move.rs` `a_work_folder_that_is_a_link_out_of_the_collect_folder_is_not_moved` | trss-collect `commands/rule_archive/work_folder/tests.rs` `links_out_of_the_two_folders_are_refused_and_links_inside_are_not` | 남은 테스트의 첫 경우와 같아요. 수집 폴더 밖을 가리키는 링크인 작품 폴더를 그 링크를 밝히는 까닭으로 거절해요. |
| trss-worker `archive_move.rs` `a_torrent_found_through_a_link_into_the_work_folder_stops_the_move` | trss-collect `commands/rule_archive/work_folder/tests.rs` `torrents_are_matched_by_where_their_folders_really_are` | 같은 `Alias/Season 02` 경우예요. |
| trss-worker `archive_move.rs` `a_torrent_folder_with_dot_dot_is_judged_by_where_it_lands` | trss-collect `commands/rule_archive/work_folder/tests.rs` `torrents_are_matched_by_where_their_folders_really_are` | `..`가 작품 폴더 밖과 안에 닿는 같은 두 경우예요. |
| trss-worker `archive_move.rs` `a_torrent_reached_through_a_link_inside_the_work_folder_stops_the_move` | trss-collect `commands/rule_archive/work_folder/tests.rs` `torrents_are_matched_by_where_their_folders_really_are` | 같은 `X/Borrowed` 경우예요. |
| trss-worker `archive_move.rs` `another_file_of_the_same_name_only_at_the_destination_stops_the_move` | trss-collect `commands/rule_archive/work_folder/tests.rs` `a_torrent_split_between_the_two_folders_is_named_without_asking_to_clear_a_side`의 셋째 블록 | 같은 파일과 같은 크기 차이를 충돌로 밝혀요. |
| trss-worker `archive_move.rs` `a_torrent_split_between_the_two_folders_stops_the_move_without_asking_to_clear_a_side` | trss-collect `commands/rule_archive/work_folder/tests.rs` 같은 테스트의 첫째 블록 | 같은 경우예요. worker 테스트만 확인하던 까닭 문장의 세 부분을 지우기 전에 남은 테스트에 더했어요. |
| trss-worker `archive_move.rs` `a_torrent_whose_files_were_all_moved_by_hand_is_pointed_at_them` | trss-collect `commands/rule_archive/work_folder/tests.rs` `a_torrent_file_the_destination_has_refuses_the_move_unless_it_is_only_there`의 `e03` 블록, `a_move_stopped_during_the_renames_finishes_the_rest_when_it_runs_again` | 파일이 모두 옮겨진 토렌트를 그 파일로 가리키는 같은 규칙이에요. |
| trss-worker `archive_move.rs` `a_rule_for_a_work_that_is_not_archived_is_made_on_and_collects_at_once` | trss-collect `commands/rule_archive/start_tests.rs` `a_rule_waits_for_its_work_folder_only_when_the_archive_folder_holds_it`, `a_start_with_no_archive_folder_or_no_work_folder_just_turns_the_rule_on`, trss-web `rules_api/tests.rs` `a_rule_for_a_work_in_the_archive_folder_is_made_paused_with_its_start_open`의 `Elsewhere` 경우 | 같은 판정과 같은 웹 응답이에요. 켜진 규칙이 곧바로 받는 것을 한 테스트가 다 확인하지는 않고, 보관 폴더에서 온 작품에 주기가 받는 것은 trss-worker `archive_move.rs` `a_new_rule_for_an_archived_work_collects_only_after_its_folder_came_into_the_collect_folder`가 확인해요. |
| trss-worker `archive_move.rs` `a_cycle_also_heals_a_work_split_across_both_folders_by_merging_the_archive_into_the_collect_one` | trss-collect `commands/rule_archive/start_tests.rs` `a_rule_waits_for_its_work_folder_only_when_the_archive_folder_holds_it`(두 폴더에 다 있는 경우), `cycle/tests.rs` `a_cycle_leaves_an_item_of_a_rule_whose_work_folder_is_archived_and_brings_the_folder_in_first`, `commands/rule_archive/work_folder/tests.rs` `a_season_merges_beside_the_archived_one_and_new_folders_take_the_parents_owner` | 판정, 주기의 반응, 합치기가 같아요. 한 테스트가 아니라 세 테스트를 합쳐야 다 덮어요. |
| trss-worker `archive_move.rs` `a_cycle_with_a_move_already_open_for_the_rule_leaves_the_item_and_stores_nothing_more` | trss-collect `commands/rule_archive/start_tests.rs` `receiving_into_an_archived_work_folder_pauses_the_rule_and_stores_a_start_once`, `cycle/tests.rs` `a_cycle_leaves_an_item_of_a_rule_whose_work_folder_is_archived_and_brings_the_folder_in_first`의 규칙을 다시 켠 블록 | 이동 명령이 이미 열려 있으면 주기가 항목을 그대로 두고 명령을 더 저장하지 않으며, 먼저 옮기라고 묻지도 않아요(`moving_first`가 거짓). |
| trss-web `archive_api/tests.rs` `an_end_date_that_has_not_passed_gives_no_suggestion` | trss-collect `archive_suggestions/tests.rs` `an_end_date_that_is_today_or_ahead_has_not_ended_the_anime` | 같은 입력이에요. |
| trss-web `archive_api/tests.rs` `an_anime_without_an_end_date_is_suggested_once_the_refresh_found_it_unlisted` | trss-collect `archive_suggestions/tests.rs` `an_anime_without_an_end_date_ends_when_anissia_no_longer_lists_it` | 같은 경우들이에요. 응답의 `unlisted` 종류는 trss-web `archive_api/tests.rs` `keeping_collecting_hides_the_same_ground_but_not_a_new_one`이 확인해요. |
| trss-web `archive_api/tests.rs` `three_weeks_without_a_new_item_is_not_enough_and_four_weeks_is` | trss-collect `archive_suggestions/tests.rs` `three_weeks_after_the_last_receive_is_not_quiet_and_four_weeks_is`, `the_quiet_ground_says_where_the_weeks_count_from` | 같은 경계예요. 응답의 `quiet`·`since`·`key`는 trss-web `archive_api/tests.rs` `keeping_collecting_hides_the_same_ground_but_not_a_new_one`이 확인해요. |
| trss-web `archive_api/tests.rs` `weeks_the_channel_could_not_be_read_are_not_quiet_weeks` | trss-collect `archive_suggestions/tests.rs` `weeks_the_channel_could_not_be_read_are_not_quiet_weeks`, `the_ground_appears_when_reading_has_made_up_the_four_weeks`, `store/status/tests.rs` `a_day_read_twice_counts_once_and_a_failed_read_or_a_gap_counts_for_nothing` | 같은 규칙이고, 남은 테스트는 읽은 날의 하한을 바로 주어요. 하한을 세는 것은 `store/status/tests.rs`가 확인해요. |
| trss-web `archive_api/tests.rs` `a_worker_that_was_off_for_four_weeks_suggests_nothing_when_it_starts_again` | trss-collect `archive_suggestions/tests.rs` `the_clock_still_has_to_reach_four_weeks_when_the_days_are_there`, `store/status/tests.rs` `a_day_read_twice_counts_once_and_a_failed_read_or_a_gap_counts_for_nothing` | 읽지 못한 기간을 세지 않는 같은 논리예요. |
| trss-web `archive_api/tests.rs` `a_channel_never_read_for_28_days_gives_no_quiet_ground_at_all` | trss-collect `archive_suggestions/tests.rs` `a_channel_that_has_not_been_read_for_28_days_gives_no_quiet_ground`, `the_day_of_the_moment_itself_is_not_one_of_the_28` | 같은 27일·28일 경계예요. |
| trss-web `archive_api/tests.rs` `a_new_item_that_matches_the_rule_ends_the_quiet_whatever_became_of_it` | trss-collect `archive_suggestions/tests.rs` `a_recent_item_that_matches_the_rule_is_a_new_item_whatever_became_of_it` | 규칙에 맞는 제목과 맞지 않는 제목이 같아요. |
| trss-web `archive_api/tests.rs` `a_paused_rule_is_suggested_but_an_archived_one_and_a_waiting_subscription_are_not` | trss-collect `archive_suggestions/tests.rs` `a_paused_rule_is_suggested_on_the_same_ground`, `an_archived_rule_and_a_subscription_waiting_for_its_title_are_never_suggested` | 멈춘 규칙, 보관한 규칙, 제목을 기다리는 구독의 같은 상태들이에요. |
| trss-web `archive_api/tests.rs` `suggestions_are_listed_in_the_order_of_the_rules` | trss-collect `archive_suggestions/tests.rs` `suggestions_follow_the_channels_and_their_rules` | 같은 순서 규칙이에요. |
| trss-web `rules_api/tests.rs` `an_excluded_item_and_an_archived_rule_make_no_overlap` | trss-collect `plan.rs` `evaluation_lists_the_later_active_rules_that_also_match_by_id` | 제외한 항목과 보관한 규칙의 같은 입력에, 겹침이 비는 같은 결과예요. 웹 응답으로 이어지는 것은 trss-web `rules_api/tests.rs` `overlap_is_shown_only_on_the_rule_an_earlier_rule_shadows`가 확인해요. |
| trss-web `subscriptions_api/title_tests.rs` `titles_seen_while_nothing_waited_make_no_candidate` | trss-collect `subscriptions/candidates_tests.rs` `nothing_is_a_candidate_while_no_subscription_waits_for_a_title`, `a_work_seen_before_the_subscription_waited_is_not_new` | 같은 입력이에요. |
| trss-web `subscriptions_api/title_tests.rs` `a_channel_without_a_waiting_subscription_has_no_candidates` | trss-collect `subscriptions/candidates_tests.rs` `nothing_is_a_candidate_while_no_subscription_waits_for_a_title`, `a_paused_or_archived_subscription_offers_nothing_and_a_resume_keeps_the_boundary`, `candidates_come_newest_work_first_and_a_title_without_a_work_is_left_out` | 같은 상태와 순서예요. 다른 채널의 항목이 후보가 되지 않는다는 확인은 지우기 전에 trss-web `subscriptions_api/title_tests.rs` `a_new_title_becomes_one_candidate_and_a_second_episode_makes_no_second`로 옮겼어요. |
| trss-web `subscriptions_api/title_tests.rs` `a_title_another_rule_already_handles_is_no_candidate` | trss-collect `subscriptions/candidates_tests.rs` `a_work_another_rule_handles_is_not_a_candidate` | 같은 입력이에요. |
| trss-web `in_place/tests.rs` `a_retry_of_a_version_unknown_item_the_listing_places_is_refused` | trss-web `in_place/tests.rs` `a_retry_of_a_revision_the_episode_holds_already_is_refused_with_the_reason`의 `placed` 경우 | 목록에서 회차 이름에 놓인 더 높은 항목이 있을 때 `버전 미상` 항목의 `다시 받기`를 같은 문장의 400으로 거절하고 아무것도 저장하지 않아요. 지운 테스트를 남은 테스트의 한 경우로 합쳤어요. |

### 명세와 코드가 다른 곳

표를 쓰며 명세의 문장 14곳이 코드와 다르다고 보고 하나씩 확인했어요(2026-10-09).

- 7곳은 명세가 코드의 동작이나 자리를 틀리게 적었어요. 규칙 미리보기의 입력, 받은 영상의 CRC32를 두는 곳, 이동 명령을 만드는 쪽, 체크한 지난 항목의 순서, 회차 변환 되돌리기의 대상, 수정본 대체의 거절 경우, 폴더의 쓰기 검사예요. 명세를 고쳤고, 쓰기 검사는 웹이 폴더를 읽기 전용으로 붙여 저장할 때 볼 수 없으므로 명세를 지금 동작에 맞추기로 했어요(사용자 결정, 2026-10-09).
- 구독을 다른 Anissia 작품으로 다시 잇는 길은 명세에만 있고 코드에 없어요. 만들기로 하고 [0129](0129-relink-subscription.md)로 남겼어요(사용자 결정, 2026-10-09).
- 6곳은 명세를 고칠 일이 아니었어요. `받는 중`의 쓰임과 보관한 규칙의 `자막 받기`는 명세대로이고, 앱 YAML 내보내기는 아직 만들지 않은 [목표 6](README.md#6-앱-yaml-내보내기와-가져오기)이에요. 릴리스 이름 묶음 테스트의 범위와 지난 티켓 두 곳의 기록은 명세의 요구가 아니에요.

### 실행 시간

2026-10-09에 `-j 4`로 빌드한 뒤 다시 돌린 한 번씩의 `finished in`이에요. 앞은 `31202ad`, 뒤는 `a4bb97c`이고, 단계마다 다른 agent가 측정했고, 다른 작업과 겹쳤는지는 확인하지 않았어요.

| 테스트 바이너리 | 앞 | 뒤 |
| --- | --- | --- |
| trss-worker `tests/it` | 422개, 11.48초 | 185개, 8.04초 |
| trss-collect 라이브러리(`test-support`) | 521개 통과·1개 무시, 1.80초 | 678개 통과·1개 무시, 2.46초 |

worker 바이너리의 남은 8초는 남긴 프로세스 테스트가 차지해요. 마지막 단계에서 테스트 27개를 뺐을 때 줄어든 시간은 약 0.2초였어요.

### 검증한 것

- `cargo test --locked --workspace -j 4`는 2,882개에서 2,807개가 되었고 모두 통과했어요. 실패는 0개, 무시는 14개예요. 커밋마다 돌렸고, 알려진 흔들리는 테스트 말고는 다시 돌린 것이 없어요.
- 2026-10-09에 trss-browser의 `a_reset_makes_the_starts_under_way_give_up`과 `a_start_gives_the_run_a_profile_and_a_downloads_folder`가 부하 아래에서 한 번씩 실패했다가 다시 돌리면 통과했어요. 이 티켓이 바꾸지 않은 크레이트예요.
- `cargo fmt --all --check`는 깨끗하고, 바꾼 크레이트의 clippy(`--all-targets`, trss-collect와 trss-transmission은 `test-support` 포함)에 경고가 없어요.
- 일부 새 테스트는 대상 코드를 일부러 틀리게 바꿔 실패하는 것을 확인했어요. `add_item`과 `remove_stale`의 네 가지 변경은 각각 새 테스트 하나를 실패시켰고, `in_place`의 두 가지 변경도 그랬어요. 나머지 내린 테스트는 worker에서 통과하던 입력과 기대를 그대로 옮긴 것으로 확인했어요.

### 남은 것

- worker의 `collect_folder.rs`, `first_read.rs`, `title_waiting.rs`, `past_search.rs`는 수집 영역이지만 이 티켓의 다섯 파일에 들지 않아 나누지 않았어요. [0113](0113-remaining-area-tests.md)에서 나눠요. `collect_folder.rs`의 `a_retry_without_a_collect_folder_ends_with_the_reason_and_leaves_the_item`은 trss-collect `ended_tests.rs`의 `Cause::NoCollectFolder` 줄과 같은 경우예요.
- `video_revisions.rs`에 남긴 동시 실행 테스트의 뒷부분(안정된 뒤 전체 목록을 다시 읽지 않음)은 trss-collect의 규칙이에요. 따로 떼지 않았어요.
- 내린 테스트 중 `row_5_an_old_file_of_no_torrent_that_cannot_be_deleted_keeps_both_files`는 root로 돌리면 아무것도 확인하지 않아요. worker에서도 그랬고, 주석으로 밝혔어요. FIFO로 파일을 바꿔치는 두 테스트는 타이밍에 기대요.
- 웹에는 여러 AniList 항목으로 된 시즌의 분할 방영 제안을 화면 응답으로 확인하는 테스트가 없어요. 판정과 문장은 trss-collect가 확인해요.
- `in_place`의 `Overtaken` 경로를 영상이 없는 채로 라우터로 확인하는 웹 테스트가 없어요. 판정의 웹 응답은 `history_api`의 테스트가 확인해요.
- 웹이 `last_received_at`을 `channel_items`에서 다시 계산하는 것과 `in_place` 규칙의 자리는 [0111](0111-web-and-screen-rules.md)의 후보예요.
