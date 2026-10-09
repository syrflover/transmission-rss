# 0110 자막 작업 줄기의 테스트를 나누고 작업·자막 명세에 검증 표를 둬요

- 상태: 완료
- 출처: [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성), [영역별 검증 표](refactoring.md#영역별-검증-표), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0105](0105-episode-mapping-in-library.md), [0106](0106-todo-aggregation-in-jobs.md), [0107](0107-split-job-store.md), [0108](0108-split-job-runner.md), [0109](0109-shared-aside-and-remove.md)

## 작업

자막 작업 줄기의 리팩터링을 마쳤으니 테스트를 나누고 정리해요.

- trss-web의 작업 API 테스트 중 배치와 교체 규칙을 다시 확인하는 것은 trss-jobs에 같은 경우가 있으면 지우고, 없으면 내려요. 웹에는 요청과 응답의 모양, 상태 코드, Host·Origin 검사만 남겨요.
- trss-jobs 테스트 파일 18개가 각자 가진 시계 도우미와 19개가 각자 가진 준비 함수를 하나로 모아요.
- 실제 시간을 기다리는 원격 화면과 찾기 테스트 14개(2026-10-07에 합 49초, 가장 긴 것 8.1초)는 주입한 시계나 멈춘 tokio 시간으로 기다려요. 확인하는 순서와 결과는 그대로예요.
- 교체와 재배치의 테스트는 0109에서 모은 구현의 테스트와 겹치는 것을 정리해요.

[작업과 인증 명세](../specs/jobs.md)와 [자막 명세](../specs/subtitles.md)에 요구별 검증 표를 둬요. 요구마다 실제 사용에서 관찰한 것(날짜와 티켓), 테스트로 확인한 것(테스트 이름이나 파일), 확인하지 않은 것을 나눠 적어요.

## 완료 기준

- 결과 절에 파일마다 앞뒤 테스트 수와, 지운 것·내린 것·남긴 것의 개수가 있어요. 지운 테스트마다 같은 경우를 확인하는 남은 테스트를 찾을 수 있어요.
- 원격 화면과 찾기 테스트의 앞뒤 시간이 결과 절에 있고, 그 테스트들이 실제 시간을 기다리지 않아요.
- workspace 테스트가 통과하고, trss-jobs와 trss-web 테스트 바이너리의 앞뒤 실행 시간이 결과 절에 있어요.
- 작업과 인증 명세와 자막 명세에 검증 표가 있어요.

## 결과

### 결정

- 검증 표는 [0101](0101-receive-line-tests.md)과 같이 명세의 항목(글머리표, 규칙을 적은 문단)과 입력·결과 표의 줄마다 한 줄로 쓰고, 영역별 표를 명세 끝의 `검증 표` 절에 둬요(사용자 결정, 2026-10-09).
- 원격 화면과 찾기 테스트는 멈춘 tokio 시간으로 기다려요. trss-jobs의 tokio 개발 의존성에 `test-util`을 더하고 테스트에 `start_paused`를 붙였어요. 주입한 시계(`Clock`)는 기록에 시각을 찍을 뿐이고 기다림은 제품의 `tokio::time::sleep`(`PAGE_POLL` 1초, `FINISH_POLL` 500ms)이 정하므로, 제품 코드를 바꾸지 않는 이 방식을 골랐어요(리드가 정함).
- 웹 코드에 결정이 적힌 규칙은 이 티켓에서 옮기지 않고 그 웹 테스트를 남겨요. [0101](0101-receive-line-tests.md)의 선례대로 [0111](0111-web-and-screen-rules.md)에 더했어요(리드가 정함).
- 할 일 카드의 규칙은 웹 라우터 대신 trss-jobs의 `todo::list`를 실제 DB 상태로 부르는 테스트에서 확인해요. 그 바탕(`TodoStores`)은 `tests/it/world.rs`에 두고, 여러 기능이 함께 쓰는 경우는 새 `tests/it/todo.rs`에 모았어요(리드가 정함).
- 교체와 재배치의 테스트 중 [0109](0109-shared-aside-and-remove.md)의 `place/aside.rs` 테스트와 겹치는 것은 지우지 않았어요. 아래 [교체와 재배치의 겹침](#교체와-재배치의-겹침)에 까닭이 있어요(리드가 정함).

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `test(jobs): wait the remote-screen and find polls in paused tokio time (0110)` | 원격 화면과 찾기 테스트 15개를 멈춘 tokio 시간으로 기다려요. 테스트 몸통은 그대로예요. |
| `test(jobs): share the clocks and the setup base of the integration tests (0110)` | 파일 18개의 시계 도우미와 파일 19개의 준비 함수를 `tests/it/world.rs`의 시계 네 가지(`ticking_clock`, `ticking_from`, `counting_clock`, `fixed_clock`)와 `Base`, `Shows`로 모았어요. |
| `test(jobs): move the to-do cards and the compare cases of the job line's web tests down (0110)` | 작업 목록과 배치 표 웹 테스트의 할 일 카드, `is_app_command`, 비교 엔진의 두 경우를 내렸어요. |
| `test(jobs): move the replacement to-do cases of the replacement web tests down (0110)` | 교체 웹 테스트의 할 일 카드 경우를 표 테스트 하나로 내렸어요. |
| `test(jobs): move the stored-copy rules of the work page's web tests down (0110)` | 작품 화면 웹 테스트의 보관본 정리와 고르기 규칙을 내렸어요. |
| `test(jobs): move the upload rules of the upload web tests down (0110)` | 자막 올리기 웹 테스트의 저장 규칙을 내렸어요. |
| `test(jobs): move the to-do, mapping and creator rules of their web tests down (0110)` | 할 일, 회차 대응, 제작자 웹 테스트의 규칙을 내렸어요. |
| `test(jobs): trim the screen requests of the screen web tests to the web's part (0110)` | 원격 화면 웹 테스트 5개를 웹의 몫으로 줄였어요. |
| `docs(jobs): add the verification tables to the jobs and subtitles specs (0110)`(이 결과를 담은 커밋) | [작업과 인증 명세의 검증 표](../specs/jobs.md#검증-표), [자막 명세의 검증 표](../specs/subtitles.md#검증-표)와 이 결과예요. |

제품 코드는 바꾸지 않았어요. 내린 테스트 중 아래 크레이트에서 실패한 것은 없었어요.

검증 표는 두 명세의 끝에 있어요.

| 명세 | 요구 절 | 행 | 실제 사용에서 본 것이 있는 행 | 테스트가 있는 행 | 둘 다 없는 행 |
| --- | ---: | ---: | ---: | ---: | ---: |
| [작업과 인증](../specs/jobs.md#검증-표) | 7 | 123 | 61 | 116 | 5 |
| [자막](../specs/subtitles.md#검증-표) | 18 | 240 | 158 | 226 | 6 |

- 둘 다 없는 행은 표 끝의 `확인하지 않은 요구`에 모았어요. 실제 사용에서 본 것은 날짜가 있는 관찰만 셌고, `근거만:`과 `참고(가짜 출처):`는 세지 않았어요.
- 처음 쓴 표는 작업 233 KB, 자막 248 KB로 명세 본문(144 KB, 139 KB)보다 컸어요. 그래서 테스트는 요구를 가장 직접 겨눈 것을 부분마다 하나만 남기고, 관찰은 행마다 둘까지로 줄이고, 확인하지 않은 것은 모두 남겨 107 KB와 126 KB로 줄였어요. 티켓 링크는 참조형으로 적고 그 정의를 명세 끝에 모았어요.
- 표를 쓴 agent가 이름만 보고 적은 테스트는 줄이는 단계에서 다시 읽었어요. 그 행을 확인하지 않는 테스트는 빼거나 행을 확인하는 다른 테스트로 바꿨어요. 실제 사용의 관찰은 인용한 티켓을 다시 읽어 확인했고, 티켓이 뒷받침하지 않는 관찰은 지웠어요.

### 파일별 테스트 수

테스트를 뺀 파일이에요. "앞"은 `eb5f495`, "뒤"는 `5d1fb3b`의 `#[test]`·`#[tokio::test]` 개수예요. "남김"에는 웹의 몫만 남기고 줄인 테스트도 들어가요.

| 파일 | 앞 | 뒤 | 지움 | 내림 | 남김 | 그중 줄임 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| trss-web `jobs_api/tests.rs` | 11 | 10 | 0 | 1 | 10 | 3 |
| trss-web `jobs_api/placement/tests.rs` | 5 | 5 | 0 | 0 | 5 | 5 |
| trss-web `jobs_api/replacement/tests.rs` | 22 | 15 | 0 | 7 | 15 | 9 |
| trss-web `library_work_api/tests.rs` | 22 | 19 | 2 | 1 | 19 | 5 |
| trss-web `subtitle_upload_api/tests.rs` | 25 | 20 | 1 | 4 | 20 | 8 |
| trss-web `todo_api/tests.rs` | 8 | 7 | 0 | 1 | 7 | 2 |
| trss-web `mapping_api/tests.rs` | 10 | 9 | 0 | 1 | 9 | 4 |
| trss-web `subtitle_creator_api/tests.rs` | 11 | 9 | 2 | 0 | 9 | 3 |
| trss-web `screen_api/tests.rs` | 30 | 30 | 0 | 0 | 30 | 4 |
| 아홉 파일 합계 | 144 | 124 | 5 | 15 | 124 | 43 |

- "내림" 15개는 테스트 전체를 내린 것이에요. 이 밖에 "줄임" 43개에서 뺀 확인 중 아래 크레이트에 같은 경우가 없던 것은 먼저 아래에 썼어요.
- 교체 웹 테스트 7개는 trss-jobs `tests/it/todo.rs`의 표 테스트 하나(경우 6가지)와 trss-subtitles 비교 테스트 2개로 내렸어요. 올리기의 "받을 것이 없음" 4가지와 같은 ID의 다른 내용 5가지도 각각 표 테스트 하나예요.
- 줄인 테스트는 상태 코드, 응답 모양, 웹이 정하는 것을 남겼어요. 원격 화면 테스트는 GET이 아무것도 쓰지 않는다는 확인(`prepare_requests`, `woken`, `input_at`)도 남겼어요. 아래 크레이트에서는 볼 수 없는 것이에요. 올리기 테스트 하나는 응답의 `path`를 확인하던 테스트가 내려가서 그 확인을 더했어요.

테스트를 받은 파일이에요. 기존 테스트에 확인을 더한 것은 세지 않았어요.

| 크레이트 | 파일 | 더함 |
| --- | --- | ---: |
| trss-jobs | `tests/it/todo.rs`(새 모듈) | 5 |
| trss-jobs | `tests/it/upload.rs` | 6 |
| trss-jobs | `tests/it/follow.rs` | 4 |
| trss-jobs | `tests/it/cleanup.rs` | 2 |
| trss-jobs | `tests/it/` 아래 `choose.rs` 1, `placement.rs` 1, `relocate.rs` 1, `screen.rs` 1 | 4 |
| trss-jobs | `src/lib.rs`(`is_app_command`) | 2 |
| trss-subtitles | `src/compare/tests.rs` | 2 |
| 합계 | | 25 |

- 기존 테스트에 더한 확인은 trss-jobs `tests/it/`의 `upload.rs` 2개, `follow.rs` 3개, `find.rs`, `replace.rs`, `choose.rs`, `cleanup.rs`, `placement.rs`, `relocate.rs`(거절 문장을 글자 그대로 확인), trss-library `creators_tests.rs` 1개에 있어요.
- 이 스무 파일의 테스트는 616개에서 621개가 되었어요. 웹에서 20개가 빠지고 아래에 25개가 늘었어요.

### 지운 테스트와 남은 테스트

| 지운 테스트 | 남은 테스트 | 같은 경우인 까닭 |
| --- | --- | --- |
| trss-web `library_work_api/tests.rs` `nothing_of_a_work_whose_folder_is_away_is_cleaned` | trss-jobs `tests/it/cleanup.rs` `nothing_is_cleaned_while_the_work_folder_is_away` | 작품 폴더가 없을 때 정리를 거절하고 그 까닭을 `FOLDER_AWAY`로 밝히는 같은 입력과 결과예요. 웹 테스트는 문장을 글자로 적었고, 남은 테스트는 같은 문장의 상수와 견줘요. |
| trss-web `library_work_api/tests.rs` `the_creators_other_format_is_added_beside_the_applied_copy_on_request` | trss-jobs `tests/it/choose.rs` `the_creators_other_format_is_added_beside_the_applied_copy_with_no_approval`, `an_added_format_whose_path_holds_a_file_is_planned_with_the_applied_copy_kept`, `an_add_is_for_the_creators_other_format_and_the_other_refusals_stand`, `a_copy_with_no_creator_is_never_added`, `another_creators_copy_on_an_episode_with_a_subtitle_is_compared_then_replaces_it` | 웹 테스트의 경우(더하기의 거절, 승인 없이 더하기, 파일이 있는 자리에 더하면 비교, 다른 제작자의 보관본 적용은 비교)가 다섯 테스트에 나뉘어 있어요. 경우마다 입력과 기대를 견줬어요. |
| trss-web `subtitle_upload_api/tests.rs` `a_text_file_sent_anyway_is_dropped_by_the_server_and_named_in_the_job` | trss-jobs `tests/it/upload.rs` `an_upload_is_a_received_job_that_waits_for_the_worker`, trss-subtitles `src/upload.rs` `what_is_neither_a_subtitle_nor_a_font_is_dropped_by_its_content`, trss-jobs `src/upload.rs` `an_arrival_is_kept_in_the_jobs_folder_under_a_name_of_its_own` | 텍스트 파일을 `NOT_THEM`으로 빼고 자막은 남겨요. 남은 trss-jobs 테스트는 뺀 개수만 확인했으므로, 지우기 전에 뺀 이름과 `NOT_THEM`을 결과와 `detail.dropped` 양쪽에 더했어요. |
| trss-web `subtitle_creator_api/tests.rs` `every_subtitle_file_is_by_an_unknown_creator_until_one_is_named` | trss-library `src/store/library/creators_tests.rs` `files_are_by_an_unknown_creator_until_the_user_names_one` | 모든 파일의 제작자가 없고 버전이 처음 값이에요. 파일 수(웹 12개, library 4개)만 달라요. |
| trss-web `subtitle_creator_api/tests.rs` `a_creator_named_for_the_files_survives_a_rescan_of_the_watch_folder` | trss-library `src/store/library/creators_tests.rs` `a_rescan_keeps_the_creator_of_a_file_it_finds_and_carries_it_when_the_episode_is_read_again` | 시즌 전체에 제작자를 정하고 같은 파일을 다시 훑으면 제작자와 버전이 그대로예요. |

### 명세와 코드가 다른 곳

표를 쓰며 명세의 현재형 문장을 기록과 코드에 견줬어요(2026-10-09).

- 자막 명세의 상태 문장 9곳이 그 뒤의 기록과 달랐어요. 명세를 기록에 맞췄어요.
  - 출처별 다운로드 표의 다섯 행이 "적용은 묶음 분석 이후"나 "사람이 풀어 받는 확인은 남았어요"라고 적었어요. 실제로는 여덟 경로가 2026-10-05–06에 개발 환경에서 적용본까지 갔고([0076](../archive/tickets/4-store-apply-and-replace/0076-eight-paths-end-to-end.md)), erulabo는 2026-10-07 실제 서버에서도 사람이 풀어 받았어요([0083](../archive/tickets/5-deployed-verification/0083-deployed-end-to-end.md)).
  - 수정본의 같은 바이트를 "교체할 것 없음"으로 적은 두 곳을 기록 이름 `바꿀 것이 없음`으로 맞췄어요. 그중 한 곳은 "교체 비교가 생기면 거기서 기록해요"라는 미래형이었고, [0121](../archive/tickets/5-deployed-verification/0121-identical-copy-becomes-applied.md) 뒤로는 같은 바이트의 파일을 적용본으로 기록하고 교체 승인을 만들지 않아요.
  - 작품 회차 줄의 보관본 버튼은 자막이 있는 회차에서 `교체 비교`예요([0120](../archive/tickets/5-deployed-verification/0120-stored-copy-on-subtitled-row.md)).
  - 실제 erulabo의 `dateModified`와 Drive `HEAD` 값은 "사람이 받은 뒤에야 볼 수 있어요"라고 적었지만, 2026-10-04에 다시 읽어 확인했어요([0050](../archive/tickets/3-subtitle-candidates-and-receiving/0050-silent-revision-recheck.md)).
- 작업과 인증 명세의 남은 구체화 4곳도 그 뒤의 기록과 달라 고쳤어요. 출처별 단계는 "초안"이 아니라 모두 구현됐고, 서버 브라우저 컨테이너에서도 받았어요. 원격 화면은 실제 서버에서도 LAN의 휴대폰으로 봤고, erulabo 확인 뒤의 정상 응답도 봤어요. 절 이름 `출처별 단계 초안`은 `출처별 단계`로 바꾸고 그 링크 셋을 고쳤어요.
- 작업 상세의 `회차별 결과`는 교체 승인을 기다리는 회차를 `교체 승인 대기`로 적어요. 다른 화면은 `교체 승인`이고, 명세는 한 상태에 한 이름을 쓰고 `교체 승인 대기`는 쓰지 않는다고 적어요. 사용자는 두 상태의 이름을 `교체 승인 대기`와 `회차 확인 필요`로 정했고(사용자 결정, 2026-10-09), 화면 문구를 바꾸는 일이라 [0131](0131-waiting-state-names.md)로 남겼어요.

### 원격 화면과 찾기 테스트의 시간

티켓의 14개는 2026-10-07 측정에서 센 수예요. 2026-10-09에 코드를 읽어 보니 제품의 `PAGE_POLL`(1초)을 기다리는 테스트가 10개, `FINISH_POLL`(500ms)을 기다리는 테스트가 5개여서 15개를 멈춘 시간으로 바꿨어요. 다른 원격 화면과 찾기 테스트는 이 타이머를 기다리지 않아 그대로 뒀어요.

아래는 2026-10-09에 trss-jobs `tests/it` 바이너리를 `--report-time`으로 한 번씩 돌린 시간이에요. 앞은 `eb5f495`, 뒤는 `5d1fb3b`예요.

| 테스트 | 앞 | 뒤 |
| --- | --- | --- |
| 멈춘 시간으로 바꾼 15개 | 합 45.778초, 가장 긴 것 8.023초 | 합 0.491초, 가장 긴 것 0.072초 |
| `screen::`과 `find::` 전체 | 57개, 합 46.909초 | 58개, 합 1.714초 |

- 가장 길던 것은 `screen::a_persons_switch_on_a_check_screen_is_kept_and_a_closed_shown_tab_goes_to_the_newest_left`예요.
- 멈춘 시간에서는 1초 간격의 페이지 확인과 테스트 도우미의 10ms 확인이 같은 순간에 겹칠 수 있어요. 그래서 `2d68052`에서 `screen::`과 `find::`를 30번 돌렸어요. 15번은 다른 작업이 없을 때, 15번은 CPU를 다 쓰는 다른 프로세스와 함께 돌렸어요. 실패는 없었어요. 부하 아래에서 `tests/it` 바이너리 전체를 5번 돌린 것도 모두 통과했어요.

### 실행 시간

2026-10-09에 `cargo test --locked --workspace -j 4`로 빌드한 뒤 다시 두 번 돌린 `finished in`이에요. 앞은 `eb5f495`, 뒤는 `5d1fb3b`예요. 그때 함께 일하던 agent들은 파일만 읽었고 cargo를 돌리지 않았어요.

| 테스트 바이너리 | 앞 | 뒤 |
| --- | --- | --- |
| trss-jobs `tests/it` | 414개 통과·5개 무시, 10.59초, 10.64초 | 435개 통과·5개 무시, 3.61초, 3.59초 |
| trss-jobs 라이브러리 | 124개, 1.92초, 1.94초 | 126개, 1.94초, 1.93초 |
| trss-jobs `tests/extract_process.rs` | 15개, 2.76초, 2.76초 | 15개, 2.74초, 2.74초 |
| trss-web 라이브러리 | 477개, 7.99초, 7.30초 | 457개, 7.14초, 7.16초 |
| trss-web의 다른 바이너리 4개 | 3개 통과·4개 무시, 4.60초, 4.71초 | 3개 통과·4개 무시, 4.54초, 4.55초 |
| 두 크레이트 합 | 1,033개 통과·9개 무시, 27.9초, 27.4초 | 1,036개 통과·9개 무시, 20.0초, 20.0초 |

줄어든 시간은 거의 다 `tests/it`에서 기다리던 실제 시간이에요. trss-web 라이브러리는 테스트 20개를 뺐지만 차이가 두 번 측정의 흔들림(7.30–7.99초)보다 작아요.

### 교체와 재배치의 겹침

`place/aside.rs`의 테스트 18개 중 11개는 교체나 재배치에 같은 경우가 없어요. 동기화 실패 셋, 되돌리는 사이의 경합, 다른 파일로 바뀐 경우, 없는 폴더 같은 것이에요. 나머지 7개와 겹치는 교체·재배치 테스트는 aside가 보지 않는 기록 상태, 보류가 번지는 범위, 사용자에게 보이는 문장까지 확인해요. 그래서 지운 것이 없어요. 재배치의 `another_file_found_aside_goes_back_unless_its_path_is_taken`과 겹치는 테스트는 "그 자리에 그대로 있음"이 사용자에게 보이는 결과여서 줄이지도 않았어요.

### 검증한 것

- `cargo test --locked --workspace -j 4`는 2,916개에서 2,921개가 되었고 모두 통과했어요. 실패는 0개, 무시는 14개예요. 커밋마다 돌렸고, 통과 수는 2,916, 2,916, 2,923, 2,919, 2,919, 2,920, 2,920, 2,921개였어요. 알려진 흔들리는 trss-web 테스트(`the_holder_shown_is_the_first_subscription_by_rule_id_whatever_the_listing_order`)가 한 번 실패했다가 다시 돌리니 통과했어요.
- 시계와 준비 함수를 모은 커밋은 `tests/it`의 테스트 이름 419개가 앞뒤로 같았어요. 테스트 몸통 419개를 공백을 무시하고 견줘 보니 하나만 달랐어요. `recheck.rs`의 테스트 하나가 `ticking_clock()` 대신 `ticking_from(NOW)`를 불러요. 시작 시각이 `NOW`여야 해서 옛 이름으로 다시 내보낼 수 없었어요.
- 멈춘 시간으로 바꾼 커밋도 테스트 몸통을 바꾸지 않았어요. `tokio::time::advance`를 더하거나 기다리는 시간을 바꾼 테스트가 없어요.
- `cargo fmt --all --check`는 깨끗하고, `cargo clippy --locked -p trss-jobs -p trss-web -p trss-library -p trss-subtitles --all-targets -j 4`에 경고가 없어요.

### 남은 것

- 원격 화면의 `a_check_screen_follows_a_popup_that_stays_and_lists_its_pages`와 찾기의 `a_page_that_opens_and_closes_at_once_is_not_followed`에서, 금방 닫히는 페이지는 +0.9초에 닫히고 페이지 확인은 +1.0초에 처음 일어나요. 그래서 따라가는 쪽은 그 페이지를 한 번도 보지 못해요. 멈춘 시간도 같은 순서를 지켜요. "한 번 보이고 사라짐"의 경로는 `runner/tender/pages.rs`의 단위 테스트만 확인해요.
- trss-jobs `tests/it/runner.rs`의 `a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_ends_it`는 실제 시간으로 1.6초를 기다려요. 원격 화면과 찾기가 아니어서 이 티켓에서 바꾸지 않았어요.
- `tests/it`의 `unpack.rs`, `winpng.rs`, `placement.rs`는 각자의 `tree`를 가져요. `world.rs`의 `tree`로 바꾸는 것은 기계적이지만 하지 않았어요. 여러 파일이 같은 모양으로 가진 `run`, `detail`, `step`, `names`, `sql`, `work` 도우미도 모으지 않았어요.
- 웹 코드에 결정이 적힌 규칙 9곳은 [0111](0111-web-and-screen-rules.md)에 더했고, 그 웹 테스트는 남겼어요.
- trss-web 안에서 `screen_api/tests.rs`와 `hub.rs`·`nav.rs`의 테스트가 겹치는 것은 크레이트 사이의 겹침이 아니어서 다루지 않았어요.
