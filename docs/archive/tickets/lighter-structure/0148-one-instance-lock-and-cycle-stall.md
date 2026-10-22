# 0148 잠금을 단일 실행 잠금 하나로 바꾸고 수집 멈춤을 수집 주기만으로 판정해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름)의 단일 실행 잠금, [명령](lighter-structure.md#명령)의 `worker_heartbeat`, [수집이 멈췄는지의 판정](lighter-structure.md#수집이-멈췄는지의-판정), [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0147](0147-commands-start-at-once.md)(명령 바로 실행)

## 작업

[0147](0147-commands-start-at-once.md) 뒤에도 프로세스 안에는 worker 잠금, 대기열마다의 잠금 파일, 브라우저 잠금, 수집 주기의 두 번째 실행 막기가 남아요. 모두 같은 앱 데이터를 쓰는 프로세스가 둘 이상일 때를 막던 장치예요. 이 티켓은 프로세스가 시작할 때 잡는 잠금 하나로 넷을 대신해요.

worker 잠금은 잡혀 있는 동안 하트비트를 이어 가고, 상태판은 그 하트비트로 worker가 도는지 판정해요. 잠금과 함께 하트비트가 없어지므로, 멈춤 판정을 수집 주기만 보게 바꾸고 멈춤 문구도 함께 바꿔요. 화면이 보인다면 프로세스는 살아 있으므로, worker가 도는지 확인하라는 지금 문구는 맞지 않아요.

- 명령의 claim은 [0147](0147-commands-start-at-once.md) 뒤에도 worker 잠금 아래에서 해요. 두 프로세스가 한 명령을 두 번 실행하지 않게 막는 것이 이 잠금이라서, 단일 실행 잠금이 들어오는 이 티켓에서 잠금과 함께 없애요(2026-10-21).
- 넷이 막던 것 중 프로세스 안에서도 지켜야 하는 것은 프로세스 안에서 지켜요. 대기열마다 실행하는 쪽이 하나이고, 서버 브라우저의 수명을 맡는 쪽이 하나이고, 수집 주기가 겹쳐 돌지 않아요.
- 화면이 멈춤을 이벤트로 아는 일은 [0156](0156-events-for-covers-schedule-status.md)이 해요. 이 티켓에서는 지금처럼 화면이 주기적으로 읽어서 알아요.

## 완료 기준

