# 0051 구독 제작자 출처의 회차 대응을 방영 시각으로 정해요

- 상태: 완료
- 출처: [자막의 회차 대응](../../../specs/library.md#자막의-회차-대응)의 방영 시각 근거(사용자 결정, 2026-10-03)
- 막는 티켓: [0045](0045-follow-creator-auto-receive.md)(회차 대응 저장과 자동 수신)

## 작업

구독 제작자 출처의 회차 대응을 규칙의 회차 변환 대신 방영 시각으로 정해요.
같은 작품이라도 제작자마다 번호를 매기는 방식이 달라서(2026-10-03 표본) 작품 규칙 하나로는 출처의 대응을 정할 수 없어요.

- AniList 방영 일정을 25개씩 끝까지 나눠 읽어 저장해요. 지금은 한 번에 받는 25개에서 잘려요(2026-10-03 복사본에서 두 항목).
- 출처의 자연수 회차마다, 그 회차 표시를 처음 관찰한 줄의 Anissia 수정 시각이 시즌의 어느 회차 방영 창(k 방영부터 k+1 방영 직전까지, 마지막 회차는 7일)에 드는지로 차이를 구해요.
- 두 회차 이상이 같은 차이를 가리키거나, 한 회차의 차이가 구조(N 안의 차이 0, 앞 시즌 합만큼의 누적 번호)와 맞을 때만 `auto`로 정해요. 그 밖은 까닭과 함께 `undecided`예요.
- 다시 한 결정이 저장된 `auto`와 다른 차이면 `undecided`로 돌려요. `user`는 건드리지 않아요.
- 정한 대응과 어긋나는 회차(소수·문자, 1–N 밖, 방영 시각이 다른 회차)는 그 회차만 받지 않고 어긋났다고 기록해요. 할 일과 화면은 [0052](0052-user-episode-mapping.md)예요.
- 묶음의 근거 문장(`자동 · 3화·4화가 방영 뒤에 올라왔어요` 같은)을 새 근거로 바꿔요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 12화 작품, 제작자가 1화를 1화 방영 1시간 반 뒤, 2화를 2화 방영 이틀 뒤에 올림 | `auto` 차이 0, 근거에 두 회차가 적혀요. |
| 2기(앞 시즌 12화), 제작자가 2기 1화 방영 뒤에 `13`을 올림 | 한 회차지만 누적 번호와 맞아 `auto` 차이 −12예요. |
| 같은 2기, 다른 제작자가 `1`을 올림 | 그 출처는 차이 0이에요. |
| 1화를 2화 방영 뒤에 처음 올린 제작자(한 회차뿐) | 차이 +1은 구조와 맞지 않아 `undecided`예요. |
| 일괄 공개 ONA, 극장판 | 근거가 없어 `undecided`예요. |
| 방영 일정이 25개를 넘는 항목 | 모든 회차의 방영 시각이 저장돼요. |
| `auto` 차이 0이던 출처에 방영 시각으로 다른 차이를 가리키는 회차가 둘 생김 | `undecided`로 바뀌고 자동 수신이 멈춰요. |
| `auto` 차이 0인 출처의 `13.5`화나 N 밖의 회차 | 그 회차만 받지 않고 어긋남으로 기록돼요. 다른 회차는 계속 받아요. |
| `user` 대응 | 근거가 무엇이든 그대로예요. |
| 실제 표본 | 2026-10-03 표본(Anissia 최근 70줄과 AniList 방영 일정)에서 이 규칙이 내는 결과를 표로 남겨요. |

## 결과

### 만든 것 (2026-10-03)

- **방영 일정 끝까지 읽기**: AniList 항목의 방영 일정을 25개씩 `hasNextPage`가 끝날 때까지(최대 20쪽) 읽어 저장해요. `pageInfo.total`은 실제 응답에서 믿을 수 없어서 쓰지 않아요. 25개에서 잘린 채 저장된 완결 항목(`airing` 25개, `episodes` 25 초과)은 다음 갱신에서 한 번 다시 읽어요.
- **방영 시각 근거**(`trss_jobs::mapping`): 출처의 자연수 회차마다 그 회차 표시를 처음 관찰한 줄의 `updDt`가 `air(k) ≤ t < air(k+1)`에 들면 차이 k − n의 근거예요. 7일 창은 k가 시즌의 마지막 회차(N)일 때만이고, 일정이 N 전에 끊기면 근거가 아니에요. 여러 회차가 같은 시각인 일괄 공개, 일정이 없는 시즌, 읽을 수 없는 시각은 근거가 아니에요. 시즌 일정은 시즌에 연결한 항목을 이어 붙여요(`schedule_times`).
- **결정**: 두 회차 이상이 같은 차이를 가리키거나, 한 회차의 차이가 구조(N 안의 0, 앞 시즌 합의 음수)와 맞으면 `auto`예요. 묶음의 근거 문장은 `1화·2화가 방영 뒤에 올라왔어요`처럼 새 근거를 적어요.
- **다시 정하기(마이그레이션 42)**: 저장된 `auto`와 다른 차이가 나오면 `undecided`로 돌리고 그 전 차이를 `retired_offset`에 남겨요. 그 뒤로는 같은 차이로만 다시 `auto`가 돼요. `user`는 건드리지 않아요.
- **어긋나는 회차(마이그레이션 42)**: 소수·문자 회차, 대응을 적용하면 1–N 밖인 회차, `auto`에서 제 방영 시각이 다른 차이를 가리키는 회차는 그 회차만 받지 않고 `subtitle_mapping_conflicts`에 까닭과 함께 남겨요. 출처를 살필 때마다 대응과 같은 트랜잭션에서 지금 집합으로 다시 써요. 이미 받은 회차의 수정본은 대응과 상관없이 받아요.
- 보관 폴더 병합은 `retired_offset`과 어긋난 회차 기록을 대응과 함께 옮겨요.
- 규칙 전체는 [자막의 회차 대응](../../../specs/library.md#자막의-회차-대응)과 [구독 제작자 자동 수신](../../../specs/subtitles.md#구독-제작자-자동-수신)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 두 회차가 방영 뒤에 올라옴 | `two_episodes_posted_after_they_aired_decide_the_mapping_and_the_episodes_are_received`, `two_episodes_posted_after_they_aired_map_as_they_are_and_both_are_the_evidence` |
| 2기의 누적 번호 `13` | `a_cumulative_number_after_the_seasons_first_episode_maps_back_by_the_earlier_season`, `a_cumulative_number_posted_after_the_seasons_first_episode_is_the_previous_seasons_sum_back` |
| 같은 2기의 다른 제작자 `1` | `another_creator_who_counts_from_one_in_the_same_season_maps_as_it_is` |
| 한 회차의 차이 +1 | `a_single_episode_that_does_not_fit_the_structure_is_undecided` |
| 일괄 공개, 일정 없음 | `an_all_at_once_release_and_a_missing_schedule_leave_the_mapping_undecided` |
| 25개를 넘는 일정 | `a_schedule_longer_than_a_page_is_read_to_its_end`, `the_pages_of_a_schedule_are_asked_by_number_and_stop_at_the_cap`, `a_schedule_of_more_than_25_episodes_decides_by_the_episodes_past_the_first_page`, `a_finished_entry_whose_schedule_was_cut_at_25_is_read_again_once` |
| `auto`에 다른 차이가 둘 | `an_auto_mapping_is_taken_back_when_two_episodes_point_at_another_offset`, `an_auto_mapping_that_the_grounds_would_move_to_another_offset_goes_undecided_and_stays`, `an_auto_mapping_is_never_changed_to_another_offset_by_itself` |
| `13.5`·N 밖의 회차 | `an_episode_that_does_not_fit_is_not_received_and_is_recorded_while_the_others_are`, `a_conflict_that_is_gone_is_no_longer_recorded`, `a_revision_of_a_received_episode_that_conflicts_is_received` |
| `user` 대응 | `a_mapping_the_user_set_is_not_changed_by_the_air_time_and_only_its_structure_conflicts`, `the_users_mapping_is_not_touched`, `a_mapping_the_user_set_is_kept` |
| 방영 창 | `a_post_belongs_to_the_episode_whose_window_it_is_in`(일정이 N 전에 끊긴 경우와 빠진 회차 포함), `a_time_that_did_not_read_is_no_evidence` |
| 병합 | `an_archive_move_that_merges_two_works_keeps_the_subtitle_sources_mappings`(`retired_offset`과 어긋난 회차 포함) |
| 실제 표본 | 아래 표본 절 |

작업 공간 시험 1,898개가 통과하고(실제 네트워크 시험은 무시), clippy 경고가 없어요. `a_revision_of_a_received_episode_that_conflicts_is_received`는 어긋남 검사를 수정본 분기 앞으로 되돌리면 실패하는 것을 확인했어요. 개발 환경(서버 데이터 복사본)에서 마이그레이션 42가 적용됐고, 예전 방식으로 정했던 `auto` 대응 하나(FX 전사 쿠루미 · 코코렛, 차이 0)가 새 근거 `1화가 방영 뒤에 올라왔고 시즌 회차 수(12) 안이에요`로 다시 정해져 자막 후보 구역에 `자동 · …`로 보이는 것을 375px 폭에서도 확인했어요.

### 실제 표본 (2026-10-03)

무시된 시험 `airtime_sample`이 Anissia 최근 목록 70줄과 각 작품의 AniList 방영 일정을 읽어, 줄마다 한 회차의 근거로 규칙을 돌렸어요. AniList 항목은 그 작품의 시즌 1 전체로 봤어요(앞 시즌 없음).

| 결과 | 줄 수 | 내용 |
| --- | --- | --- |
| `auto` 차이 0 | 29 | 바로 앞에 방영된 회차를 가리켰어요. |
| 구조와 맞지 않아 미정 | 3 | 블랙 토치 +1, 검은 고양이와 마녀의 교실 +11(늦게 올림), 정반대의 너와 나 2기의 C소라 −12(누적 번호 24화, 표본이 앞 시즌을 모름) |
| 근거 없음 | 10 | 첫 회차 방영 전에 올린 줄(선공개 스트리밍), 다음 회차 방영 뒤에 올린 줄, 극장판 |
| 일정 없음 | 1 | AniList 항목에 방영 일정이 없어요. |
| AniList 항목 없음 | 27 | 원제를 모르거나 같은 제목의 항목이 없거나 여럿이에요. |

같은 날 앞서 돌린 표본과 줄 하나만 달랐어요. 하늘은 붉은 강가의 키리유가 12화(13화 방영 뒤, +1로 미정) 대신 13화를 올려 `auto` 차이 0이 됐어요. 실제 AniList는 한 쪽에 방영 일정을 25개까지 주고, `pageInfo.total`은 믿을 수 없었어요.

### 독립 리뷰

결정 규칙, `store_in`, 마이그레이션, 병합, 나눠 읽기를 두고 리뷰를 받았어요. 판정은 결함 없음이었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| `retired_offset`이 명세에 없고 지우는 길이 없음 | [다시 정하기](../../../specs/library.md#자막의-회차-대응)에 적고, 지우는 일을 [0052](0052-user-episode-mapping.md)의 작업과 완료 기준에 넣었어요 |
| 25개에서 잘린 채 저장된 완결 항목을 다시 읽지 않음 | `airing`이 25개이고 `episodes`가 25를 넘는 항목을 갱신 대상에 넣었어요 |
| 쪽 상한에서 잘린 일정이 끝까지 읽은 것으로 저장됨, 빠진 회차를 마지막 회차로 읽음 | 7일 창을 시즌의 마지막 회차(N)에만 주고, 일정이 N 전에 끊기면 근거로 쓰지 않아요 |
| 어긋남 검사가 받은 회차의 수정본보다 앞에 있어, 사용자가 받은 `13.5`의 수정본을 받지 않음 | 수정본 분기 뒤로 옮기고 시험을 더했어요 |
| 병합이 `retired_offset`을 옮기는지 시험이 없음 | 병합 시험에 넣었어요 |

### 한계

- AniList 회차 수가 없으면 N은 일정의 가장 큰 회차이고, 회차 수를 모르는 앞 항목 뒤의 일정은 이어 붙이지 않아요. 그 밖의 진짜 회차는 `시즌 회차 수 밖`으로 어긋나 받지 않아요(리뷰 지적, 규칙대로 둠).
- 매주 한 주 늦게 올리는 제작자는 두 회차가 같은 +1을 가리켜 `auto` +1이 돼요. 두 회차의 일치는 구조 검사를 건너뛰기로 정했기 때문이에요(리뷰 지적, 명세대로 둠).
- 선공개 스트리밍처럼 TV 방영 전에 올린 줄은 근거가 되지 않아요. 표본의 근거 없음 10줄 가운데 몇 줄이 이 경우예요.
- `retired_offset`에는 `undecided` 줄에만 둔다는 CHECK가 없어요. 병합은 옮긴 대응을 값이 모두 같은 줄로 알아보므로, 우연히 같은 줄이면 어긋남 기록이 바뀌어도 다음 살핌에서 다시 써요.
- `회차 확인 필요` 할 일, 사용자 설정 화면, `retired_offset`을 지우는 길은 [0052](0052-user-episode-mapping.md)에서 만들어요. 그 전까지 미정으로 돌아간 출처는 그 전 차이로만 다시 정해져요.
