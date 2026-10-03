# 0048 제작자 알 수 없는 자막에 Anissia 제작자를 붙여요

- 상태: 완료
- 출처: [머리와 시즌](../specs/library.md#머리와-시즌)의 `제작자 지정`, [자막 후보 조회](../specs/subtitles.md#자막-후보-조회)의 수정 후보
- 막는 티켓: [0034](0034-season-anissia-link.md), [0037](0037-candidate-section.md)(수정 후보 분류)

## 작업

감시 폴더에서 발견했거나 올린 자막은 `제작자 알 수 없음`이에요. 사용자가 그 시즌에 연결한 Anissia 작품의 제작자를 붙일 수 있게 해요(사용자 결정, 2026-10-02).

- 작품 상세 머리의 `제작자 지정`은 선택한 시즌의 제작자 모르는 자막 전체에 한 번에 붙이고, 회차 줄을 펼친 자리의 자막 파일에서는 하나씩 바꿔요.
- 붙인 제작자는 머리와 `자막` 카드의 표시, 수정 후보 분류(같은 회차·같은 제작자), 교체 비교의 출처에 쓰여요.
- 파일을 옮기거나 다시 받지 않고, 구독이 없는 작품에 자동 수신을 만들지 않아요.
- 구독 제작자와 같은 제작자를 붙인 회차는, 붙인 뒤에 새로 나온 그 제작자의 줄을 자동으로 수정본으로 받아요(사용자 결정, 2026-10-03, 아래 결과).
- Anissia 연결이 없는 시즌에서는 먼저 연결하라고 알려요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 구독 없는 작품, 시즌 1에 제작자 모르는 자막 12개, Anissia 연결됨 | `제작자 지정`으로 하느를 고르면 머리에 `하느`가 보이고 12개 파일이 하느의 것으로 기록돼요. 파일은 그대로예요. |
| 그 뒤 하느의 5화 후보가 관찰됨 | 5화 자막이 이미 있으므로 수정 후보로 보여요. |
| 한 파일만 다른 제작자로 바꿈 | 그 파일만 바뀌고, 머리에 두 제작자가 모두 보여요. |
| Anissia 연결 없는 시즌 | `제작자 지정` 대신 먼저 연결하라는 안내가 있어요. |
| 제작자를 붙인 작품에 구독이 없음 | 자동 수신 작업이 생기지 않아요. |
| 두 화면에서 같은 파일의 제작자를 바꿈 | 늦은 저장은 버전 충돌로 거부돼요. |

## 결과

### 만든 것 (2026-10-03)

- **시즌 한 번에**: 작품 상세 머리의 `제작자 지정`이 선택한 시즌의 Anissia 작품 제작자 목록(아직 줄을 관찰하지 못한 제작자 포함)을 보여주고, 고른 제작자를 그 시즌의 제작자 모르는 자막 전체에 붙여요. 붙인 개수를 알려 주고(`자막 6개에 제작자를 붙였어요`), 이미 제작자가 있는 자막과 파일 자체는 그대로 둬요. `POST /api/library/works/{id}/seasons/{n}/subtitle-creators`.
- **파일 하나씩**: 회차 줄을 펼친 자리의 자막 파일에서 바꾸거나 지워요. `PUT …/subtitle-creators/file`이 `{path, version, creator|null}`을 받고, 낡은 버전이면 409와 지금 상태(`current`)를 돌려줘요.
- **표시**: 머리는 구독 제작자, 붙인 제작자, 남아 있으면 `제작자 알 수 없음`을 함께 보여요. `파일` 카드의 회차별 파일 대응은 자막마다 제작자를 적어요.
- **수정 후보와 자동 수신**: 붙인 제작자의 같은 회차 후보는 수정 후보로 보여요(회차 대응이 정해졌을 때만, 그 오프셋으로 맞춰서). 구독 제작자와 같은 제작자를 붙인 회차에서는, 붙인 뒤에 처음 관찰했고 Anissia 수정 시각(`updDt`)도 그 뒤인 줄을 자동 작업으로 받고 `제작자를 붙인 자막의 수정본`으로 기록해요(사용자 결정, 2026-10-03). 받기만 하고 바이트 비교는 하지 않아요.
- **기록(마이그레이션 40)**: `media_files.creator_source_id`·`creator_version`·`creator_set_at`, `subtitle_jobs.revises_attributed`예요. 새로 발견한 파일의 버전은 발견 시각(ms)에서 시작해서, 지웠다 다시 생긴 파일을 낡은 화면이 바꾸지 못해요. 다시 읽기가 제작자를 지우면 버전이 올라가요. 작품 폴더를 옮기거나 합쳐도 제작자와 붙인 시각이 따라가고, 같은 경로가 겹치면 제작자가 있는 쪽을 남겨요.
- 규칙 전체는 [머리와 시즌](../specs/library.md#머리와-시즌)과 [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 제작자 모르는 자막 12개에 하느 | `naming_a_creator_marks_all_twelve_unknown_files_and_moves_nothing`, `naming_a_creator_covers_the_seasons_unknown_subtitles_only_and_leaves_the_rest`. 개발 환경(서버 데이터 복사본)의 `Kimi ga Shinu made Koi wo Shitai` 시즌 1에서 카이란을 고르면 `자막 6개에 제작자를 붙였어요`가 보이고, 머리가 `카이란`으로, 회차별 파일 대응의 자막 6개가 `· 카이란`으로 바뀌었어요. DB에는 6개 모두 같은 출처와 붙인 시각이 기록됐어요. |
| 그 뒤 하느의 5화 후보 | `the_creators_later_candidate_of_an_episode_with_such_a_file_is_a_revision_candidate`, `the_sources_mapping_decides_which_episode_of_the_season_a_candidate_is`, `a_season_without_a_creator_named_marks_no_candidate_a_revision` |
| 한 파일만 다른 제작자 | `one_file_changes_alone_and_the_season_then_has_both_creators`, `one_files_creator_changes_by_its_version_and_a_stale_version_changes_nothing` |
| Anissia 연결 없는 시즌 | `a_season_without_an_anissia_link_cannot_name_a_creator`, `a_creator_the_anime_does_not_have_is_refused` |
| 구독 없는 작품 | `naming_a_creator_makes_no_automatic_receipt_for_a_work_with_no_subscription`, `a_named_file_makes_no_receipt_without_a_subscribed_creator_or_with_subtitles_off` |
| 두 화면에서 같은 파일 | `the_later_of_two_screens_changing_the_same_file_is_refused_with_the_file_as_it_is`. 개발 환경에서 버전 0으로 보낸 PUT이 409와 지금 상태(카이란, 버전 1)를 돌려줬고, 버전 1로 지우면 200이었어요(확인 뒤 6개 모두 지워서 되돌렸어요). |
| 붙인 뒤의 수정본 자동 수신(추가 결정) | `a_line_first_seen_after_the_creator_was_named_for_the_file_is_received_as_its_revision`, `a_line_anissia_shows_was_there_before_the_creator_was_named_is_no_such_revision`, `a_named_file_of_another_creator_or_a_line_of_another_creator_is_no_such_revision`, `the_attributed_revision_follows_the_sources_mapping_and_waits_for_a_decision`, `another_creators_job_that_has_not_failed_still_holds_an_attributed_episode` |
| 다시 읽기·옮기기·합치기 | `a_rescan_keeps_the_creator_…`, `a_renamed_file_or_one_that_left_and_came_back_is_by_an_unknown_creator_again`, `a_moved_work_folder_keeps_its_files_creators_and_a_merge_carries_them_over`, `a_rescan_that_clears_the_creator_raises_the_version`, `a_file_removed_and_added_again_is_a_new_file_no_older_version_can_change`, `a_merge_keeps_a_named_creator_over_an_unknown_one_for_the_same_path`, `a_creator_named_for_the_files_survives_a_rescan_of_the_watch_folder` |
| 마이그레이션 39 → 40 | `a_library_from_before_subtitle_creators_keeps_its_files_with_no_creator` |

작업 공간 시험 1,836개가 통과하고(실제 네트워크 시험 3개는 무시), web node 시험이 통과하고, clippy 경고가 없고, 웹 빌드가 돼요. 전체 시험을 네 번 돌린 중 한 번은 시험 하나가 실패했어요. 그 실행이 첫 실패에서 멈춰 어느 시험인지 남지 않았고, 이어 세 번은 모두 통과했어요(이 세션에서 보던 잠금·worker 시험의 간헐 실패와 같은 것인지는 확인하지 못했어요). 개발 환경에서 수정본 자동 수신은 돌려 보지 않았어요(구독 제작자에 붙일 자막 파일이 있는 작품이 복사본에 없어요).

### 독립 리뷰

마이그레이션, 버전 계약, 다시 읽기·합치기, 후보 분류, 화면을 두고 리뷰를 받았어요. 막는 결함은 없었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| 구독이 있으면 머리에서 `제작자 알 수 없음`이 사라짐 | 모르는 자막이 남아 있으면 늘 보여요 |
| 한 번에 붙인 개수를 알리지 않음 | 개수를 알리고, 0개면 이미 붙어 있다고 알려요 |
| 다시 읽기가 제작자를 지워도 버전이 그대로이고, 지웠다 다시 생긴 파일이 낡은 버전에서 다시 시작함 | 지울 때 버전을 올리고, 새 파일은 발견 시각에서 시작해요 |
| 회차 `0`과 대응 뒤 0 이하인 회차를 수정 후보로 셈 | 1 이상만 세요 |
| 후보 API가 미정 회차 대응의 오프셋을 그대로 씀 | 자동 수신과 같은 판단(`decided_offset`)을 써요 |
| 합치기에서 같은 경로가 겹치면 제작자를 잃을 수 있음 | 제작자가 있는 쪽을 남겨요 |
| 시즌의 Anissia 연결을 바꾼 뒤의 붙인 제작자 설명이 없음 | 명세에 적었어요 |

그 뒤 사용자 결정(2026-10-03)으로 붙인 자막의 수정본 자동 수신을 더했고, 이것을 따로 리뷰받았어요.

| 지적 | 처리 |
| --- | --- |
| 아직 관찰하지 못한 제작자를 붙이면, 붙이기 전부터 있던 그 제작자의 줄(대개 그 파일의 게시물)을 앱이 붙인 뒤에 처음 보고 수정본으로 받음 | Anissia 수정 시각도 붙인 시각 뒤여야 해요. 시험이 고치기 전 코드에서 실패하는 것을 확인했어요 |
| 시험 파일 하나의 수정이 커밋에 빠질 뻔함 | 같이 커밋했어요 |
| 티켓이 추가 결정을 적지 않음 | 작업과 결과에 적었어요 |
| 지웠다 다시 붙이기, 붙인 시각과 같은 시각의 관찰 | 위 시험에 넣었어요 |

### 남은 한계

- 다시 읽기가 파일의 회차를 새로 읽으면 제작자와 붙인 시각이 새 회차로 따라가요. 사용자가 그 회차에 붙인 적은 없지만, 붙인 시각 뒤의 그 제작자 줄은 자동으로 받아요(드묾).
- Anissia 수정 시각을 읽을 수 없는 줄은 처음 본 시각만 봐요.
- 시즌의 Anissia 연결을 바꾸면 붙인 제작자는 예전 작품의 출처로 남아서 새 작품의 후보와 맞지 않아요. 다시 붙여야 해요.
- 붙인 자막의 수정본을 받아도 지금 자막과 같은지는 비교하지 않아요(교체 비교는 결과 목표 4예요).

### 커밋

- `31d7b6f` feat(library): name the Anissia creator of a subtitle file the library found

