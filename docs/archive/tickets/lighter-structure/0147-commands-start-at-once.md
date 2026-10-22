# 0147 명령과 자막 작업을 적은 곳에서 바로 실행을 시작해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 명령](lighter-structure.md#명령), [한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름)의 쓰기 확인 목록과 API 처리기, [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0146](0146-one-trss-process-and-container.md)(한 프로세스)

## 작업

[0146](0146-one-trss-process-and-container.md) 뒤에도 web 쪽은 명령을 적은 뒤 깨우기 소켓으로 같은 프로세스의 worker 쪽을 깨우고, worker 쪽은 3초마다 찾아보며 worker 잠금 아래에서 명령을 집어요. 이 티켓은 이 전달을 없애고, 적은 곳에서 같은 프로세스의 실행을 바로 시작해요. 명령은 지금처럼 `commands` 테이블에 기록으로 남아요(사용자 결정, 2026-10-21).

web이 worker를 깨우는 곳은 명령만이 아니에요. 2026-10-21에 깨우기(`wake_worker`)를 보내는 곳은 16곳이었어요. 명령 접수, 규칙의 수집 폴더와 회차 대응의 저장, 자막 작업을 만들거나 진행시키는 동작(배치, 교체, 올리기, 끝내기 등), 보관본의 적용과 정리, 원격 화면의 준비가 보내요. 이 곳들이 모두 실행을 바로 시작해요.

- 명령 안에서 다시 시도하는 대기와 횟수는 지금과 같아요. 기능 안의 규칙을 그대로 두기로 한 결정을 따른 거예요.
- 대상이 겹치는 명령의 차례 규칙과 토렌트를 함께 건드리는 일의 차례는 그대로예요.
- worker 잠금과 하트비트는 이 티켓 뒤에도 남고, [0148](0148-one-instance-lock-and-cycle-stall.md)이 없애요. 실행이 그 잠금을 함께 잡는 방식은 지금 그대로 써도 돼요.

## 완료 기준

