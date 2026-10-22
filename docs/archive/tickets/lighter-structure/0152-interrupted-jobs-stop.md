# 0152 실행 도중 끊긴 자막 작업을 `중단됨`으로 멈추고 사람이 다시 시작하게 해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 끊긴 자막 작업](lighter-structure.md#끊긴-자막-작업), [끊긴 작업 ADR](../../../adr/0020-interrupted-jobs-wait-for-a-restart.md)
- 막는 티켓: [0145](0145-merge-web-and-worker-crates.md)(크레이트 합치기)

## 작업

지금은 worker가 다시 시작하면 효과별 기록과 디스크를 대조해, 실행 도중이던 자막 작업을 이어 해요. [끊긴 작업 ADR](../../../adr/0020-interrupted-jobs-wait-for-a-restart.md)에 따라, 이제는 받기·보관·적용·교체·재배치 단계를 실행하던 중에 끊긴 작업이 `중단됨`이 되고, 사람이 `다시 시작`을 눌러 끊긴 단계부터 다시 해요.

이 티켓은 상태와 화면, 다시 시작하는 경로를 만들어요. `다시 시작`이 끊긴 단계를 다시 하는 데는 지금의 이어 하기 코드를 그대로 써요. 효과별 기록을 없애고, 기록 없이 단계를 다시 해도 안전하게 만드는 일은 [0153](0153-no-effect-records.md)이 해요. 이렇게 나누면 사용자가 보는 동작은 이 티켓에서 명세대로 바뀌고, 0153은 안쪽만 바꿔요.

- `중단됨`인 작업은 할 일로 올리지 않고, 할 일 화면의 `자막 작업` 맨 위에 실패한 작업과 함께 보여요(사용자 결정, 2026-10-21).
- 작업을 다시 시작하는 API 경로를 더해요. 명세가 더하기로 한 경로예요.

## 완료 기준

