# 0023 방영이 끝났거나 새 항목이 없는 규칙에 보관을 제안해요

- 상태: 완료 (실제 Transmission·실제 Anissia와 429·네트워크 단절은 못 봤어요. 아래 "검증하지 못한 것")
- 출처: [규칙 보관](../../../specs/collection.md#규칙-보관)(보관 제안), [수집 화면](../../../specs/collection.md#수집-화면)(구독 탭, 상태 배너), [할 일](../../../specs/jobs.md#할-일)(`보관 제안`)
- 막는 티켓: 없음(보관·복원의 폴더 이동은 [0011](0011-archive-folder-move.md), Anissia 종영 정보는 [0021](0021-weekly-schedule.md)에 있어요)

## 작업

방영이 끝났거나 오래 새 항목이 없는 규칙을 자동으로 끄지 않고 `보관 제안`으로 알려, 사용자가 한 번에 보관하게 해요. 보관 자체와 폴더 이동은 0011의 명령을 그대로 써요.

- 근거는 둘이에요. 구독 규칙은 Anissia의 방영 종료(0021의 종영일이 지남, 또는 종영일 없이 편성표에서 빠짐 `unlisted_at`)이고, 모든 규칙은 마지막 수신 뒤 4주 동안 규칙에 맞는 새 항목이 없음(사용자 결정, 2026-10-01)이에요. `멈춤` 규칙도 같은 근거로 제안을 받고, 보관된 규칙과 제목 대기 구독은 받지 않아요.
- 제안은 저장하지 않고 읽을 때 계산하되, 사용자가 `수집 유지`를 고른 규칙은 같은 근거로 다시 제안하지 않도록 그 선택을 남겨요(새 근거가 생기면 다시 제안). 이 저장이 마이그레이션을 부르면 번호는 그때 master의 다음 번호예요.
- 화면: 구독 탭의 보라 강조 `보관 제안` 묶음(하나면 그 작품 한 줄과 `보관`, 여럿이면 개수와 `모두 보관`·`선택 보관`의 접힌 한 줄, 펼치면 모두 미리 선택된 체크 목록과 `N개 보관`), PC에서 제목 후보 묶음 옆에 나란히, 규칙 상세의 보관 제안 배너(`이 규칙을 보관할까요?`, 까닭, 보관한 뒤의 결과, `보관`·`수집 유지`)와 이동 상태 문장. 메뉴 배지에는 세지 않아요.
- 할 일 화면은 아직 없으므로, 할 일의 `제안` 원천이 읽을 함수와 API만 둬요(0020의 제목 후보와 같은 방식).

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 구독 규칙의 Anissia 종영일이 지남 | 구독 탭과 규칙 상세에 보관 제안이 생기고, 까닭이 방영 종료예요. |
| 종영일 없는 구독이 Anissia 편성표에서 빠짐(`unlisted_at`) | 보관 제안이 생겨요. Anissia에 닿지 못하는 동안에는 생기지 않아요. |
| 마지막 수신 뒤 3주 동안 새 항목 없음 / 4주 넘게 없음 | 3주에는 제안이 없고, 4주가 지나면 제안이 생겨요. |
| 마지막 항목이 5주 전인데 채널을 최근 2주 읽지 못함 / worker가 4주 꺼졌다 다시 켜짐 | 읽은 기간이 4주가 안 되어서 제안이 없고, 읽기가 성공한 날이 28일이 되면 나와요. |
| `멈춤` 규칙이 위 근거를 만족 | 제안을 받아요. 보관된 규칙과 제목 대기 구독은 받지 않아요. |
| 제안이 셋이고 `선택 보관`에서 하나를 풀고 `2개 보관` | 두 규칙이 0011의 순서대로 보관되고 같은 작품 폴더의 규칙들은 마지막 규칙에서 한 번 옮겨요. 푼 규칙은 제안에 남아요. |
| 제안이 하나 | 접지 않고 그 작품 한 줄과 `보관` 버튼만 보여요. |
| 규칙 상세에서 `수집 유지` | 제안이 사라지고 같은 근거로 다시 나오지 않아요. 새 근거(예: 종영)가 생기면 다시 나와요. |
| 메뉴 배지 | 보관 제안을 세지 않아요. |
| PC와 휴대폰 너비 | PC는 제목 후보 옆에 나란히, 휴대폰은 위아래로 쌓이고 가로 스크롤이 없어요. |

## 결과

### 구현한 것

- 마이그레이션 24(`crates/trss-core/migrations/channels/archive_suggestion.sql`, 0024의 `episode_basis`가 23): `channel_read_days`(아래 결정), `rule_started`(규칙이 처음 생긴 때, `rules`의 삽입·삭제 트리거가 앱 쪽 코드를 건드리지 않고 찍고 지워요)와 `archive_suggestion_kept`(`수집 유지`를 고른 규칙과 근거 키, 규칙이 지워지면 같이 지워져요). 옛 DB가 규칙과 이력을 그대로 두고 올라오는 것은 `crates/trss-core/src/db.rs`의 `a_database_from_before_archive_suggestions_keeps_its_rows_and_stamps_new_rules`가 봐요.
- 판단(`crates/trss-collect/src/archive_suggestions/mod.rs`): 근거 둘을 읽을 때마다 계산해요. 방영 종료는 `schedule::slot::run_over`로 주간 편성표가 카드를 내리는 규칙과 같게 하고(종영일이 있으면 그것이 정하고, 없을 때만 `unlisted_at`), `새 항목 없음`은 아래 결정을 따라요. 근거마다 `Ground::key`(`ended:<번호>:<종영일>`, `unlisted:<번호>`, `quiet:<기준 시각>`)가 있고 `수집 유지`는 그 키를 남겨요. 근거가 다른 키로 바뀌면 다시 제안해요.
- 저장소: `HistoryStore::last_received_of_rules`(규칙마다 색인 한 번), `titles_since`(채널 색인으로 4주 안의 제목, 최대 20,000개), `ChannelStore::rule_starts`·`keep_archive_grounds`·`kept_archive_grounds`.
- 웹 API(`crates/trss-web/src/archive_api.rs`): `GET /api/archive-suggestions`(규칙, 작품, 채널, 근거 목록, 마지막 수신, `after` = 보관하면 작품 폴더가 어떻게 되는지 한 문장)와 `POST /api/archive-suggestions/keep`(`{ rule_id, grounds }`). 할 일의 `제안` 원천이 읽을 함수는 `archive_api::archive_suggestions`예요(0020의 제목 후보와 같은 방식, 할 일 화면은 아직 없어요). `after`는 `rule_archive` 명령과 같은 판단(`worker/commands/rule_archive.rs`의 `forecast_archive`)에서 나와서 명령이 실제로 하는 일과 어긋나지 않아요.
- 웹 화면: 구독 탭의 보라 `보관 제안`(`web/src/screens/collect/archive/ArchiveSuggestions.tsx`)은 하나면 그 작품 한 줄과 `보관`, 여럿이면 `보관 제안 N개`와 `모두 보관`·`선택 보관`의 한 줄이고 펼치면 모두 선택된 체크 목록과 `N개 보관`이에요. 여러 규칙은 목록 순서대로 규칙마다 기존 `rule_archive` 명령을 하나 보내고 그 명령이 끝나야 다음을 보내서(`archive/api.ts`의 `archiveRule`: 명령 ID 하나, 답이 없으면 같은 ID로 다시, 끝날 때까지 조회), 같은 작품 폴더를 쓰는 규칙은 명령 쪽 규칙대로 마지막 규칙에서 한 번 옮겨요. 도중에 실패하면 거기서 멈추고 그 까닭을 보여 줘요. 규칙 상세의 `ArchiveBanner`는 `이 규칙을 보관할까요?`와 까닭, `after`, `보관`(규칙 상세의 보관 버튼과 같은 흐름이라 이동 상태 문장은 기존 `ArchiveMoveNotice`가 보여요)·`수집 유지`예요. PC에서는 제목 후보 옆에 나란히 놓이고 휴대폰 너비에서는 위아래로 쌓여요.

### 결정

- "새 항목 없음"의 *새 항목*: 규칙의 채널이 최근 4주 안에 처음 기록한 항목 중 규칙이 맞는 제목이에요(채널 제외어와 앞선 규칙을 치운 규칙 미리보기와 같은 판단). 그 항목이 어떻게 되었는지는 보지 않아요. 앞선 규칙이 가져갔거나 멈춘 규칙이 본 항목도 새 항목이에요. 작품은 아직 올라오고 있기 때문이에요. 4주 전 바로 그 순간에 받은 항목은 새 항목이 아니에요(경계는 시험이 가려요).
- 기준 시각: 마지막 수신이에요. 한 번도 받지 않은 규칙은 "수집을 시작한 때"부터 세요. 앱이 규칙을 처음 가진 때(`rule_started`), 구독이 된 때, 제목을 받은 때, 마지막으로 다시 켜거나 복원한 때, 채널을 처음 읽은 때 가운데 가장 늦은 시각이에요. 이 기능 전부터 있던 규칙은 업그레이드(마이그레이션 24) 순간이 앱이 규칙을 처음 가진 때예요. 채널을 처음 읽은 때만 보면 오래전에 만든 채널의, 업그레이드 며칠 전에 만든 규칙이 업그레이드하자마자 제안되기 때문이에요. 다시 켜거나 복원하면 4주가 새로 시작해요. 이 기준은 사용자가 정한 것이 아니라 제가 정한 것이어서 아래 "남은 일"에 올렸어요.
- 매일 읽었다면 정확히 4주(`28 * 86_400_000` ms)가 지난 순간부터 제안해요(3주는 아니에요).
- 4주 안의 제목이 한 채널에서 20,000개를 넘으면 그 채널의 규칙에는 `새 항목 없음`을 붙이지 않아요(최근 항목을 다 가려내지 못하니까요). 웹 로그에 한 줄 남겨요. 정규식이 컴파일되지 않는 규칙도 같아요. 둘 다 `방영 종료`는 그대로예요.
- 메모리 한도(128M): 읽을 때마다 규칙마다 색인 조회 한 번, 채널마다 첫 기록 조회 한 번과 읽은 날 조회 한 번(키 조회, 28줄 이하), 시계로는 조용하고 `수집 유지`가 아직 없는 규칙이 있는 채널에서만 4주 치 제목(채널 색인, 최대 20,000개)을 읽어요. 이력 전체를 훑지 않는 것은 `the_last_receive_and_the_recent_titles_are_read_through_indexes_not_the_whole_history`(`EXPLAIN QUERY PLAN`)가 봐요.
- `수집 유지` API는 근거 키의 형식(`ended:<번호>:<날짜>`, `unlisted:<번호>`, `quiet:<ms>`)만 받아요.
- 일괄 보관 중 그 규칙에 이미 열린 보관·복원 명령이 있으면(다른 탭) 그것을 자기 보관으로 여기지 않고 그 규칙에서 멈춰요. 명령 응답이 방향을 말하지 않아서 복원을 보관으로 셀 수 있었기 때문이에요.
- 4주는 채널을 실제로 읽은 기간만 세요(사용자 결정, 2026-10-01). 채널 주소가 죽었거나 worker가 꺼져 있던 주는 조용한 주가 아니에요. worker가 피드 읽기에 성공한 날마다 채널당 하루 한 줄을 남기고(`channel_read_days`, 마이그레이션 24, `StatusStore::record_reads`), 채널마다 가장 최근 28일만 가지고 있어요. 규칙이 `새 항목 없음`이려면 기준 시각(`since`)의 날 *다음*으로 읽은 날이 28일 이상이고 시계로도 4주가 지나 있어야 해요(그래서 4주 전에는 제안이 나오지 않아요). 최근 28일 가운데 가장 오래된 날(`read_day_floors`) 이후에 처음 기록된 항목은 새 항목으로 쳐요. 읽지 못한 날이 끼면 새 항목을 찾는 창도 그만큼 거슬러 올라가요(`Facts::window_start`). 예: 마지막 항목이 5주 전인데 최근 2주를 읽지 못했으면 읽은 기간은 3주라서 제안이 없고, 읽기가 다시 성공한 지 일주일 뒤(읽은 날 28일째)에 나와요. 채널이 계속 실패하는 동안에는 그 규칙에 새 `새 항목 없음` 제안이 생기지 않아요(이미 낸 제안은 그대로예요). 시계가 앞서 갔다 돌아오면 앞선 시계로 남은 미래의 날이 최근 28일 자리를 차지해 요구를 줄일 수 있으므로, `read_day_floors`는 지금까지의 날만 세고 읽기를 기록할 때 그 시각보다 뒤의 날은 지워요(`days_written_by_a_clock_that_was_ahead_do_not_count_once_it_is_back`, `a_clock_that_went_back_takes_the_days_ahead_of_it_with_it`).
- 업그레이드 전 날짜는 알 수 없어서 채널마다 마지막 성공 읽기(`channel_read_status.ok_at`)까지 28일을 읽은 것으로 쳐요. 이전에 시계로 세던 것과 같은 가정이어서 업그레이드 직후 제안이 사라지지 않고, 읽기에 성공한 적 없는 채널은 업그레이드 뒤 28일을 읽을 때까지 이 근거가 없어요. 마이그레이션 24를 이미 적용한 개발 DB는 이 표가 없어서 새로 만들어야 해요(24는 출시 전이라 제자리에서 고쳤어요).
- `수집 유지`가 남기는 것은 사용자가 본 근거 키들이에요. 같은 규칙에 새 근거가 생기면(종영, 또 받고 다시 조용해짐) 다시 제안해요. 규칙의 다른 것은 바꾸지 않아요.
- 구독 탭이 이미 보여 주는 `구독` 목록과 제목 후보는 보관 뒤에 그 자리에서 다시 읽고(`useCached.reload`), 그 규칙 상세에서 보관하면 구독 목록은 버려요.

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`가 통과했고(master의 0024를 합친 뒤에도), `cargo test --no-fail-fast`는 1099개 통과, 실패 0개였어요(lib 880개). 합치기 전 첫 전체 실행에서 시간에 민감한 `artwork::tests::hundreds_of_new_works_are_searched_one_at_a_time_at_the_pace`가 다른 에이전트의 부하로 한 번 실패했고(이 변경이 건드리지 않은 시험, 단독 실행은 통과), 이어진 전체 실행은 모두 통과했어요. 웹은 `bun install --frozen-lockfile`, `bun run typecheck`, `bun run build`가 통과했어요.
- 완료 기준의 행과 시험·관찰(자동 시험은 가짜 시계의 `App`, 브라우저는 로컬 `trss-web`·`trss-worker`·가짜 Transmission과 스크래치 DB, 2026-10-01):
  - 종영일이 지남: `a_subscription_whose_end_date_has_passed_is_suggested_and_says_so`(API), `a_subscription_whose_end_date_has_passed_is_suggested_as_ended`·`an_end_date_that_is_today_or_ahead_has_not_ended_the_anime`(판단). 브라우저: 구독 탭과 규칙 상세에 `Anissia에서 방영이 끝났어요. 종영일은 …`.
  - 종영일 없는 구독이 편성표에서 빠짐: `an_anime_without_an_end_date_is_suggested_once_the_refresh_found_it_unlisted`, `an_end_date_ahead_wins_over_a_missing_listing`. Anissia에 닿지 못하는 동안에는 새로 고침이 아무것도 기록하지 않으므로(0021) `unlisted_at`이 없어 제안이 생기지 않아요. 닿지 못함 자체를 만들어 보지는 않았어요.
  - 3주와 4주: `three_weeks_without_a_new_item_is_not_enough_and_four_weeks_is`(API, 정확히 4주 직전과 정각의 경계까지), `three_weeks_after_the_last_receive_is_not_quiet_and_four_weeks_is`. 브라우저: 3주 전에 받은 규칙은 제안에 없고 5주 전에 받은 규칙은 있어요.
  - `멈춤`은 받고 보관·제목 대기는 못 받음: `a_paused_rule_is_suggested_but_an_archived_one_and_a_waiting_subscription_are_not`, `a_paused_rule_is_suggested_on_the_same_ground`, `an_archived_rule_and_a_subscription_waiting_for_its_title_are_never_suggested`. 브라우저에서도 같아요.
  - 셋 중 하나를 풀고 `2개 보관`: 브라우저에서 같은 작품 폴더(`Alpha`)를 쓰는 두 규칙과 하나의 다른 규칙으로 해 봤고, 풀지 않은 둘이 목록 순서대로 명령이 되어(명령 표의 두 줄: 첫째는 `kept`, 둘째는 `moved`) 보관 폴더에 `Alpha`가 한 번 옮겨졌고 푼 규칙은 제안에 남아 한 줄로 보였어요. 명령의 폴더 이동은 0011의 시험(`crates/trss-worker/tests/archive_move.rs`)이 봐요.
  - 제안이 하나: 위 관찰의 마지막 상태에서 접힘 없이 그 작품 한 줄과 `보관`이었어요. 여럿일 때는 `보관 제안 N개`와 두 버튼의 한 줄이었어요.
  - 규칙 상세의 `수집 유지`: `keeping_collecting_hides_the_same_ground_but_not_a_new_one`, `a_kept_ground_does_not_suggest_again_and_a_new_one_does`, `a_rule_that_receives_and_falls_quiet_again_is_a_new_ground`, 저장소의 `a_kept_ground_is_remembered_once_and_goes_with_its_rule`. 브라우저: 누르면 배너가 사라지고 종영일이 바뀐 새 근거에서는 다시 나타났어요(복원해 두었어요).
  - 읽은 기간만 세기: `weeks_the_channel_could_not_be_read_are_not_quiet_weeks`·`the_ground_appears_when_reading_has_made_up_the_four_weeks`·`the_day_of_the_moment_itself_is_not_one_of_the_28`·`the_clock_still_has_to_reach_four_weeks_when_the_days_are_there`·`the_window_reaches_back_to_the_first_of_the_read_days`(판단), API의 `weeks_the_channel_could_not_be_read_are_not_quiet_weeks`(5주 전 항목, 최근 2주 읽기 실패: 제안 없음, 읽기가 돌아오고 7일째에 제안)·`a_worker_that_was_off_for_four_weeks_suggests_nothing_when_it_starts_again`·`a_channel_never_read_for_28_days_gives_no_quiet_ground_at_all`, 읽은 날 저장 `crates/trss-collect/src/store/status/tests.rs`의 읽은 날 시험 5개, 업그레이드 `a_database_from_before_read_days_takes_the_28_days_up_to_each_last_success`, worker 주기가 성공한 읽기만 읽은 날로 남기는 것 `crates/trss-worker/tests/status_snapshots_from_cycle.rs`의 `only_a_cycle_that_read_the_feed_leaves_a_read_day`.
  - 메뉴 배지: `web/src/app/todo-count.ts`가 제안을 세지 않고(지금은 항상 비어 있어요), 제안이 있는 화면의 메뉴에 배지가 없는 것을 봤어요.
  - PC와 휴대폰: 1024px에서 두 묶음의 위쪽이 같고 왼쪽이 달라 나란히였고, 375px에서 위아래로 쌓였으며 구독 탭(접힘·펼침)과 규칙 상세에서 `scrollWidth == innerWidth`(375)였어요.
- 시험이 실패하는 것을 본 것(변이): 읽은 날을 세지 않고 시계만 보게 하면(읽은 기간 시험 7개: 판단 4개, API 3개) 실패했어요(되돌리면 통과). 4주를 3주로 바꾸면 시험 6개가, `수집 유지` 거르기와 맞는 새 항목 확인을 빼면 8개가 실패했어요(되돌리면 통과). 이 과정에서 `titles_since`가 경계를 포함하고 있어 정확히 4주 전에 받은 항목이 새 항목으로 세어지는 오류를 찾아 고쳤고, 경계 시험을 더했어요. 화면 쪽은 자동 시험 도구가 없어요(저장소에 웹 시험이 없어요).

### 검증하지 못한 것

- 실제 Transmission, 실제 Anissia(429·단절 포함), 실제 휴대폰·스크린 리더, 라이트 모드, 376–1099px 너비는 보지 않았어요. 폴더 이동은 가짜 Transmission(토런트 없음)으로 폴더 이름 바꾸기만 봤어요.
- 한 채널의 4주 제목이 20,000개를 넘는 경우는 "읽다 만 창" 신호를 넣은 판단 시험(`a_window_that_was_cut_short_or_a_broken_regex_gives_no_quiet_ground`)으로만 봤고, 실제 그 규모의 이력으로는 보지 않았어요.
- 보관 도중 탭을 닫거나 서버가 끊기는 경우는 만들어 보지 않았어요(같은 ID로 다시 보내는 경로는 0011의 명령 계약에 기대요).

### 남은 일

- 한 번도 받지 않은 규칙의 기준 시각과 "새 항목"의 정의는 이 티켓의 해석으로 표시해 명세의 규칙 보관에 옮겼어요.
- 알려진 한계: `새 항목 없음`이 최근 4주 안의 항목을 읽는 방식이라 채널 하나가 4주에 20,000개를 넘게 올리는 피드는 이 근거가 꺼져요. 그런 피드가 실제로 생기면 상한을 올리거나 규칙별 조회로 바꿔요.
- 할 일 화면이 생기면 `archive_api::archive_suggestions`를 `제안` 줄의 원천으로 읽으면 돼요.