- [x] API는 명령을 `commands` 테이블에 적은 뒤 같은 프로세스에서 곧바로 실행을 시작하고, 끝나기를 기다리지 않고 적은 명령으로 응답해요. 실행 중인 수집 주기가 있어도 명령이 그 주기가 끝나기를 기다리지 않는 것을 프로세스 테스트가 확인해요.
- [x] 대상이 겹치는 명령만 [웹 명령과 상태 갱신](../../../requirements/web-app.md#웹-명령과-상태-갱신)의 차례 규칙대로 기다리고, 관계없는 명령은 기다리지 않아요.
- [x] 명령 말고 깨우기를 보내던 곳도 모두 같은 프로세스의 실행을 바로 시작해요. 결과에 그 곳들과 각각 시작하는 실행을 적어요.
- [x] 같은 ID와 같은 내용으로 다시 보낸 명령은 적은 명령을 돌려주고, 같은 ID에 다른 내용이면 409로 거절해요. 같은 종류와 대상으로 열린 명령은 하나뿐이에요.
- [x] 명령 안에서 다시 시도하는 경우(Transmission이 새 폴더를 늦게 보고할 때, 토렌트 추가에 응답이 없을 때, DB 오류)의 대기와 횟수가 지금과 같아요. 실행을 시작할 때마다 시도 횟수가 하나 늘고, 다섯 번째 시작에서 이유와 함께 끝나요.
- [x] 다음 확인을 기다리던 경우는 실행 안에서 기다려요. 결과에 그런 경우를 모두 적어요.
- [x] 동시에 실행하는 명령은 8개까지예요.
- [x] 프로세스가 다시 시작하면 끝나지 않은 명령을 다시 실행하고, 이것도 시도 한 번으로 세요. 열린 명령이 남은 DB로 시작하는 테스트가 업그레이드 직전에 남은 명령도 같은 길로 실행되는 것을 확인해요.
- [x] 프로세스가 다시 시작하면, 사람을 기다리지 않고 실행 차례를 기다리던 자막 작업도 지금처럼 다시 시작해요.
- [x] 깨우기 소켓과 3초 주기 조회가 코드에서 없어지고, 시작 처리가 남은 소켓 파일을 지워요. worker 잠금 아래의 claim은 잠금과 함께 [0148](0148-one-instance-lock-and-cycle-stall.md)이 없애요(2026-10-21, 아래 결과).
- [x] 시작할 때 쓸 수 있는지 확인하는 목록에서 깨우기 소켓이 빠지고, 나머지 확인은 그대로예요.
- [x] 명령의 지난 결과를 읽는 곳은 그대로 `commands` 테이블을 읽어요. 규칙의 마지막 폴더 이동과 폴더 편집 막기, 회차 되돌리기, Anissia 새로고침의 결과, 수집 주기가 세는 응답 없던 토렌트 추가예요.
- [x] 보관 파일 정리는 요청하면 곧바로 실행해요. 프로세스가 다시 시작하면 `asked`인 정리를 다시 실행하고, 파일마다 길이와 SHA-256을 확인하는 지금 규칙은 그대로예요.
- [x] API 처리기는 미디어 파일을 직접 바꾸지 않아요. 파일을 바꾸는 일은 명령, 자막 작업, 보관 파일 정리의 실행 경로에서만 하고, 그 경로가 쓰기 직전에 계획·승인·현재 파일을 확인하는 지금 규칙은 그대로예요. 결과에 확인한 방법을 적어요.
- [x] 같은 커밋에서 다음 문서를 고쳐요.
  - [웹 앱 공통 명세의 웹 명령과 상태 갱신](../../../requirements/web-app.md#웹-명령과-상태-갱신)과 [구현 경계와 실행 순서](../../../requirements/web-app.md#구현-경계와-실행-순서)
  - [라이브러리와 작품 명세의 보관 파일의 정리](../../../requirements/library.md#보관-파일의-정리)
  - [readme](../../../../readme.md)가 앱 데이터 폴더의 파일과 시작할 때의 쓰기 확인으로 적은 깨우기 소켓
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요. 지운 테스트는 없어진 인계만 확인하던 것이어야 하고, 결과에 그 까닭과 짝지어 적어요.
- [x] 구현이 [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.

## 결과

2026-10-21에 깨우기 소켓을 같은 프로세스 안의 알림(`trss::wake::Wakes`)으로 바꾸고, 명령 흐름의 3초 주기 조회를 없앴어요. 명령을 적는 곳은 모두 명령 흐름을 깨우고, 흐름은 곧바로 명령을 claim해 실행을 시작해요. API는 지금처럼 적은 명령으로 곧바로 응답해요.

- 명령 흐름은 프로세스가 시작할 때, 깨워질 때, 명령 하나가 끝날 때, 나중으로 미룬 명령의 3초가 찼을 때만 명령을 찾아요. 정해진 주기로 찾는 일은 없어요.
- claim은 지금처럼 worker 잠금 아래에서 해요. 완료 기준은 claim도 잠금 없이 하라고 했지만, 그러면 [0148](0148-one-instance-lock-and-cycle-stall.md)의 단일 실행 잠금이 들어오기 전까지는 같은 앱 데이터를 쓰는 두 프로세스가 한 명령을 두 번 실행할 수 있어요. 이 티켓의 커밋만으로도 [웹 앱 공통 명세](../../../requirements/web-app.md#구현-경계와-실행-순서)의 "worker를 중복 기동해도 같은 작업이 중복 수행되지 않아야" 하는 요구를 지키려고, 이 항목을 worker 잠금을 없애는 0148로 옮겼어요(구현 결정, 2026-10-21). 한 프로세스 안에서는 잠금을 함께 잡으므로, claim이 수집 주기나 다른 명령을 기다리지 않아요.
- 시작할 때 0146까지의 릴리스가 남긴 `trss.db.wake` 소켓을 지우고 그 일을 로그에 남겨요. 시작할 때의 쓰기 확인 목록에서도 소켓을 뺐어요.

### 깨우던 곳과 시작하는 실행

0146까지는 web 쪽의 어느 곳이 깨워도 명령, 자막 작업, 원격 화면의 세 흐름을 모두 깨웠어요. 이제는 적은 것을 실행하는 흐름만 깨워요. 자막 작업 흐름과 원격 화면 흐름은 지금처럼 3초와 2초마다도 찾으므로, 깨우는 흐름이 틀려도 늦어질 뿐이에요. 2026-10-21에 web 쪽에서 깨우는 호출은 16곳이었어요.

| 곳 | 코드 | 깨우는 흐름 |
| --- | --- | --- |
| 명령 접수(`POST /api/commands`) | `commands_api.rs` | 명령 |
| 시즌을 연결한 뒤 Anissia 자막 줄 읽기 요청 | `commands_api.rs` `ask_anissia_captions` | 명령 |
| 멈춘 규칙의 `start`·`resume` | `rules_api.rs` `ask_start` | 명령 |
| 교체 결정 하나 | `jobs_api/replacement.rs` | 자막 작업 |
| 교체 결정 여럿 | `jobs_api/replacement.rs` | 자막 작업 |
| 배치 확인 | `jobs_api/placement.rs` | 자막 작업 |
| 고른 후보의 자막 작업 만들기 | `jobs_api.rs` | 자막 작업 |
| 직접 찾기 작업 만들기 | `jobs_api.rs` | 자막 작업 |
| 직접 찾기의 받기 끝내기 | `jobs_api.rs` | 원격 화면 |
| 구독 제작자의 작업 만들기(`follow_now`) | `jobs_api.rs` | 자막 작업 |
| 회차 대응을 저장해 다시 줄 선 작업 | `mapping_api.rs` | 자막 작업 |
| 보관본 적용 | `library_work_api.rs` | 자막 작업 |
| 보관본 정리 | `library_work_api.rs` | 자막 작업 |
| 자막 올리기 | `subtitle_upload_api.rs` | 자막 작업 |
| 원격 화면 준비 요청 | `screen_api.rs` | 원격 화면 |
| 원격 화면 새로 띄우기 | `screen_api.rs` | 원격 화면 |

worker 쪽이 자기 소켓으로 깨우던 곳 둘, 곧 수집 주기가 적은 `start` 명령과 시즌 연결이 적은 `anissia_captions` 명령도 명령 흐름을 깨워요. `start` 명령이 체크한 회차를 받으려고 적는 `receive_once` 명령은 따로 깨우지 않아도, 그 `start` 명령이 끝날 때 명령 흐름이 다시 찾아서 시작해요.

### 다음 확인을 기다리던 경우

0146까지 명령은 다음 경우에 worker의 다음 확인(3초 뒤)을 기다렸어요. 이제 앞의 넷은 명령 흐름이 명령마다 미룬 시각을 기억해 3초가 차면 그 명령을 다시 claim해 시작하고, 시도 횟수는 다시 시작할 때 하나 늘어요. 다섯째와 여섯째는 명령 흐름이 3초 뒤에 다시 찾아요.

1. 토렌트 추가에 Transmission이 답하지 않은 시작(`receive_once`, `receive_past`)
2. 명령 안에서 DB에 닿지 못한 시작
3. 명령의 끝을 DB에 적지 못한 경우
4. claim한 명령이 일할 폴더를 정하지 못한 경우(DB 오류)
5. 명령을 찾거나 claim하다 DB 오류가 난 경우
6. 다른 프로세스가 worker 잠금을 쥐고 있어 claim하지 못한 경우

깨우기 소켓의 신호를 놓친 경우(받는 쪽이 없거나 소켓이 찼을 때)도 다음 확인을 기다렸는데, 같은 프로세스 안의 알림은 놓치지 않아서 이 경우는 없어졌어요. 다시 시도하는 대기와 횟수는 그대로예요. Transmission이 새 폴더를 늦게 보고할 때의 대기는 `rule_archive` 실행 안의 것이라 바뀌지 않았어요.

### 남은 주기 조회

자막 작업 흐름의 3초 조회와 감시 폴더 목록의 3초 조회는 남겼어요. 깨우지 않아도 기다림이 끝나는 작업(영상이 들어옴, 압축을 다시 풀 때가 됨)과 web이 등록한 감시 폴더를 찾기 때문이에요. 둘이 쓰던 `command_poll`은 `poll`(`Worker::with_poll`)로 이름을 바꿨고, 명령이 다시 시작하기까지의 대기는 `with_command_retry`로 따로 뒀어요.

### API 처리기의 미디어 쓰기

2026-10-21에 web의 API 처리기가 미디어 파일, 곧 영상, 자막, 감시 폴더와 수집 폴더의 파일, 보관본 폴더를 바꾸는 길이 없는 것을 확인했어요. 다른 크레이트까지 포함해 테스트가 아닌 코드에서 파일을 만들거나 바꾸거나 지우는 호출(`std::fs`와 `tokio::fs`의 쓰기·이름 바꾸기·지우기·만들기·복사, `File::create`, `OpenOptions`, `rustix`의 `renameat`·`linkat`·`unlinkat`, `set_permissions`, `set_len`, `symlink`) 118곳을 찾고, `crates/trss/src/web`의 테스트가 아닌 코드가 이름을 부르는 다른 크레이트 항목 146개와 쓰기 함수마다의 호출자를 따라가 봤어요. API 처리기에서 닿는 쓰기는 앱 데이터 폴더 안의 두 곳뿐이에요.

- 자막 올리기가 올린 파일을 수신 영역(`receive/`)에 두는 것
- 표지를 올리거나 고르거나 바꿀 때 `artwork/`에 표지 파일을 두고, 어느 선택도 쓰지 않는 표지 파일을 지우는 것

보관본 정리 요청(`clean_stored`)은 정리 행을 DB에 적고 폴더가 있는지 읽기만 하며, 파일은 worker 쪽의 정리 실행이 지워요. 자막 작업의 배치와 대체(`trss-jobs`의 `Placer`, `Runner`)와 명령의 실행(`trss-collect`의 명령 실행 코드)은 web이 부르지 않아요. 이 확인은 컴파일러의 호출 그래프가 아니라 위 호출들을 찾아 읽은 것이라, 다른 방법으로 파일을 쓰는 코드는 놓칠 수 있어요. 원격 화면이 사람의 입력을 서버 브라우저로 넘겨 브라우저가 파일을 내려받는 길은 따라가지 않았어요. 받은 파일은 브라우저의 내려받기 폴더에 생기고 worker 쪽이 옮겨요.

### 테스트

2026-10-21에 이 티켓의 변경으로 `cargo test --locked --workspace -j 4`를 돌렸더니 통과 3,088개, 실패 0개, 건너뛴 것 14개였어요(0146에서는 통과 3,086개). 테스트 바이너리 시간의 합은 43.58초로, [0144의 기준값](0144-lighter-structure-baseline.md) 43.0초와 비슷했어요. `cargo clippy --workspace --all-targets -j 4`에 경고가 없어요. 아래 리뷰에서 고친 뒤 다시 돌려도 통과 3,088개, 실패 0개, 건너뛴 것 14개였고, 합은 43.77초였어요. 원본 로그는 이 PC의 `dev/local/measure/0147/`에 있어요.

- 지운 테스트: `trss-core` `wake.rs`의 `the_wake_socket_sits_next_to_the_database`, `a_listening_worker_hears_the_wake_and_a_missing_one_is_no_error`, `a_wake_with_a_path_reaches_the_listener_and_one_with_none_sends_nothing`. 깨우기 소켓 자체만 확인하던 테스트이고, 소켓이 없어졌어요.
- 이름을 바꾼 테스트: `trss-core` `access.rs`의 `the_checked_suffixes_hold_every_lock_file_and_the_wake_socket_once`를 `the_checked_suffixes_hold_every_lock_file_once`로 바꿨어요.
- 깨우기를 알림으로 바꾼 테스트: `trss` `tests/it/receive_once.rs`의 `the_running_worker_picks_up_a_command_without_waiting_for_the_next_cycle`과 `a_command_the_web_accepts_starts_within_a_second_without_waiting_for_a_look`, `web/screen_api/tests.rs`의 `woken`, `web/jobs_api/replacement/tests.rs`의 `the_worker_is_woken_only_when_a_decision_was_written`. 앞의 둘은 이제 주기 조회 없이 깨우기만으로 명령이 시작하는 것을 확인해요.
- 더한 테스트(`trss` `tests/it/receive_once.rs`):
  - `the_process_runs_a_command_to_its_end_while_its_cycle_is_held_in_transmission`: `trss` 프로세스의 첫 수집 주기를 Transmission에서 붙잡아 둔 채, 프로세스의 API로 보낸 명령이 끝나요.
  - `a_started_process_runs_again_the_commands_an_earlier_one_left_open`: 앞선 프로세스가 시작했다 멈춘 명령과 접수만 한 명령이 남은 DB로 `trss`를 띄우면 둘 다 끝나고, 시도 횟수는 2와 1이에요.
  - `a_command_left_for_later_starts_again_by_itself_once_its_wait_is_over`: 토렌트 추가에 답이 없어 미룬 명령을, 깨우기도 주기 조회도 없이 명령 흐름이 대기 뒤에 다시 시작해요.
  - `a_look_that_finds_the_lock_taken_looks_again_by_itself`: 다른 프로세스가 잠금을 쥐어 claim하지 못한 명령을, 잠금이 풀린 뒤 명령 흐름이 스스로 다시 찾아 시작해요.
  - `at_most_eight_commands_run_at_once_and_the_next_starts_when_one_ends`: 아홉 명령 중 여덟이 Transmission에서 기다리는 동안 아홉째는 `pending`이고, 하나가 끝나면 시작해요.
- `web/library_work_api/tests.rs`의 `a_work_shows_its_stored_files_and_one_is_cleaned_on_request`에, 받아들인 정리 요청이 자막 작업 흐름을 깨우는지 확인하는 줄을 더했어요.

그대로 통과한 테스트가 나머지 기준을 확인해요. 대상이 겹치는 명령의 차례는 `receive_once.rs`의 `a_retry_and_the_cycles_add_of_the_same_item_go_in_turn`과 `archive_move.rs`의 `two_starts_for_one_work_folder_run_one_after_the_other_and_the_second_just_turns_its_rule_on`이, 관계없는 명령이 서로 기다리지 않는 것은 위의 동시 실행 테스트가 확인해요. 같은 ID로 다시 보낸 명령과 409는 `web/commands_api/tests.rs`의 `the_same_command_delivered_twice_is_stored_once_and_answered_alike`, `the_same_id_for_another_item_is_refused`, `a_second_command_for_an_item_still_being_added_is_refused`가, 다시 시도의 횟수는 `receive_once.rs`의 `a_command_whose_adds_never_get_an_answer_ends_add_failed_and_holds_the_next_cleanup`이, 다시 시작한 뒤의 자막 작업과 정리는 `trss-jobs`의 `runner.rs` `a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_ends_it`과 `cleanup.rs` `a_worker_killed_after_the_confirmation_takes_the_cleanup_up`이 확인해요. 자막 작업 흐름이 시작할 때 처음 찾는 코드는 바꾸지 않았어요.

### 개발 환경에서 확인한 것

2026-10-21에 이 티켓의 변경으로 `dev/compose.sh up`을 실행해, 0146의 이미지가 남긴 `trss.db.wake`가 있는 앱 데이터로 `trss`를 띄웠어요.

- 시작 로그에 `trss: removed the wake socket of an earlier release (/data/trss.db.wake)`가 나오고, 앱 데이터 폴더에서 소켓이 없어졌어요.
- `POST /api/commands`로 `watch_rescan` 명령을 세 번 보냈어요. 셋 다 `202`로 `pending` 명령을 돌려받았고, 명령의 `created_at`에서 `finished_at`까지 13–14 ms였어요. 수집 주기가 300초마다 도는 환경이라, 명령이 주기나 3초 조회를 기다리지 않은 거예요.

### ADR과의 관계

[한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)은 결정의 결과로 깨우기 소켓, 3초 주기 조회, worker 잠금 아래의 claim, heartbeat가 없어진다고 적어요. 이 티켓이 앞의 둘을 없애고, claim은 heartbeat와 함께 0148이 없애요. 끝난 모습은 ADR과 같아서 ADR은 고치지 않았어요.

### 리뷰와 고친 것

`stora:complex-reviewer`가 커밋 전의 변경을 리뷰했어요. 깨우기를 놓치는 엇갈림, 열린 명령을 다시 찾을 일이 없어지는 경우, 바쁜 반복, 동시 실행 8개, 멈출 때의 정리, 깨우는 곳마다의 흐름, 새 테스트가 이름대로의 동작에 닿는지를 봤고, 막는 문제는 찾지 않았어요. 리뷰어가 미룬 명령의 타이머와 잠금 경합 뒤의 재시도를 빼면 두 재시도 테스트가, 상한을 9로 바꾸면 동시 실행 테스트가, 깨우기를 받는 분기를 빼면 깨우기 테스트 셋이 실패했어요. 새 테스트 다섯 개는 CPU를 모두 쓰는 동안 16번 돌려 한 번도 실패하지 않았어요. 다음을 고쳤어요.

- 미룬 명령이 다시 시작하는 테스트는 붙잡은 토렌트 추가의 답을 다음 시작 전에 풀어야 하는데, 재시도 대기가 500 ms라 테스트가 그만큼 늦어지면 두 번째 추가도 붙잡혀 시도 횟수가 3이 될 수 있었어요. 대기를 2초로 늘렸어요.
- `trss-library`와 `trss-jobs`의 주석 세 곳이 없어진 "command poll"을 적고 있어서, 자막 작업 흐름과 감시 폴더의 조회로 고쳤어요. 명령 흐름의 모듈 문서에는 실행 중인 명령이 적는 명령(`start`가 묻는 받기, `receive_once`가 묻는 `start`)은 깨우지 않고 그 명령이 끝난 뒤의 찾기가 집는다는 것과, 아래 한계를 더했어요.

### 0148까지의 한계

같은 앱 데이터로 프로세스 둘이 도는 동안, 한 프로세스가 명령을 시작한 채 멈추면 다른 프로세스는 그 명령을 곧바로 다시 집지 않아요. 깨워지거나, 다른 명령이 끝나거나, 재시도하거나, 다시 시작할 때 집어요. 0146까지는 3초 조회가 집었어요. 지원하지 않는 구성이고, [0148](0148-one-instance-lock-and-cycle-stall.md)이 두 번째 프로세스가 시작하지 못하게 막아서 없어지므로, 따로 조회를 남기지 않았어요(구현 결정, 2026-10-21). 리뷰어가 코드로 따진 것이고 재현하지는 않았어요.

### 문서

[웹 앱 공통 명세의 웹 명령과 상태 갱신](../../../requirements/web-app.md#웹-명령과-상태-갱신)과 검증 표, [라이브러리와 작품 명세의 보관 파일의 정리](../../../requirements/library.md#보관-파일의-정리)와 검증 표, readme의 앱 데이터 폴더 파일과 시작할 때의 쓰기 확인을 고쳤어요. 같은 이름으로 찾아, 작업 명세의 화면 준비 문장, 수집 명세의 다시 시작할 때의 시도 문장, `docker-compose.trss.yml`의 깨우기 소켓 주석도 고쳤어요. [구현 경계와 실행 순서](../../../requirements/web-app.md#구현-경계와-실행-순서)에서는 시작할 때의 쓰기 확인에서 소켓을 뺐어요. 같은 절의 "worker를 중복 기동해도 같은 작업·정기 처리가 중복 수행되지 않아야" 한다는 문장은 claim이 worker 잠금 아래에 남아 지금도 맞아서, 잠금을 바꾸는 0148이 고쳐요. "worker를 깨워요"라고 적은 다른 명세 문장은 같은 프로세스의 worker 쪽 흐름을 깨우는 것이라 지금도 맞아서 그대로 뒀어요.

### 확인하지 않은 것

- 서버(j4105)에 올려 확인하지 않았어요. [구조 가볍게 하기 명세](lighter-structure.md)는 이 변경을 모두 마친 뒤 사용자 승인을 받아 서버에 올려 확인하기로 했어요.
- 개발 환경에서는 명령만 보내 봤고, 자막 작업과 원격 화면을 깨우는 곳은 테스트로만 확인했어요. 둘은 깨우지 않아도 3초와 2초마다 찾아요.
- 원격 화면의 입력으로 서버 브라우저가 파일을 내려받는 길은 따라가지 않았어요(위 API 처리기의 미디어 쓰기).