- [x] 받기·보관·적용·교체·재배치 단계를 실행하던 중에 프로세스가 멈춘 작업은, 다시 시작한 뒤 `중단됨`이 돼요. 단계마다 실행 도중의 작업이 남은 DB로 시작하는 테스트가 확인해요.
- [x] 인증·배치 확인·교체 승인·영상 대기·자막 대기처럼 사람을 기다리던 작업은 다시 시작한 뒤에도 그대로 기다려요.
- [x] `중단됨`인 작업은 할 일로 올라가지 않고, 할 일 화면의 `자막 작업` 맨 위에 실패한 작업과 함께 보여요.
- [x] `중단됨`은 `held`와 다른 상태예요. 할 일 개수, 작업 목록과 상세, 라이브러리 배지, 작품 상세의 적용 상태, 영상 대기 판정은 이 상태를 끝나지 않은 실패로 다뤄요. 각 판정을 그 규칙을 가진 크레이트의 테스트가 확인해요.
- [x] `중단됨`인 작업의 목록 행과 작업 상세에 `다시 시작` 버튼이 있고, 누르면 끊긴 단계부터 다시 해요. 수신 영역에 이미 받아 둔 묶음은 다시 받지 않아요.
- [x] API 테스트가 다시 시작하는 경로의 요청과 응답 모양, `중단됨`이 아닌 작업을 거절하는 응답을 확인해요.
- [x] 같은 커밋에서 [작업과 인증 명세](../../../specs/jobs.md)의 [작업 결과](../../../specs/jobs.md#작업-결과)와 [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)에서 다시 시작할 때의 동작을 고쳐요. 할 일 화면과 라이브러리 배지를 적은 명세가 작업 상태를 나열하면 그곳도 고쳐요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요. 다시 시작할 때 자동으로 이어 하는 것을 확인하던 테스트는 `중단됨`이 되는 것을 확인하게 고치고, 결과에 적어요.
- [x] 구현이 [끊긴 작업 ADR](../../../adr/0020-interrupted-jobs-wait-for-a-restart.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.

## 결과

2026-10-21에 실행 도중 끊긴 자막 작업이 `중단됨`으로 멈추고, 사람이 `다시 시작`을 눌러야 끊긴 단계부터 이어 가게 했어요. 다시 시작한 작업이 단계를 이어 하는 데는 지금의 이어 하기 코드, 곧 기록과 디스크의 대조를 그대로 써요. ADR과 다르게 한 것은 없어요.

### 상태와 바꾸는 때

- 새 상태 `interrupted`를 화면에서 `중단됨`이라고 불러요. 마이그레이션 65(`jobs/interrupted.sql`)가 `subtitle_jobs`를 다시 만들어 상태의 CHECK에 더해요. 열 순서와 행, `AUTOINCREMENT`가 다다른 수, 인덱스는 그대로예요.
- `trss`는 DB를 연 바로 뒤, 웹을 열고 작업을 집기 전에 `running`인 작업을 모두 `interrupted`로 바꿔요. 작업은 단계(`stage`)와 시작 횟수를 그대로 간직하고, 기록에 `중단됐어요`와 `<단계> 단계에서 멈췄어요`(단계 전이면 `단계를 시작하기 전에 멈췄어요`)가 남아요. 바꾸지 못하면 시작을 멈춰서, 작업 루프가 그 작업을 자동으로 이어 하지 않아요.
- 단계를 실행하던 중에 SIGTERM으로 멈춘 작업도 `running`으로 남아, 다음 시작에서 `중단됨`이 돼요. ADR이 말하는 끊긴 작업과 같게 다뤄요(`trss-jobs` `tests/it/runner.rs`의 이름을 바꾼 테스트).
- 처음에는 이 변환을 worker 부분의 시작(`Worker::run`)에서 했는데, 리뷰에서 그 전에 웹이 이미 요청을 받는다는 점이 나왔어요. 그 사이에 사람이 누른 `다시 시작`이 변환에 되돌려지고, 변환이 실패해도 작업 루프가 그 작업을 이어 할 수 있었어요. 그래서 변환을 `main`의 DB를 연 자리로 옮겼어요.
- `POST /api/subtitle-jobs/{id}/restart`는 `중단됨`인 작업만 단계를 간직한 채 `running`으로 돌리고, 기록에 `다시 시작했어요`와 `<단계> 단계부터`를 남긴 뒤 작업 루프를 깨워요. 응답은 200 `{"state":"running"}`이고, `중단됨`이 아닌 작업은 409, 없는 작업은 404예요. 루프는 예전에 `running`으로 남은 작업과 같은 길(`claim_next`가 이어 하기로 집음)로 끊긴 단계를 다시 해요.
- 직접 찾기 작업의 받기 끝내기는 `중단됨`인 동안 409로 거절해요. 다시 시작한 뒤에 끝내게 했어요.
- 회차 대응 변경이 `중단됨`인 작업의 줄을 옮기면 실행 중인 작업처럼 `remapped_at`을 남겨, 다시 시작한 실행이 끝날 때 작업을 다시 줄에 세워요. 이것도 리뷰에서 나온 것이에요.

### `중단됨`을 읽는 판정

| 판정 | 다룸 | 확인한 테스트 |
| --- | --- | --- |
| 작업 목록의 묶음 | `실패` 묶음에서 새로 바뀐 순서로, 묶음 이름은 `실패` 그대로 | `trss` `web/jobs_api/tests.rs` `restarting_an_interrupted_job_runs_it_again_and_any_other_job_is_refused` |
| 할 일과 개수, 라이브러리 배지 | 올리지 않고 더하지 않음 | `trss-jobs` `tests/it/todo.rs` `an_interrupted_job_is_no_todo_and_leaves_the_count_as_it_was` |
| 작업이 끝났는지 | 끝나지 않음 | `trss-jobs` `tests/it/interrupted.rs` `an_interrupted_job_is_a_failure_to_show_and_an_unfinished_one_for_the_rest` |
| 같은 회차의 새 작업 | 회차를 차지해 막음 | `trss-jobs` `tests/it/follow.rs` `another_creators_interrupted_job_holds_the_episode_like_one_in_line` |
| 영상 대기 | 다시 시작하기 전에는 영상을 기다리지 않음 | `trss-jobs` `tests/it/place.rs` `a_job_cut_short_does_not_wait_for_a_video_until_it_is_restarted` |
| 보관본 정리 | 정리할 수 없음(`INTERRUPTED_JOB`) | `trss-jobs` `tests/it/cleanup.rs` `a_stored_subtitle_a_running_held_or_interrupted_job_uses_is_not_cleanable` |
| 보관본 고르기 | 고를 수 없고 작업 상세에서 다시 시작하라고 알림 | `trss-jobs` `tests/it/choose.rs` `the_works_copies_come_by_episode_and_newest_first_and_say_why_one_cannot_be_chosen` |
| 배치 확인 | 다시 시작하기 전에는 받지 않음 | `trss-jobs` `tests/it/placement.rs` `a_job_cut_short_does_not_take_a_placement_until_it_is_restarted` |
| 작품 상세의 적용 상태 | 실패로 끝난 기다림, 작업 화면을 가리킴 | `web` `src/screens/library/detail/subtitles.test.ts` `an interrupted job ends the wait as a failure that points at the job's screen` |
| 작업 루프 | 집지 않음 | `trss-jobs` `tests/it/interrupted.rs` `a_job_left_running_in_any_step_is_interrupted_there_and_one_that_waits_is_not` |

### 화면

할 일 화면의 `자막 작업`에서 `중단됨`인 작업은 `실패` 묶음에 `중단됨` 배지(빨간색, 일시 정지 아이콘)로 보이고, 줄을 펼치지 않아도 `다시 시작`이 있어요. 작업 상세는 머리 아래에 `<단계> 단계를 실행하던 중에 멈췄어요. 다시 시작하면 끊긴 단계부터 이어서 해요.`와 `다시 시작`을 두고, 단계 표시에서 끊긴 단계를 `진행 중`이 아니라 `중단`으로 보여요. 단계 표시는 리뷰에서 고친 것이에요. 누른 뒤 409를 받으면 거절 문장을 보이고 작업을 다시 읽어요.

### 바뀐 테스트

다시 시작할 때 자동으로 이어 하는 것을 확인하던 테스트 14곳(`runner.rs` 5, `replace.rs` 3, `drive_fonts.rs` 2, `place.rs` 2, `unpack.rs` 1, `winpng.rs` 1)은 이어 가기 전에 `cut_and_restart`를 불러요. 이 도우미는 시작이 작업을 `중단됨`으로 바꾸고 단계를 간직하는지, 다시 시작이 그 단계 그대로 `running`으로 돌리는지 확인해요. 그 뒤의 단언은 바꾸지 않았어요. 이름을 바꾼 테스트는 둘이에요.

- `runner.rs` `a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_ends_it` → `…_and_the_next_start_interrupts_it_until_a_restart_ends_it`
- `cleanup.rs` `a_stored_subtitle_a_running_or_held_job_uses_is_not_cleanable` → `a_stored_subtitle_a_running_held_or_interrupted_job_uses_is_not_cleanable`

더한 테스트는 위 표의 것 말고 이래요.

- `trss-core` `db.rs` `the_interrupted_migration_keeps_the_jobs_their_children_and_their_indexes`
- `trss-jobs` `tests/it/interrupted.rs`: 단계마다 끊긴 작업과 기다리던 작업, 다시 시작이 `중단됨`만 받는 것, 수신 영역에 받아 둔 묶음을 다시 받지 않는 것, 직접 찾기 작업의 끝내기
- `trss-jobs` `tests/it/relocate.rs` `an_interrupted_job_whose_rows_a_mapping_change_moved_goes_back_in_line_after_its_restarted_run`: 고치기 전 코드에서 실패하는 것을 봤어요.
- `trss` `tests/it/worker_process.rs` `a_job_left_running_is_interrupted_before_the_web_serves`: 실제 `trss`를 띄워, `trss listening`이 찍힐 때 작업이 이미 `interrupted`이고 변환 줄이 그보다 먼저 찍히는지 봐요.
- `trss` `web/jobs_api/tests.rs` `finishing_a_find_job_the_process_cut_short_is_refused_until_it_is_restarted`
- `web` `src/screens/todo/interrupted.test.ts`: 다시 시작할 수 있는 상태, 끊긴 단계 문장, 단계 표시의 `중단`

2026-10-21에 리뷰에서 나온 것을 고친 코드로 `cargo test --locked --workspace -j 4`를 돌렸더니 통과 3,068개, 실패 0개, 건너뛴 것 10개였어요. [0150](0150-trss-crate-tests-by-adr-0015.md) 뒤의 통과 3,054개보다 14개 많아요. `cargo clippy -p trss -p trss-jobs -p trss-core --all-targets`에 경고가 없고, 웹의 `npm test`(242개)와 `npm run typecheck`도 통과했어요. 원본 로그는 이 PC의 `dev/local/measure/0152/`에 있어요.

### 개발 환경에서 확인

2026-10-21에 이 PC의 개발 환경(서버 DB 사본, 가짜 자막 출처)에서 봤어요. 먼저 `trss`를 멈추고 영상을 기다리던 작업 하나(`55b6982b`, FX 전사 쿠루미 3화)를 SQL로 적용 단계의 `running`으로 바꾸고, 그 단계 줄도 `current`로 바꿨어요. 실제로 단계 도중에 프로세스를 죽인 것은 아니에요.

- 리뷰 전 이미지: `trss`를 띄우자 `Subtitle jobs: 1 left running by an earlier process are interrupted`가 찍히고, 작업이 단계 `apply`를 간직한 `interrupted`가 됐어요. 기록에는 `중단됐어요 · 적용 단계에서 멈췄어요`가 남았어요. 할 일 화면에서는 `자막 작업`의 `실패 1`에 `중단됨` 배지와 `다시 시작`이 보였어요. 줄의 `다시 시작`을 누르자 기록에 `다시 시작했어요 · 적용 단계부터`와 `멈췄던 작업을 이어가요`가 남고, 작업이 적용 단계를 다시 해서 영상이 없어 `영상 대기`로 돌아갔어요. 같은 준비로 작업 상세의 설명 문장과 `다시 시작`도 눌러 봤고 결과는 같았어요.
- 리뷰를 고친 이미지: 로그에서 변환 줄이 `trss listening`보다 먼저 찍혔어요. 작업 상세의 단계 표시는 `적용`을 경고 아이콘과 빨간 테두리의 `중단`으로 보였고, 375×812 휴대폰 흉내에서는 접힌 한 줄이 `적용 중단 … 5/5`였어요. `다시 시작`을 누르자 작업이 다시 `영상 대기`가 되고 단계 표시도 `대기`로 돌아왔어요.

머리 아래의 `영상이 아직 없어 영상이 들어오면 적용해요`는 SQL로 상태를 바꾸기 전의 사유가 남은 것이에요. 작업은 처음 상태(`waiting`, `video`)로 돌아왔고, 세 번 끊고 다시 시작한 기록 12줄이 더해졌어요.

### 리뷰

`stora:reviewer`가 마이그레이션, 상태를 읽는 곳, 다시 시작하는 경로와 화면을 봤어요. 마이그레이션 65는 개발 DB의 사본에 적용해 `foreign_key_check`와 `integrity_check`가 깨끗하고, 열 순서·행·`sqlite_sequence`·인덱스가 그대로인 것을 확인했어요. 나온 것 여섯 가지 가운데 다섯은 고쳤고, 리뷰어가 다시 읽어 고쳐진 것을 확인했어요.

- 웹이 먼저 열려 변환 전의 `다시 시작`이 되돌려짐, 변환이 실패해도 루프가 이어 함: 변환을 `main`으로 옮기고 실패하면 시작을 멈춰요.
- 단계 표시가 끊긴 단계를 `진행 중`으로 보임: `중단`으로 보여요.
- 회차 대응 변경이 `중단됨` 작업에 표시를 남기지 않음: 실행 중인 작업처럼 남겨요.
- 주석의 옛 이름과 줄 바꿈: 고쳤어요.

고치지 않은 하나는, 다시 시작한 작업이 먼저 줄에 선 `pending` 작업 뒤에서 기다리는 동안에도 `진행 중` 묶음에 보이는 점이에요. 예전에 `running`으로 남은 작업을 이어 하던 길을 그대로 써서 생기는 것이고, 명세가 정한 동작과 어긋나지 않아요.

### 확인하지 않은 것

- 단계를 실행하는 도중에 실제로 `trss`를 죽였다 띄운 적은 없어요(2026-10-21). 단계마다 끊긴 상태는 테스트와 개발 환경 모두 DB에 SQL로 만들었어요.
- `다시 시작`이 끊긴 단계를 다시 하는 동작은 기존의 기록과 디스크 대조 코드를 그대로 써서, 기존 테스트로만 확인했어요(2026-10-21). 그 대조를 없애는 일은 [0153](0153-no-effect-records.md)의 몫이에요.
- 409를 받았을 때의 거절 문장은 브라우저에서 보지 않았어요(2026-10-21).