- [x] 같은 앱 데이터 폴더로 두 번째 `trss`를 시작하면, 그 프로세스는 이미 도는 프로세스가 있다고 로그에 남기고 멈추고, 수집이나 명령을 하나도 실행하지 않아요. 두 프로세스를 띄우는 테스트가 확인해요.
- [x] worker 잠금(명령의 claim이 잡던 것 포함), 대기열마다의 잠금 파일, 브라우저 잠금, 수집 주기의 두 번째 실행 막기가 코드에서 없어져요. 대기열마다 실행하는 쪽이 하나이고, 서버 브라우저의 수명을 맡는 쪽이 하나이고, 수집 주기가 겹쳐 돌지 않는 것을 테스트가 확인해요.
- [x] 하트비트가 없어지고, 마이그레이션이 `worker_heartbeat` 테이블을 지워요.
- [x] 상태판의 멈춤 판정은 수집 주기만 봐요. 진행 중인 주기가 `max(30분, 주기 간격의 10배)`를 넘게 돌거나, 끝난 주기의 다음 확인 시각이 한 주기 넘게 지났는데 다음 주기가 시작하지 않았으면 멈춘 것이에요. 감시 폴더 읽기와 명령은 판정에 들어가지 않아요. 이 경우들을 판정 규칙의 테스트가 확인해요.
- [x] 멈췄을 때 편성 옆 열의 `다음` 자리와 수집 상태판의 RSS 칸은 `RSS 확인이 멈췄어요. 컨테이너 로그를 확인해 주세요.`를 강조 색으로 보여요. 휴대폰 요약의 `RSS 멈춤`은 그대로예요.
- [x] 같은 커밋에서 다음 문서를 고쳐요.
  - [웹 앱 공통 명세의 이번 주 편성](../../../specs/web-app.md#이번-주-편성)의 멈춤 판정과 문구
  - [웹 앱 공통 명세의 구현 경계와 실행 순서](../../../specs/web-app.md#구현-경계와-실행-순서)에서 잠금과 하트비트를 적은 부분
  - [작업과 인증 명세의 작업 화면 안의 인증과 브라우저 수명](../../../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명)에서 브라우저 잠금을 적은 부분
  - [readme](../../../../readme.md)가 앱 데이터 폴더의 파일과 시작할 때의 쓰기 확인으로 적은 잠금 파일 6개, 대기열마다 잠금을 잡는다고 적은 문장
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요. 지운 테스트는 결과에 그 까닭과 짝지어 적어요.
- [x] 구현이 [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.

## 결과

2026-10-21에 worker 잠금, 대기열마다의 잠금 파일, 브라우저 잠금, 수집 주기의 두 번째 실행 막기를 없애고, `trss`가 시작할 때 잡는 단일 실행 잠금(`trss_core::InstanceLock`) 하나로 대신했어요. 하트비트와 `worker_heartbeat` 테이블도 없앴고, 상태판은 멈춤을 수집 주기만 보고 판정해요.

- 잠금 파일은 DB 옆의 `trss.db.lock`이에요. `trss`는 앱 데이터의 쓰기 확인 뒤, DB를 열기 전에 이 파일에 `flock`을 걸어요. 이미 잡혀 있으면 `another trss runs on this app data (it holds …); this one stops`를 남기고 0이 아닌 코드로 끝나요. 잠금은 커널이 프로세스와 함께 풀어서, 죽은 프로세스가 잠금을 남기지 않아요. `trss`는 잡은 잠금을 놓지 않고 프로세스가 끝날 때 커널이 풀게 둬요. 그래서 명령이 끝난 뒤에도 남은 폴더 이름 바꾸기처럼, 런타임이 끝나기 전에 기다리는 일이 도는 동안에도 잠금이 잡혀 있어요.
- 시작할 때 0147까지의 릴리스가 남긴 파일 일곱 개, 곧 잠금 파일 여섯 개(`.worker.lock`, `.browser.lock`, `.artwork.lock`, `.seasons.lock`, `.anissia.lock`, `.anissia-captions.lock`)와 `.wake` 소켓을 지우고 그 일을 로그에 남겨요. 0147이 소켓만 지우던 것을 넓혔어요. 시작할 때의 쓰기 확인은 `trss.db.lock` 하나만 봐요.
- "수집 주기의 두 번째 실행 막기"는 주기의 시작 간격(`min_gap`, 주기 간격의 절반) 안에 시작하는 주기를 건너뛰던 것으로 읽었어요(구현 결정, 2026-10-21). 이 간격은 함께 시작한 두 worker가 한 주기를 한 번만 돌게 하던 장치예요. 이제 다시 시작한 `trss`는 앞 주기가 막 시작했더라도 곧바로 주기를 돌고, 앞 프로세스가 끝내지 못한 주기 표시도 그 시작이 바로 바꿔요.
- 서버 브라우저가 설정되지 않은 `trss`도 시작할 때 앞 프로세스의 원격 화면 연결을 닫아요. 0147까지는 브라우저 잠금을 다른 worker가 쥐고 있으면 그 worker의 것일 수 있어서 남겨 뒀는데, 이제 같은 앱 데이터의 다른 프로세스가 없어요(구현 결정, 2026-10-21).
- 명령 흐름이 명령을 찾다 3초 뒤에 다시 찾는 경우는 DB 오류만 남았어요. 0147 결과의 여섯째 경우, 곧 다른 프로세스가 worker 잠금을 쥔 경우는 없어졌어요.

### 프로세스 안에서 지키는 것

넷이 막던 것 중 한 프로세스 안에서도 지켜야 하는 것은 아래처럼 지켜요. 앞의 셋은 0147까지도 프로세스 안에서 같은 방법으로 지켰고, 대기열과 브라우저 풀은 잠금 파일 대신 새로 지켜요.

| 지키는 것 | 프로세스 안의 방법 | 확인하는 테스트 |
| --- | --- | --- |
| 수집 주기가 겹쳐 돌지 않음 | `cycling` 뮤텍스. 돌고 있으면 `TickOutcome::Busy` | `trss` `tests/it/worker_cycle.rs` `a_second_cycle_of_the_process_skips_while_the_first_is_running` |
| 자막 작업의 실행이 하나 | `jobs_running` 뮤텍스 | `trss` `worker/jobs.rs`의 기존 테스트 |
| 같은 폴더를 건드리는 일의 차례 | 폴더 차례(`FolderLocks`)와 토렌트 관문 | `trss` `tests/it/archive_move.rs` `the_same_workers_cycle_adds_beside_a_move_and_skips_the_rule_archived_meanwhile` 등 기존 테스트 |
| 대기열마다 실행하는 쪽이 하나 | `Queue`마다의 실행 표시. 두 번째 실행은 곧바로 돌아가요 | `trss-core` `queue.rs` `a_second_run_of_one_queue_returns_at_once_while_the_first_goes_on`, `a_queue_runs_again_once_its_run_stopped_or_was_aborted` |
| 서버 브라우저의 수명을 맡는 풀이 하나 | 프로세스 전역 표시. 두 번째 `connect`는 거절되고, 풀을 만들지 못한 `connect`는 표시를 되돌려요 | `trss` `worker/browser.rs` `a_process_makes_one_pool_over_the_browser_container` |

대기열의 실행 표시는 `Queue` 값마다 있고 복제한 값끼리 나눠 가져요. `Artwork`, `Seasons`, Anissia 대기열, Anissia 자막 줄 읽기가 각자 하나씩 갖고, `trss`는 시작할 때 돌릴 넷을 하나씩 만들어 한 번씩 돌려요. web 쪽이 쓰는 `Artwork`도 대기열을 갖지만 돌리지 않아요. 같은 이름의 대기열을 프로세스 전체에서 막지 않는 까닭은 테스트가 한 바이너리 안에서 저마다 대기열을 만들어 함께 돌리기 때문이에요(구현 결정, 2026-10-21).

### 멈춤 판정

판정은 `trss_collect::store::history::CycleState`의 `is_stalled`와 `is_running`이 맡아요. 끝나지 않은 주기는 `max(30분, 주기 간격의 10배)`까지 도는 중이고 그 뒤로는 멈춘 것이에요. 끝난 주기는 다음 확인 시각(시작 시각 + 간격)이 한 간격 넘게 지나면 멈춘 것이에요. 0147까지 하트비트가 없을 때 쓰던 판정과 같아요. 상태판의 `cycle.stalled`와, 편성이 `영상 받는 중`을 남겨 두는 판정이 이 둘을 써요. 화면의 문구는 `RSS 확인이 멈췄어요. 컨테이너 로그를 확인해 주세요.`로 바꿨고, 휴대폰 요약의 `RSS 멈춤`은 그대로예요.

하트비트가 있을 때는 죽은 worker의 끝나지 않은 주기를 1분 만에 멈춤으로 보였어요. 이제는 프로세스가 죽으면 화면도 함께 사라지고, 다시 시작한 프로세스가 곧바로 새 주기를 시작해요. 그래서 살아 있는 프로세스의 주기가 걸린 경우만 남고, 이 경우는 `max(30분, 주기 간격의 10배)`가 지나야 멈춤으로 보여요. 간격이 300초면 50분이에요.

주기는 끝을 적은 뒤 같은 차례에서 감시 폴더를 읽고 시즌을 연결하고, 그동안 다음 주기는 시작하지 않아요. 이 일이 길어 다음 확인이 한 간격 넘게 늦으면, 프로세스가 일하는 중이어도 멈춤으로 보여요. 0147까지는 이 동안 하트비트가 멈춤을 가렸어요. 명세대로 감시 폴더 읽기를 판정에 넣지 않고 이대로 두었어요(구현 결정, 2026-10-21). 이 늦음은 실제로 RSS 확인이 늦는 것이라 문구가 틀리지 않고, 2026-10-21 개발 환경에서 서버의 파일 이름을 옮긴 감시 폴더 두 개를 읽는 데 각각 20 ms와 34 ms가 걸려서, 기본 간격으로 계산한 10분을 채우기 어려워요. 주기 끝을 폴더 읽기 뒤로 옮기면 이 동안도 도는 중으로 보일 수 있어요. [웹 앱 공통 명세의 이번 주 편성](../../../specs/web-app.md#이번-주-편성)에서 "감시 폴더 읽기는 주기와 따로 돈다"고 적었던 문장을 이 순서에 맞게 고쳤어요.

### 테스트

2026-10-21에 이 티켓의 변경으로 `cargo test --locked --workspace -j 4`를 돌렸더니 통과 3,055개, 실패 0개, 건너뛴 것 14개였어요. 0147의 통과 3,088개에서 아래의 지운 테스트 38개를 빼고, 옮기거나 더한 테스트 5개를 더한 수예요. 테스트 바이너리 시간의 합은 43.9초로 0147의 43.8초와 비슷했어요. `cargo clippy --workspace --all-targets -j 4`에 경고가 없고, 웹의 `npm run typecheck`와 `npm test`(238개)도 통과했어요. 원본 로그는 이 PC의 `dev/local/measure/0148/`에 있어요.

지운 테스트와 그 까닭은 아래와 같아요.

- `trss-core` `lock.rs`
  - `every_lock_file_has_its_own_name_next_to_the_database`: 잠금 파일 여섯 개의 이름이 서로 다른지 확인했는데, 잠금 파일이 하나가 됐어요.
  - `WorkerLock`의 공유 잡기 테스트 9개(`one_workers_work_shares_the_hold_and_another_worker_waits`, `the_heartbeat_holds_from_the_first_handle_until_the_last_lets_go`, `a_last_handle_dropped_without_letting_go_still_ends_the_hold`, `what_keeps_the_lock_ends_the_hold_when_it_outlives_the_handles`, `a_hold_taken_while_the_last_one_ends_waits_for_its_last_beat`, `a_hold_given_up_while_the_last_one_ends_leaves_the_wait_to_the_next`, `a_hold_taken_while_the_last_one_is_dropped_on_another_thread_waits_for_it`, `a_hold_given_up_while_its_first_beat_is_written_clears_it_before_the_lock_goes`, `what_keeps_the_lock_outlives_its_handle`): `WorkerLock`과 그 하트비트가 없어졌어요.
- `trss-core` `heartbeat.rs`의 테스트 12개: 하트비트가 없어졌어요. 하트비트 없이도 뜻이 있는 셋은 `trss-collect` `store/history/tests.rs`로 옮겼어요. `a_beat_is_fresh_for_a_minute_and_a_cycle_runs_for_half_an_hour_at_least`는 주기 한도 부분만 남겨 `a_cycle_runs_for_half_an_hour_at_least_and_ten_intervals_with_a_long_one`이 됐고, `a_finished_cycle_is_stalled_once_the_next_check_is_more_than_one_interval_overdue`는 이름 그대로, `without_a_heartbeat_an_unfinished_cycle_runs_until_the_larger_of_half_an_hour_and_ten_intervals`는 `an_unfinished_cycle_runs_until_the_larger_of_half_an_hour_and_ten_intervals_and_then_is_stalled`가 됐어요. 나머지 9개(`a_stale_heartbeat_tells_a_worker_killed_in_a_cycle_from_a_slow_one`, `a_worker_that_beats_through_a_long_folder_scan_is_not_stalled_but_one_that_does_not_come_back_is`, `a_worker_that_beats_but_holds_the_lock_past_the_bound_is_stalled`, `an_idle_worker_with_a_stale_heartbeat_is_stalled_only_when_the_next_check_is_overdue`, `the_worker_is_busy_while_it_beats_until_it_holds_the_lock_past_the_bound`, `the_heartbeat_is_whatever_the_worker_last_wrote`, `the_first_beat_is_written_before_start_returns`, `it_beats_while_held_and_clears_the_hold_when_stopped`, `a_dropped_heartbeat_stops_beating_and_keeps_the_hold`)는 하트비트가 있을 때의 판정이나 하트비트를 적는 일만 확인했어요.
- `trss-core` `queue.rs`의 `a_queue_whose_lock_is_taken_elsewhere_waits_and_starts_when_it_is_let_go`, `a_lock_file_that_cannot_be_opened_is_tried_again_like_one_taken_elsewhere`: 대기열의 잠금 파일이 없어졌어요.
- `trss-collect` `store/history/tests.rs`의 `two_handles_race_for_one_cycle_start`: 두 DB 핸들이 시작 간격 안의 주기 시작을 하나만 받는지 확인했는데, 시작 간격이 없어졌어요.
- `trss` `worker/jobs.rs`의 `a_worker_that_does_not_hold_the_browser_lock_leaves_the_screens_alone`: 브라우저 잠금이 없어져서 원격 화면 연결을 남겨 두는 경우가 없어요.
- `trss` `tests/it/archive_move.rs`의 `no_cycle_runs_while_a_folder_moves`: 폴더를 옮기는 동안 다른 worker의 주기와 명령 찾기가 잠금에 막히는지 확인했어요. 같은 프로세스의 주기가 이동 옆에서 도는 것은 `the_same_workers_cycle_adds_beside_a_move_and_skips_the_rule_archived_meanwhile`가 확인해요.
- `trss` `tests/it/worker_cycle.rs`의 `workers_started_together_run_each_period_once`: 함께 시작한 두 worker가 한 주기를 한 번만 도는지 확인했는데, 두 번째 프로세스는 이제 시작하지 못해요.
- `trss` `tests/it/status_snapshots_from_cycle.rs`의 `a_worker_beats_while_a_cycle_holds_the_lock_and_clears_the_hold_after_it`, `a_cycle_that_does_not_start_leaves_no_hold`: 주기 동안의 하트비트를 확인했어요.
- `trss` `tests/it/live_watch.rs`의 `a_reading_an_alert_asked_for_beats_under_the_lock_and_lets_go_of_the_hold`, `work_read_by_an_alert_waits_for_the_workers_lock`: 알림이 부른 읽기가 worker 잠금 아래에서 하트비트를 적는지와, 다른 쪽이 쥔 worker 잠금을 기다리는지 확인했어요. 읽기는 이제 폴더 차례만 잡아요.
- `trss` `tests/it/receive_once.rs`
  - `two_workers_never_run_one_command_twice`: 두 worker가 한 명령을 두 번 실행하지 않는지 확인했는데, 두 번째 프로세스는 시작하지 못해요. 한 프로세스 안에서는 claim이 명령을 한 번만 집어요.
  - `a_command_waits_while_a_cycle_holds_the_lock`: 다른 쪽의 주기가 쥔 worker 잠금을 명령이 기다리는지 확인했어요.
  - `a_busy_lock_is_not_an_error_and_the_next_look_takes_the_command`: 잠금이 잡혀 있을 때의 `CommandsOutcome::Busy`를 확인했는데, 이 결과가 없어졌어요.
  - `a_long_command_beats_and_the_board_shows_no_stall_meanwhile`: 오래 걸리는 명령이 하트비트를 적는지 확인했어요. 이제 명령은 판정에 들어가지 않아요.
- `trss` `tests/it/worker_process.rs`의 `two_worker_processes_run_a_period_once`, `the_lock_alone_keeps_a_second_process_out_of_a_running_cycle`: 두 프로세스가 잠금으로 한 주기를 나눠 도는지 확인했어요. 두 번째 프로세스가 멈추는 것을 확인하는 아래 테스트로 바꿨어요.

이름이나 내용을 바꾼 테스트는 아래와 같아요.

- `trss` `tests/it/worker_process.rs` `a_second_process_on_the_same_app_data_stops_before_it_runs_anything`: 첫 `trss` 프로세스가 피드를 읽다 붙잡힌 동안 같은 앱 데이터로 둘째를 1초 주기로 띄워요. 둘째는 10초 안에 0이 아닌 코드로 끝나고, 위 문구를 남기고, `trss listening`·`trss started`·`Cycle started`를 남기지 않아요. 피드는 한 번만 읽혔고 토렌트 추가는 없어요. 첫째는 주기를 끝내 세 항목을 받고, 첫째가 멈춘 뒤에는 셋째가 시작해요.
- `trss` `tests/it/worker_cycle.rs`의 `a_second_worker_skips_while_the_first_is_running`은 `a_second_cycle_of_the_process_skips_while_the_first_is_running`이 됐어요. 다른 worker 대신 같은 worker의 복제로 주기를 겹쳐 시작해요.
- `trss` `tests/it/receive_once.rs`의 `a_look_that_finds_the_lock_taken_looks_again_by_itself`는 `a_look_that_fails_looks_again_by_itself`가 됐어요. 명령 흐름이 시작할 때 `commands` 테이블의 이름을 잠시 바꿔 찾기를 DB 오류로 실패시키고, 테이블을 되돌리면 흐름이 스스로 다시 찾아 명령을 한 번에 끝내요.
- `trss-core` `queue.rs`: `two_queues_on_one_lock_file_do_not_run_side_by_side`는 `a_second_run_of_one_queue_returns_at_once_while_the_first_goes_on`으로, `a_queue_holds_its_lock_while_it_runs_and_lets_go_when_it_stops`는 `a_queue_runs_again_once_its_run_stopped_or_was_aborted`로, `upkeep_runs_once_the_lock_is_taken_and_then_every_period_between_items`는 `upkeep_runs_at_the_start_and_then_every_period_between_items`로 바꿨어요. `cancel_ends_a_queue_in_each_of_its_waits`에서는 잠금을 기다리는 경우를 뺐어요.
- `trss-core` `lock.rs`: `lock_path_sits_next_to_the_database`는 `the_lock_file_sits_next_to_the_database`로, `a_second_holder_is_refused_until_the_first_lets_go`는 `a_second_taker_is_refused_until_the_first_lets_go`로 바꿨어요. `access.rs`의 `the_checked_suffixes_hold_every_lock_file_once`는 `the_lock_file_is_checked`가 됐어요.
- `trss-collect` `store/history/tests.rs`의 `cycle_marker_admits_one_start_per_gap`은 `the_cycle_marker_keeps_the_last_start_and_its_end`가 됐어요. 곧바로 다음 주기가 시작해도 표시가 바뀌어요. `a_finished_cycle_is_not_running`을 더했어요.
- `trss-collect` `store/status/tests.rs`의 `a_database_from_before_the_heartbeat_has_none_until_the_worker_writes_one`은 `the_heartbeat_an_earlier_release_left_goes_and_the_cycle_interval_stays`가 됐어요. 하트비트가 적힌 63번 마이그레이션의 DB를 64번으로 올리면 테이블이 없어지고 주기 간격은 남아요.
- `trss` `web/status_api/tests.rs`의 `a_worker_killed_in_a_cycle_is_stalled_once_its_heartbeat_is_stale`은 `a_cycle_without_an_end_past_its_bound_is_stalled`가, `web/schedule_api/tests.rs`의 `a_worker_killed_in_a_cycle_no_longer_holds_the_downloading_look`은 `a_cycle_that_runs_holds_the_downloading_look_until_it_outlives_its_bound`가 됐어요.
- `trss` `worker/browser.rs`의 `one_worker_at_a_time_has_the_browser`는 `a_process_makes_one_pool_over_the_browser_container`가, `worker/jobs.rs`의 `a_worker_that_holds_the_browser_lock_without_the_browser_closes_the_leftover_screens`는 `a_worker_without_the_browser_closes_the_leftover_screens`가 됐어요.

### 개발 환경에서 확인한 것

2026-10-21에 이 티켓의 변경으로 `dev/compose.sh up`을 실행해, 0147의 이미지가 쓰던 앱 데이터로 `trss`를 띄웠어요. 아래 리뷰에서 고치기 전의 코드였고, 고친 것은 잠금을 놓는 때와 브라우저 풀을 만들지 못했을 때뿐이라 아래 확인에는 닿지 않아요. 결과는 이 PC의 `dev/local/measure/0148/`에 있어요.

- 시작 로그에 `trss: removed a file of an earlier release (/data/trss.db.worker.lock)`부터 `.anissia-captions.lock`까지 여섯 줄이 나오고, 앱 데이터 폴더에는 잠금 파일로 `trss.db.lock` 하나만 남았어요. 소켓 파일은 0147이 이미 지워서 없었어요.
- 도는 컨테이너 안에서 `docker exec trss /usr/local/bin/trss`로 같은 앱 데이터의 두 번째 `trss`를 실행했어요. `trss: another trss runs on this app data (it holds /data/trss.db.lock); this one stops`를 남기고 코드 1로 끝났어요.
- 도는 동안 DB의 주기 표시를 `sqlite3`로 바꾸고 `GET /api/collect/status`의 `cycle.stalled`를 읽었어요. 주기 간격은 300초예요. 끝난 주기는 9분 전에 시작했으면 `false`, 11분 전이면 `true`였어요. 끝나지 않은 주기는 20분·31분·49분 전에 시작했으면 `false`, 51분 전이면 `true`였어요. 한도가 `max(30분, 50분)`이라 31분은 아직 도는 중이에요.
- 60분 전에 끝난 주기로 바꿔 두고 앱의 내장 브라우저로 열었더니, 편성 옆 열의 `다음` 자리와 수집 화면의 상태판에 `RSS 확인이 멈췄어요. 컨테이너 로그를 확인해 주세요.`가 나왔어요. 편성 옆 열에서는 강조 색인 것을 화면으로 봤고, 상태판의 색은 보지 않았어요. 확인한 뒤 주기 표시를 원래 값으로 되돌렸어요.

### ADR과의 관계

[한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)은 결정의 결과로 깨우기 소켓, 3초 주기 조회, worker 잠금 아래의 claim, heartbeat가 없어진다고 적어요. 0147이 앞의 둘을, 이 티켓이 뒤의 둘을 없앴어요. 끝난 모습은 ADR과 같아서 ADR은 고치지 않았어요.

### 리뷰와 고친 것

`stora:complex-reviewer`가 커밋 전의 변경을 리뷰했어요. 프로세스 안에서 주기, 명령, 알림이 부른 읽기, 다시 읽기가 엇갈리는 경우, 잠금 없는 claim, 자막 작업과 원격 화면의 시작 순서, 대기열과 브라우저 풀의 표시, 단일 실행 잠금의 수명, 마이그레이션, 판정이 이전과 같은지, 새 테스트가 이름대로의 동작에 닿는지를 봤고, 막는 문제는 찾지 않았어요. worker 잠금은 한 worker의 일들이 함께 나눠 잡는 잠금이라 프로세스 안의 일들을 서로 떼어 놓지 않았으므로, 없애도 프로세스 안의 엇갈림은 그대로라고 봤어요. 리뷰어가 명령 찾기에서 실행 중인 명령을 빼는 조건을 지우자 `receive_once` 테스트 20개가 실패했고, `a_look_that_fails_looks_again_by_itself`는 혼자 60번, 8개씩 함께 120번 돌려 한 번도 실패하지 않았어요. 다음을 고쳤어요.

- `trss`가 단일 실행 잠금을 `run`이 끝날 때 놓던 것을, 놓지 않고 프로세스가 끝날 때 커널이 풀게 바꿨어요. `run`이 끝난 뒤에도 런타임은 남은 폴더 이름 바꾸기를 기다리는데, 그동안 잠금이 풀려 있으면 다음 프로세스가 시작할 수 있었어요. Compose는 앞 컨테이너가 끝난 뒤 다음을 시작하므로, 손으로 띄운 경우에만 닿는 일이에요.
- 브라우저 풀을 만들지 못한 `connect`가 프로세스 전역 표시를 남겨, 다시 부르면 "이미 만들었다"고 거절하던 것을 고쳤어요. 만들지 못하면 표시를 되돌려요.
- readme의 서버 브라우저 문단이 아직 `trss.db.browser.lock`을 적고 있어서, 두 번째 `trss`가 시작할 때 멈춘다고 고쳤어요. 자막 명세의 검증 표에서 없어진 잠금 테스트를 적은 문장, 통합 테스트 하나와 `trss-jobs` `recheck.rs`의 "다른 worker"를 적은 주석도 고쳤어요.
- 감시 폴더 읽기가 길면 멈춤으로 보이는 경우는 위 멈춤 판정에 적은 대로 그대로 두고, 명세의 문장만 고쳤어요.
- 대기열의 표시가 `Queue` 값마다라는 것과 돌릴 대기열을 만드는 곳을 `Queue`의 문서와 `main`의 주석에 적었어요.

고친 뒤 `cargo test --locked --workspace -j 4`를 다시 돌렸더니 통과 3,055개, 실패 0개, 건너뛴 것 14개였고, 테스트 바이너리 시간의 합은 43.8초였어요. clippy에도 경고가 없어요.

### 문서

완료 기준에 적은 네 곳을 고쳤어요. [이번 주 편성](../../../specs/web-app.md#이번-주-편성)에는 바뀐 문구와 판정 규칙을, [구현 경계와 실행 순서](../../../specs/web-app.md#구현-경계와-실행-순서)에는 잠금 파일 하나와 두 번째 `trss`가 멈추는 것, 프로세스 안에서 하나씩만 도는 것을 적었어요. [작업 화면 안의 인증과 브라우저 수명](../../../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명)에서는 브라우저 잠금 대신 같은 앱 데이터의 `trss`가 하나이고 풀을 한 번만 만든다고 적었어요. readme에서는 앱 데이터 폴더의 파일, 시작할 때의 쓰기 확인, 대기열의 잠금, 멈춤 판정을 고쳤어요.

없어진 이름(`WorkerLock`, `CycleLock`, `heartbeat`, `min_gap`, 잠금 파일 이름)과 "worker lock"으로 찾아, 자막 명세의 Anissia 자막 줄 읽기 문장, 웹 앱 공통 명세와 작업 명세의 검증 표 네 행, `docker-compose.trss.yml`의 잠금 파일 주석, 웹의 상태판 API 주석, `trss-core`·`trss-collect`·`trss-library`·`trss-jobs`·`trss-transmission`·`trss`의 코드 주석도 고쳤어요. 작업 명세의 서버 브라우저 검증 행에서는 `.browser.lock`과 유휴 종료의 순서를 확인하지 않았다는 문장을 지웠어요. 끝난 티켓과 ADR 0011·0016·0019는 그때의 기록이라 그대로 뒀어요.

### 확인하지 않은 것

- 서버(j4105)에 올려 확인하지 않았어요. [구조 가볍게 하기 명세](lighter-structure.md)는 이 변경을 모두 마친 뒤 사용자 승인을 받아 서버에 올려 확인하기로 했어요.
- 단일 실행 잠금이 런타임이 기다리는 일이 끝날 때까지 잡혀 있는 것은 코드와 tokio의 런타임 종료 동작을 읽고 판단했고, 테스트로 재현하지 않았어요.
- 브라우저 풀을 만들지 못했을 때 표시를 되돌리는 것은 테스트가 없어요. `BrowserPool::new`은 HTTP 클라이언트를 만들지 못할 때만 실패해서, 그렇게 만들 입력이 없어요.
- 0147의 컨테이너가 아직 멈추지 않은 동안 0148의 컨테이너가 시작하는 경우는 확인하지 않았어요. 0147은 `trss.db.lock`을 잡지 않으므로 이때는 두 프로세스가 함께 돌 수 있어요. Compose는 컨테이너를 바꿀 때 앞의 것을 멈춘 뒤 새것을 시작해요.
