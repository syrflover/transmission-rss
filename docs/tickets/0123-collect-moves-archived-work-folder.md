# 0123 보관 폴더에 있는 작품의 규칙이 수집을 시작하면 작품 폴더를 수집 폴더로 옮겨요

- 상태: 완료 (2026-10-08)
- 출처: [보관과 복원의 폴더 이동](../specs/collection.md#보관과-복원의-폴더-이동), [방영작 구독](../specs/collection.md#방영작-구독), 사용자 요청(2026-10-07)
- 막는 티켓: 없음. 목표 5의 확인을 마친 뒤 0118–0123을 함께 마쳐 0.6.1로 올려요(사용자 결정, 2026-10-07).

## 작업

지금은 보관 폴더(`Shows`)에 있는 작품 `A`의 3기를 구독하면, 새 규칙이 수집 폴더에 `Shows (current)/A/Season 03`을 만들어 받아요. 지난 시즌은 `Shows/A`에 남아요.
라이브러리는 감시 폴더와 폴더 이름의 짝으로 작품을 알아보므로, 방영하는 동안 같은 이름의 작품이 둘이에요. 3기 규칙을 보관할 때 `Shows/A`로 합쳐져야 하나가 돼요.
손으로 먼저 옮기면 라이브러리가 따라가지 않아요(`rule_archive.rs`의 "Nothing follows a folder moved by hand"). 지난 시즌의 작품은 `missing`이 되고 수집 폴더에 새 작품이 생겨, 시즌 연결과 표지 선택, 제작자 지정이 옛 작품에 남아요.

사용자는 속편을 구독하면 작품 폴더를 통째로 수집 폴더로 옮기기를 바라요(사용자 결정, 2026-10-07). trss가 생기기 전부터 그렇게 손으로 옮겨 왔어요.
2026-10-07에 실제 서버의 라이브러리를 읽었어요. 작품 88개 가운데 두 감시 폴더에 나뉜 작품과 `missing`인 작품은 없었어요. `Shows (current)`에서 2기 이상인 7개(Clevatess, Hell Mode, Mushoku Tensei, Nige Jouzu no Wakagimi, Otome Game Sekai wa Mob ni Kibishii Sekai desu, Re Zero kara Hajimeru Isekai Seikatsu, Tensei Shitara Slime Datta Ken)는 지난 시즌까지 손으로 옮겨 둔 작품이에요. 그래서 이 변경으로 따로 정리할 기존 데이터는 없어요.

심링크(`Shows (current)/A → ../Shows/A`)는 쓰지 않아요. 사용자도 좋지 않다고 봤고, 제가 확인한 까닭은 다음과 같아요.

- 라이브러리 검색은 감시 폴더 안을 가리키는 링크만 따라가요(`discovery.rs`). 수집 폴더의 링크는 건너뛰어, 새 시즌이 보관 폴더의 작품으로 들어와요.
- 그 시즌 규칙을 보관하면 보관 이동이 링크를 그 대상 폴더 안으로 합치게 돼요. 이동 코드는 이 경우를 다루지 않아요.
- 컨테이너(`/downloads/…`)와 호스트(`/data/storage1/media/…`)의 경로가 달라, 절대 경로 링크는 한쪽에서 깨져요.

제안한 동작이에요(2026-10-07). 사용자는 이를 듣고 넣을 시점을 정했어요.

- 기준은 구독만이 아니라 규칙이 수집을 시작하는 때예요. 구독이나 규칙을 웹에서 추가할 때, 멈춘 규칙의 `영상 받기`를 다시 켤 때 그 규칙의 작품 폴더가 보관 폴더에 있으면 옮겨요. 앱 YAML 가져오기(목표 6)가 규칙을 만들 때도 옮길지는 구현 때 정해요.
- 복원처럼 폴더를 먼저 옮기고 나서 받아요. 옮기지 못하면(겹치는 파일, 다른 파일시스템) 규칙은 받지 않고 규칙 상세에 까닭을 보여줘요. 한 작품이 두 폴더에 나뉘는 경우를 애초에 만들지 않는다는 결정(사용자 결정)을 따른 거예요.
- 옮기기는 보관과 복원이 쓰는 이동(`move_work_folder`)과 라이브러리의 작품 잇기(`follow_move`)를 그대로 써요. Transmission의 토렌트 위치를 먼저 옮기고, 수집 폴더에 같은 이름의 작품 폴더가 있으면 그 안으로 합치며, 겹치는 파일이 있으면 아무것도 옮기지 않아요.
- 구독과 규칙 추가 화면은 저장 폴더의 작품이 보관 폴더에 있으면 만들기 전에 `Shows/A`를 수집 폴더로 옮긴다고 알려요. 라이브러리의 기록으로 판단하고 디스크를 읽지 않아요.
- 그 시즌 규칙을 보관하면 지금처럼, 수집 중인 규칙이 더 없을 때 `A`를 통째로 보관 폴더로 옮겨요.
- [보관과 복원의 폴더 이동](../specs/collection.md#보관과-복원의-폴더-이동)과 [방영작 구독](../specs/collection.md#방영작-구독)을 같은 변경에서 고쳐요.

## 완료 기준

| 상태 | 기대 결과 |
| --- | --- |
| 보관 폴더에만 `A`가 있고 `A/Season 03`을 구독 | 구독 화면이 만들기 전에 옮긴다고 알려요. 만들면 `A`가 통째로 수집 폴더로 옮겨진 뒤 받아요. 라이브러리의 작품 ID, 시즌 연결, 표지 선택, 제작자 지정이 그대로예요. |
| 두 감시 폴더에 모두 `A`가 있음 | 보관 폴더의 `A`를 수집 폴더의 `A` 안으로 합치고, 라이브러리의 두 작품도 하나가 돼요. 같은 상대 경로의 파일이 있으면 아무것도 옮기지 않고, 규칙은 받지 않으며 규칙 상세에 겹치는 파일이 있어요. |
| 보관 폴더의 `A` 안에서 Transmission이 시딩하는 토렌트 | 토렌트의 위치가 먼저 옮겨져, Transmission이 보는 경로와 파일이 맞아요. |
| 작품 폴더가 보관 폴더에 있는 멈춘 규칙의 `영상 받기`를 다시 켬 | 같은 방식으로 옮긴 뒤 받아요. |
| 작품 폴더가 수집 폴더에만 있거나 어디에도 없음 | 옮기지 않고 지금처럼 받아요. |
| 그 시즌 규칙을 보관 | 지금처럼 수집 중인 규칙이 더 없으면 `A`를 통째로 보관 폴더로 옮겨요. |

- 시험은 규칙과 이동을 맡은 trss-collect에 두고, worker에는 명령의 순서와 중단 뒤 이어 하기만 둬요([테스트 나눔 ADR](../adr/0015-test-a-rule-once-in-its-crate.md)).
- 0.6.1을 올린 뒤 실제 서버에서, 지난 시즌이 `Shows`에 있는 속편을 휴대폰으로 구독해 봐요. [0085](0085-phone-and-tablet-flows.md)에는 휴대폰으로 구독한 관찰이 없어서 이것으로 채워요.

## 결과

2026-10-08에 구현했어요. 커밋은 `feat(collect): move an archived work folder into the collect folder when its rule starts collecting (0123)` 하나예요. 독립 검토가 찾은 빈틈과, 받는 곳에서 막으라는 사용자 결정(2026-10-08)은 같은 날 이 커밋에 `fixup!`으로 이어서 고쳤어요.

### 바뀐 것

- `rule_archive` 명령에 방향 `start`(새 규칙이나 구독, 저장 폴더 변경, 옮기지 못한 `start`의 재시도)와 `resume`(`영상 받기` 켜기)를 더했어요. 복원과 같은 이동(`move_work_folder`, `follow_move`)을 쓰고 새 이동 코드는 없어요. 옮기기를 마친 뒤에야 규칙을 켜요.
- 웹은 구독이나 규칙 추가, 수집 중인 규칙의 저장 폴더 변경, `영상 받기` 켜기에서 작품 폴더가 보관 폴더에 있으면 규칙을 `멈춤`으로 만들거나 저장하거나 둔 채 명령을 접수해요. 멈춘 규칙은 수집 주기가 받지 않으므로(`process_job`이 작업 폴더 잠금 안에서 상태를 다시 확인해요) 옮기기 전에 수집 폴더로 들어오는 것이 없어요.
- worker는 규칙의 저장 폴더에 토렌트를 넣는 모든 길에서 넣기 직전에 `plan_start`로 다시 확인해요(`move_before_receiving`, 한 곳에만 있어요). 수집 주기의 `process_job`은 작업 폴더의 차례를 잡고 규칙이 아직 켜져 있는지 본 뒤에, `receive_once`는 규칙을 읽고 받을 수 있는지 가린 뒤에 불러요. `receive_past`와 재시도는 `receive_once`의 같은 길이에요. 작품 폴더가 보관 폴더에 있으면(수집 폴더에도 있는 이미 나뉜 경우도 같아요) 받지 않고 규칙을 `멈춤`으로 만든 뒤 `start`를 접수해요. 수집 주기는 항목을 기록하지 않고 다음 주기에 남기고(`CycleReport`의 `waiting_for_move`, 접수한 명령 수는 `moves_asked`), 명령 실행기를 깨워요. 같은 규칙에 열린 명령이 있으면 항목만 남겨요. 명령은 두 폴더의 쓰기 차례가 필요하므로 차례를 쥔 채 기다리지 않아요. 명령 길(`다시 받기`, `받기`)은 `작품 폴더가 보관 폴더에 있어서 먼저 수집 폴더로 옮기고 있어요…` 문장으로 `failed`로 끝나요. 항목은 그대로예요. 규칙 상세는 열린 `start`가 있는 `멈춤` 규칙이라 `멈춤` 배너 대신 이동 문장을 보여요(`archive_move`).
- 옮기지 못하면 규칙은 `멈춤`인 채로 있고, 규칙 상세의 폴더 이동 문장이 겹치는 파일 같은 까닭을 보여줘요. `영상 받기`를 다시 켜면 다시 시도해요.
- 접수된 명령이 있는 동안은 같은 규칙에 `rule_archive`를 더 접수하지 않고(`Accepted::Busy`), 그 작품 폴더에 다른 규칙을 새로 만드는 것과 옮기는 규칙의 `영상 받기`를 켜거나 끄는 것을 400으로 거절해요. `start`와 `resume`은 `POST /api/commands`로 보낼 수 없어요(400). 웹이 규칙을 저장할 때만 만들어요.
- 옮기기를 마치고 규칙이 켜지면 worker가 구독 제작자의 자막을 찾아 받아요(`follow_logged`).
- `GET /api/rules/archived-work?directory=[&from=]`가 라이브러리의 기록으로 보관 폴더의 작품을 알려 주고, 구독 화면(저장 폴더 단계와 확인 단계), 규칙 추가 화면, 수집 중인 규칙의 편집 화면이 저장하기 전에 옮긴다고(수집 폴더에도 있으면 합친다고) 알려요. 화면은 적은 폴더를 그대로 보내고 작품 폴더는 서버가 가려내요. 웹의 판단(`plan_start`)은 디스크를 봐요.
- 규칙 상세의 보기: 옮기는 중에는 `멈춤` 배너 대신 이동 문장을 보이고, 폴더 이동 중인 규칙의 `보관`·`삭제`·스위치는 잠겨요. 옮기지 못한 `start`나 `resume`의 문장은 규칙이 아직 `멈춤`인 동안만 보여요. 구독 확인 단계는 만들어진 규칙이 `멈춤`으로 돌아왔을 때 체크한 지난 항목을 받지 않아요.

### 완료 기준과 시험

| 완료 기준 | 시험 |
| --- | --- |
| 보관 폴더에만 `A`, 구독하면 알리고 옮긴 뒤 받음. ID·시즌 연결 그대로 | 알림은 `trss-web`의 `the_form_is_told_which_work_in_the_archive_folder_it_would_bring_over`와 `the_edit_form_asks_whether_the_changed_folder_moves_a_work_over`, `trss-collect`의 `the_form_learns_of_an_archived_work_from_the_library_not_the_disk`. 옮긴 뒤 받기는 `trss-worker`의 `a_new_rule_for_an_archived_work_collects_only_after_its_folder_came_into_the_collect_folder`(옮기기 전과 옮기는 중의 RSS 확인이 수집 폴더에 아무것도 받지 않고, 옮긴 뒤에 받음). 작품 ID·회차 기록·표지 선택·시즌 연결(AniList)은 `starting_a_rule_for_an_archived_work_keeps_its_id_under_the_collect_folder` |
| 두 폴더에 `A`가 있음. 합치고 작품이 하나가 됨 | `starting_a_rule_for_a_work_in_both_folders_merges_the_archives_into_the_collect_folders_id`, `turning_video_on_moves_an_archived_work_first_and_a_work_not_archived_turns_on_at_once` |
| 겹치는 파일이 있음. 옮기지 않고 받지 않으며 까닭을 보여줌 | `trss-collect`의 `a_start_that_cannot_move_the_folder_leaves_the_rule_off_and_says_why`, `trss-worker`의 `a_new_rule_whose_work_folder_cannot_be_moved_stays_off_and_shows_the_reason`(겹침을 치운 뒤 `영상 받기`로 `start`를 다시 접수해 성공) |
| 보관 폴더의 `A` 안에서 시딩하는 토렌트 | `a_new_rule_for_an_archived_work_collects_only_after_...`(가짜 Transmission의 토렌트가 새 위치를 가리킴), `two_starts_for_one_work_folder_run_one_after_the_other_...`(토렌트 위치를 한 번만 옮김). Transmission이 답하지 않으면 규칙이 꺼진 채인 것은 `a_start_that_cannot_reach_transmission_leaves_the_rule_off` |
| 멈춘 규칙의 `영상 받기`를 다시 켬 | `trss-web`의 `turning_video_on_for_a_work_in_the_archive_folder_leaves_the_rule_paused_with_its_resume_open`, `trss-worker`의 `turning_video_on_moves_an_archived_work_first_and_a_work_not_archived_turns_on_at_once` |
| 작품 폴더가 수집 폴더에만 있거나 어디에도 없음 | `trss-collect`의 `a_start_with_no_archive_folder_or_no_work_folder_just_turns_the_rule_on`, `trss-worker`의 `a_rule_for_a_work_that_is_not_archived_is_made_on_and_collects_at_once` |
| 그 시즌 규칙을 보관 | 보관 동작은 바꾸지 않았고, 기존 보관 시험(`archive_move.rs`의 1–8절)이 그대로 통과해요. |
| 옮기는 중 worker가 멈춤 | `a_start_stopped_midway_leaves_the_rule_off_until_the_next_run_finishes_it` |

검토가 찾은 빈틈을 막는 시험이에요.

| 빈틈 | 시험 |
| --- | --- |
| 수집 중인 규칙의 저장 폴더를 보관 폴더에 있는 작품으로 바꿔 저장 | `trss-web`의 `editing_a_collecting_rule_into_an_archived_work_folder_saves_it_paused_with_its_start_open`, `trss-worker`의 `saving_a_collecting_rule_into_an_archived_work_folder_moves_the_work_before_it_collects_there`(옮긴 뒤 켜지고 `resumed_at`이 그대로) |
| 옮긴 뒤 구독 제작자의 자막을 찾지 않음 | `trss-worker`의 `a_start_or_resume_that_turns_a_followed_subscription_on_makes_the_creators_jobs` |
| 옮기지 못한 `start`를 `영상 받기`로 다시 하면 `resume`이 돼 `resumed_at`이 생김 | `trss-web`의 `a_failed_start_is_retried_as_a_start_and_a_move_open_refuses_off_and_two_switches`, `trss-worker`의 `a_new_rule_whose_work_folder_cannot_be_moved_...`(`resumed_at`이 비어 있음) |
| 옮기는 중 `영상 받기`를 끄거나 스위치 둘을 함께 보냄 | 같은 `trss-web` 시험 |
| 브라우저가 `start`·`resume`을 직접 보냄 | `trss-web`의 `a_browser_cannot_post_a_start_or_a_resume` |
| 같은 작품 폴더의 `start` 둘 | `trss-worker`의 `two_starts_for_one_work_folder_run_one_after_the_other_and_the_second_just_turns_its_rule_on` |
| 적은 폴더의 철자(`./A/S3`, 절대 경로)를 화면과 서버가 다르게 읽음, 낡은 안내로 체크한 항목을 버림 | 요청 경로와 `받을 수 있는지`는 `web/src/screens/collect/rules/archivedWork.test.ts`, 서버의 해석은 `trss-web`의 `the_edit_form_asks_whether_the_changed_folder_moves_a_work_over` |
| 해결한 옛 실패 문장이 남음 | `web/src/screens/collect/rules/moveStillTells.test.ts` |

받는 곳의 확인(사용자 결정, 2026-10-08)을 막는 시험이에요. 웹의 확인을 거치지 않도록 규칙을 저장소에 켠 채로 직접 만들고(`name_title`이 남기는 규칙과 같아요) 작품 폴더를 보관 폴더에 둬요. `move_before_receiving`이 바로 `Go`를 돌려주게 바꾸면 이 표의 시험은 `receiving_goes_on_when_the_archive_folder_does_not_hold_the_work_folder`만 빼고 모두 실패해요(2026-10-08 확인).

| 받는 길 | 시험 |
| --- | --- |
| 수집 주기 | `trss-worker`의 `a_cycle_leaves_an_item_of_a_rule_whose_work_folder_is_archived_and_brings_the_folder_in_first`(받지 않고 `<수집>/A`를 만들지 않으며 기록도 없음. 규칙은 `멈춤`이고 `start`가 열려 있고 `resumed_at`이 그대로. 명령이 옮긴 뒤 규칙이 켜지고 다음 주기가 `<수집>/A`에 받음) |
| 수집 주기, 이미 나뉨 | `a_cycle_also_heals_a_work_split_across_both_folders_by_merging_the_archive_into_the_collect_one` |
| 수집 주기, 열린 명령 | `a_cycle_with_a_move_already_open_for_the_rule_leaves_the_item_and_stores_nothing_more` |
| `다시 받기`(`receive_once`) | `a_retry_into_an_archived_work_folder_ends_saying_so_pauses_the_rule_and_the_move_runs`(문장으로 끝나고 토렌트를 넣지 않으며 항목은 그대로. 이동 중 규칙은 `멈춤`. 옮긴 뒤 다시 받으면 `<수집>/A`에 들어감) |
| 지난 회차 검색의 `받기`(`receive_past`) | `trss-worker`의 `past_search::a_result_received_into_an_archived_work_folder_waits_for_its_move_and_goes_in_after_it` |
| 한 곳의 확인 | `trss-collect`의 `receiving_into_an_archived_work_folder_pauses_the_rule_and_stores_a_start_once`, `receiving_goes_on_when_the_archive_folder_does_not_hold_the_work_folder` |

이 밖에 명령 값의 모양(`a_start_is_a_payload_of_its_own`), 새 규칙과 `영상 받기`의 `resumed_at` 처리(`a_new_rule_is_turned_on_as_if_...`), 접수 한 번(`a_start_is_stored_once_per_rule_while_it_is_open`)을 시험해요. 화면 쪽 순수 모듈(`archivedWork.ts`, `moveStillTells.ts`)은 아무것도 import하지 않아요. `@/`를 쓰는 `api.ts`에서 타입을 가져오면 경로 설정이 없는 테스트 project의 typecheck가 실패하기 때문이에요.

### 구현 결정 (구현 결정, 2026-10-08)

- 이전 trss 설정 가져오기(`crates/trss-import`, `import_api`)는 폴더를 옮기지 않아요. 가져오기는 규칙을 만들 뿐이고, 앱 YAML 가져오기(목표 6)가 옮길지는 그때 정해요.
- 웹이 옮길지 정하는 근거는 디스크예요(`plan_start`). 라이브러리의 기록은 감시 주기만큼 늦을 수 있어서, 기록으로 정하면 폴더가 나뉘는 규칙이 만들어질 수 있어요. 화면의 알림만 완료 기준대로 라이브러리의 기록을 써요. 알림이 어긋나면 안내와 실제가 다를 수 있지만 폴더는 나뉘지 않아요.
- `start`는 `resumed_at`을 건드리지 않고, `resume`은 접수한 때를 `resumed_at`으로 써요. 새 규칙이 기다린 동안 기록된 항목을 지난 회차로 만들면 이미 올라온 새 회차를 받지 못해서예요. 수집 중이던 규칙의 저장 폴더 변경도 `start`라서 `resumed_at`이 그대로예요. 실패한 `start`의 재시도도 `start`로 접수해요. `start`는 새 규칙, 수집 중이던 규칙의 저장 폴더 변경, 받는 곳의 확인에서 생겨요. 셋 다 사용자가 규칙을 멈춘 것이 아니라 폴더를 기다리는 것이라, 재시도도 `resumed_at`을 두어 실패한 동안 처음 본 항목을 옮긴 뒤 받아요. 받는 곳의 확인이 멈춘 규칙은 그전까지 받아 왔으므로 "받은 적이 없다"는 까닭은 맞지 않아요(리뷰 뒤 바로잡음, 2026-10-08).
- 수집 중인 규칙의 저장 폴더를 같은 작품 폴더 안에서 바꾸면 옮기지 않고, 다른 작품 폴더로 바꿀 때만 옮겨요. 같은 작품 폴더에 이미 받던 규칙은 보관 폴더에 같은 이름의 작품이 있어도 나뉜 상태가 먼저 있던 것이라 이 변경이 새로 만드는 것이 아니에요.
- 옮기는 동안 규칙이 `멈춤`이라 `receive_once`도 거절해요. 구독 확인 단계에서 체크한 지난 항목은 만들어진 규칙이 `멈춤`으로 돌아왔을 때만 받지 않고, 옮긴 뒤 규칙 화면에서 골라 받아요.
- 옮기기를 마친 규칙에는 worker가 `follow_logged`로 구독 제작자의 자막을 찾아요. 웹의 `switch_rule`이 부르는 `follow_now`는 옮기기가 필요한 규칙에서는 부르지 않으므로(규칙이 아직 `멈춤`이라 하는 일이 없어요) 그 일을 worker 명령 끝으로 옮겼어요. `trss-jobs`가 `trss-collect`에 의존하므로 `trss-collect`가 아니라 `trss-worker`의 명령 실행에 이었어요. 이 점검은 모든 `rule_archive` 명령이 끝날 때 한 번 돌고, 꺼진 규칙은 건너뛰어요.
- 이동 명령의 ID는 방향과 규칙, 접수한 때로 만들어요. 한 규칙에 열린 명령은 하나라서 같은 규칙에 `start`나 `resume`을 두 번 접수하지 못해요.
- 받는 곳의 확인(2026-10-08, 사용자 결정): 웹의 입구를 하나씩 막는 것으로는 한 작품이 두 폴더에 나뉘지 않는다고 보장할 수 없어서, trss가 실제로 규칙의 폴더에 받는 곳에서 막기로 했어요. 웹의 확인은 그대로 둬요. 제목을 기다리던 구독에 제목과 저장 폴더를 정하는 `name_title`(`subscriptions_api.rs`)에는 웹의 확인을 더하지 않아요. 수집 주기의 확인이 막아 주기 때문이에요.

### 알려진 한계

- 규칙이나 구독을 만들 때 규칙(`멈춤`)과 그 `start` 명령을 두 단계로 저장해요. 명령 저장이 실패하면 규칙은 `멈춤`인 채 까닭 없이 남고, `영상 받기`를 켜면 시작해요.
- 자막을 놓는 코드(`trss-jobs`의 `files.rs` `make_dirs`, `runner.rs` `create_dir_all`)는 폴더 잠금을 잡지 않아서, 옮기는 중에 놓은 자막이 옛 쪽 폴더를 다시 만들 수 있어요. 보관과 복원에서도 있던 한계인데, 이제 구독도 이동을 시작해요.
- 옮기는 중에 규칙을 지우면 폴더는 이미 옮겨진 채 명령이 `RULE_GONE`으로 끝나요(화면은 잠가요).
- 받는 곳의 확인은 `start`를 접수한 뒤에 규칙을 멈춰요. 멈추는 데 실패하면 규칙은 켜진 채 남고(항목은 남겨요), 명령이 끝나면 폴더가 옮겨져요. 지난 회차 검색의 `받기`가 옮기는 중이라 끝나면 그 항목은 지난 항목(`no_match`)으로 기록만 남아요(`receive_past`가 받기 전에 기록해요). 옮긴 뒤 같은 결과를 다시 골라야 받아요(시험이 이 순서를 확인해요).
- 저장 폴더에 `..`가 든 예전 규칙은 받는 곳의 확인(`plan_start`)이 작품 폴더를 수집 폴더 밖으로 읽어 지나가요. 수집 주기의 잠금 경로는 `..`를 글자로 풀어서, 그런 규칙은 보관 폴더에 있는 작품을 수집 폴더에 다시 만들 수 있어요. 새 규칙은 `..`를 가질 수 없고, 2026-10-08 실제 서버의 규칙 27개에도 `..`, 절대 경로, `./`로 시작하는 저장 폴더가 없었어요.
- 받는 곳의 확인이 `start`를 접수하는 바로 그 순간(몇 ms)에 `영상 받기`를 끄면, 끄기는 받아들여지지만 `start`가 옮긴 뒤 규칙을 다시 켜요.
- 첫 완료 기준의 `제작자 지정`(파일 제작자)이 이동 뒤에도 그대로인지는 시험으로 확인하지 않았어요. 시험은 작품 ID, 회차 기록, 표지 선택, AniList 시즌 연결까지 봐요. Anissia 시즌 연결도 보지 않았어요.

### 실제 서버 (2026-10-08, 0.6.1)

- 사용자가 `정반대의 너와 나` 2기를 Anissia 편성표로 구독했어요(03:23:55 KST). 저장 폴더는 `Seihantai na Kimi to Boku/Season 02`이고, 1기 작품 폴더는 `Shows`에 있었어요. 어느 기기로 구독했는지는 기록하지 않았어요.
- 규칙은 `멈춤`으로 만들어졌고, `start` 명령이 9 ms 만에 `moved`로 끝났어요. 규칙은 `활성`이 됐고, 라이브러리의 작품(1기 01–12화, AniList 184951 연결)은 `Shows (current)`에 있어요.
- 구독 확인 단계에서 체크한 지난 항목 두 개(SubsPlease `- 24`, `- 25`)는 받지 않았고, 규칙 화면의 지난 회차로 남았어요. 위 구현 결정대로예요. 사용자는 체크한 항목을 옮긴 뒤 받기를 바라서 [0125](0125-receive-ticked-items-after-move.md)로 바꿔요(사용자 결정, 2026-10-08).
- 규칙이 아직 아무것도 받지 않아 회차 변환이 정해지지 않았어요. 방영이 2026-10-04에 끝나 새 회차가 오지 않고, `- 24`를 처음 받으면 −12를 제안만 하고 `S02E24`로 받아요. 받기 전에 제안을 보여주기로 해서 [0126](0126-offset-suggestion-before-first-receive.md)으로 바꿔요(사용자 결정, 2026-10-08). 사용자는 받기 전에 회차 변환을 −12로 직접 저장했어요.

### 검증하지 못한 것

- 속편 구독을 휴대폰으로 했는지는 기록하지 않아서, [0085](0085-phone-and-tablet-flows.md)에 채울 휴대폰 관찰은 없어요.
- 실제 Transmission과 실제 디스크에서의 이동은 가짜 Transmission과 임시 폴더로만 시험했어요.
- 화면(`RuleDetail`, `Confirm`, `PickFolder`)은 타입 검사와 순수 모듈 시험만 했고 브라우저에서 눌러 보지 않았어요.
