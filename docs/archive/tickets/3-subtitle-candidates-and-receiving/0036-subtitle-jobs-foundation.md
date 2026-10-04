# 0036 자막 작업을 기록하고 할 일 화면과 작업 상세를 세워요

- 상태: 완료
- 출처: [할 일](../../../specs/jobs.md#할-일), [할 일 화면](../../../specs/jobs.md#할-일-화면), [작업 상세](../../../specs/jobs.md#작업-상세), [작업 결과](../../../specs/jobs.md#작업-결과), [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)
- 막는 티켓: [0030](0030-feature-crates.md), [0031](0031-immediate-commands.md)(수집 주기의 잠금 밖에서 도는 실행 기반)

## 작업

모든 자막 수신이 올라탈 작업 기반이에요. 출처별 수신과 인증은 뒤의 티켓이 더해요.

- 작업 관리 크레이트에 자막 작업·항목·파일 동작 기록을 둬요. 선택한 후보와 출처, 수신 시도 ID와 전용 임시 경로, 단계와 대기 사유(`인증 필요`·`자막 대기` 등)를 구분해 남기고, 쿠키·토큰·서명 URL은 저장하지 않아요.
- worker는 작업을 수집 주기와 별개로 실행하고, 재시작하면 기록을 보고 이어가거나 보류해요. 이 티켓에서 다루는 단계는 `후보 발견`부터 `받기`까지이며, 받은 묶음은 앱 데이터 폴더의 수신 영역에 둬요(분석·보관·적용은 결과 목표 4).
- 할 일 화면을 만들어요. `처리 필요`(이번 목표에서는 `인증 필요`, 이미 있는 `받기 실패`)와 `제안`(이미 있는 `제목 후보`·`보관 제안`, 이번 목표의 `자막 구독`)을 모으고, 아래 `자막 작업`은 `실패`·`대기 중`·`진행 중`·`최근 완료` 순서예요. 메뉴 배지는 처리 필요만 세요.
- `#/job/<작업>` 작업 상세에 돌아가기·머리·작업 단계·주 영역(이번 티켓은 회차·파일별 결과)·경로·작업 기록을 둬요.
- 시험용 가짜 출처로 작업 하나가 끝까지 흐르는 경로를 만들어, 출처가 없어도 기반을 검증할 수 있게 해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 가짜 출처로 작업을 만듦 | 작업 상세의 단계가 `후보 발견`부터 `받기`까지 시각과 함께 나아가고, 받은 묶음이 수신 영역에 있어요. 접수 응답만으로 완료로 보이지 않아요. |
| 받기 단계 도중 worker를 강제로 종료하고 다시 시작 | 완료 기록이 있는 수신물은 바이트를 확인해 재사용하고, 불명확하면 보류해요. 같은 파일을 두 번 받지 않아요. |
| 인증 필요 1·받기 실패 1·보관 제안 3·제목 후보 1 | 처리 필요 카드가 인증 필요, 받기 실패 순서이고, 제안 줄 넷, 메뉴 배지 2예요. |
| 자막 작업 실패 2·대기 5·진행 1·완료 115 | 실패가 맨 위, 완료는 다섯 개 뒤 스크롤로 이어지고, 실패는 할 일에 없어요. |
| 3개 회차 중 1개 실패한 작업 | 회차별 결과에 성공 2·실패 1과 이유가 있고 상태는 `일부 실패`예요. |
| 웹만 재시작 | 진행 중 작업이 끊기지 않고, 웹이 돌아오면 같은 상태를 보여줘요. |
| 작업 기록과 로그 | 쿠키·토큰·서명 URL이 없어요. |
| 휴대폰 너비 | 할 일 화면과 작업 상세에 가로 스크롤이 없어요. |

## 결과

### 만든 것 (2026-10-02)

- **저장(마이그레이션 34)**: `subtitle_jobs`(브라우저가 만든 요청 ID와 요청 내용, 작품·시즌·Anissia 작품·출처·제작자, 상태·대기 사유·단계·설명·시도 횟수), `subtitle_job_items`(고른 후보 하나: 관찰 ID와 그때의 게시물 주소·회차 표시), `subtitle_job_steps`(단계별 상태와 시각), `subtitle_job_events`(작업 기록), `subtitle_job_files`(파일 수신 하나: 시도 ID와 전용 임시 폴더, 알린 길이, 길이·SHA-256·파일 객체, 공개 경로, 같은 파일을 받은 수신). 상태는 `pending`·`running`·`waiting`(`auth`·`subtitle`)·`held`·`failed`·`partial`·`done`이에요. 쿠키·토큰·서명 주소를 담는 칸은 없어요.
- **크레이트**: `trss-subtitles`는 출처(열린 집합이라 enum)와 주소로 출처를 고르는 `Sources`, 가짜 출처를 둬요. `trss-jobs`는 기록(`JobStore`), 실행(`Runner`), 수신 영역(`ReceiveArea`)을 둬요. 둘 다 `import` 위, 바이너리 아래예요.
- **가짜 출처**: `fake.trss.invalid`의 게시물 경로가 동작을 정해요(`ok`·`shared`(여러 회차가 같은 파일)·`auth`·`missing`·`empty`·`short`(알린 길이보다 짧음), `?delay_ms=`). 늘 같은 정상 ASS 바이트를 줘요. worker는 `TRSS_FAKE_SUBTITLE_SOURCE=1`일 때만 켜고, 개발 환경은 `dev/trss.dev.yml`로 켜요. 운영 compose 파일은 그대로예요.
- **실행**: worker가 수집 주기와 따로 worker 잠금을 쥐고 작업을 하나씩, 회차를 차례로 실행해요. 시작할 때, 웹이 깨울 때, 3초마다 봐요. 출처가 없는 주소는 `자막 대기`, 사이트 확인은 `인증 필요`로 멈추고, worker가 시작하면 `자막 대기`만 다시 줄에 세워요. 한 작업 안에서 같은 파일은 한 번만 받아요. 수신 순서와 재시작 복구 판정은 [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)에 적었어요.
- **웹 API**: `POST /api/subtitle-jobs`(시즌에 연결된 Anissia 작품의 한 제작자 후보만, 같은 요청 ID 반복은 같은 작업, 다른 내용은 409, 만들면 worker를 깨움), `GET /api/subtitle-jobs`(묶음과 완료 첫 5개), `/done?after=`(완료 다음 쪽), `/{id}`(단계·회차·파일·경로·기록). `GET /api/todo`·`/api/todo/count`가 `인증 필요`(작품마다 하나)와 기존 `받기 실패`(대체 실패는 작품마다, 추가 실패는 규칙마다 하나)를 모아요.
- **화면**: 할 일 화면(`처리 필요` 카드, `제안` 줄, `자막 작업`의 실패·대기 중·진행 중·최근 완료와 무한 스크롤, 메뉴 배지)과 작업 상세(`/todo/job/<작업>`: 돌아가기·머리·단계·회차별 결과·경로·기록, 휴대폰에서는 단계를 한 줄로 접음). 명세의 `#/job/<작업>`은 다른 화면과 같은 경로 주소로 바꿨어요.
- 작업을 만드는 화면(작품 상세의 자막 후보 구역)은 0037이에요. 이 티켓에서는 API로 만들어요.


### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 가짜 출처로 작업을 만듦 | `a_job_goes_from_found_to_receive_and_leaves_its_files_in_the_receive_area`. 개발 환경에서 API로 만든 작업의 단계가 `후보 발견`·`게시물 열기`·`받기`로 시각과 함께 나아가고, 파일이 `receive/<작업>/`에 있었어요. 접수 응답(202) 뒤에는 `시작 대기`로 보여요. |
| 받기 도중 worker 강제 종료 후 재시작 | 개발 환경에서 2,097바이트 중 640바이트를 받았을 때 `docker kill trss-worker`, 다시 시작했어요. 그 시도는 `abandoned`(`2097바이트 중 640바이트에서 멈췄어요`)로 남고 새 시도가 한 번 받아 작업이 `done`이에요. 같은 경계와 그 밖의 재시작 판정은 시험으로 확인해요: `a_shutdown_in_the_middle_of_a_file_leaves_the_job_running_and_the_next_start_ends_it`, `an_intent_with_no_bytes_is_abandoned_and_received_anew`, `bytes_known_to_stop_short_are_received_again_and_whole_ones_are_published_as_they_are`, `bytes_with_no_announced_length_or_more_than_it_are_held_not_received_again`, `a_rename_done_before_its_record_is_found_by_its_object_and_bytes`, `a_fetched_file_is_published_but_a_copy_with_the_same_bytes_at_its_path_is_held`, `a_done_file_is_reused_after_its_bytes_are_checked_and_held_when_they_changed`, `an_unfinished_receipt_is_settled_from_the_disk_even_when_the_post_no_longer_opens`, `an_empty_temporary_file_is_no_bytes_even_when_nothing_was_announced`, `an_item_with_an_abandoned_attempt_still_records_the_shared_file` |
| 인증 필요 1·받기 실패 1·보관 제안 3·제목 후보 1 | `a_site_check_comes_before_a_receive_failure_and_the_badge_counts_both`(카드 순서와 배지 수 2). 화면은 개발 환경의 실제 `인증 필요` 하나에 나머지 응답을 브라우저에서 바꿔 넣어 봤어요: 카드가 인증 필요·받기 실패 순서이고, 제안 줄 넷, 메뉴 배지 `처리 필요 2`예요. 개발 데이터에는 제목 후보와 보관 제안이 없어서 실제 데이터로는 보지 못했어요. |
| 자막 작업 실패 2·대기 5·진행 1·완료 115 | `the_groups_put_failures_first_and_page_the_done_jobs_five_then_more`, `done_jobs_come_newest_first_a_page_at_a_time`. 실패는 `/api/todo`에 없어요. |
| 3개 회차 중 1개 실패 | `a_job_with_one_of_three_failed_is_partial_with_the_reason`, `a_job_with_one_of_three_failed_shows_each_episode_with_its_reason`. 개발 환경에서 `일부 실패`, 3화 `게시물이 없어요 (404)`를 확인했어요. |
| 웹만 재시작 | 개발 환경에서 작업이 도는 중에 웹 컨테이너만 다시 시작했어요. 작업은 이어서 끝났고 웹은 같은 상태를 보여줬어요. 실행은 worker에만 있어요. |
| 작업 기록과 로그 | 기록 칸에 쿠키·토큰·서명 주소를 담는 자리가 없어요. 개발 환경의 작업 표들과 worker 로그에서 `token`·`cookie`·`sig`·`key=` 따위를 찾았어요. 작업 표에는 없었고, 로그에는 이전부터 있던 RSS 주소 한 줄이 `token=***`로 가려져 있었어요. |
| 휴대폰 너비 | 할 일 화면과 작업 상세를 390px(어두운 테마)과 320px(밝은 테마)에서 봤어요. 가로 스크롤이 없어요. |

그 밖에 `a_file_two_posts_share_is_received_once`(같은 파일을 한 번만 받음), `a_site_check_and_an_unknown_site_wait_for_different_things`(`인증 필요`와 `자막 대기`), `the_same_browser_id_finds_its_job_and_another_request_with_it_is_refused`, `a_pick_makes_one_job_per_browser_id_of_one_creators_candidates`, `a_job_started_too_many_times_is_held_instead_of_blocking_the_line`, `a_job_that_ends_each_run_waiting_for_a_source_is_never_held_for_its_starts`, `an_unfinished_file_is_receiving_only_while_its_episode_runs`, 마이그레이션 `a_database_from_before_subtitle_jobs_keeps_its_observations_and_starts_with_no_job`(제약 7가지가 거절하는지와 지울 때 따라 지워지는지 포함)가 있어요. 작업 공간 시험 1,608개가 통과하고, clippy 경고가 없고, 웹 빌드가 돼요.

### 독립 리뷰

체크포인트·재시작 복구·동시 실행을 두고 독립 리뷰를 받았고, 고친 뒤 두 번 다시 확인받았어요. 막는 결함은 없었어요.

| 지적 | 처리 |
| --- | --- |
| 출처 조회나 게시물 열기에 실패하면 미완료 수신 기록을 정리하지 못함 | 게시물을 열기 전에 회차의 `intended`·`fetched` 기록부터 디스크와 맞춰요 |
| 수신 기록이 `synchronous=NORMAL`이라 전원이 꺼지면 파일 효과만 남을 수 있음 | 수신 기록 쓰기는 `FULL`로 커밋하고, 복구 때 임시 파일도 동기화한 뒤 읽어요. 새 작업 폴더는 매번 부모 폴더까지 동기화해요 |
| 빈 임시 파일을 알린 길이가 없을 때 받은 파일로 볼 수 있음 | 빈 파일은 받은 바이트가 없는 것으로 보고 버린 뒤 새로 받아요 |
| 이미 받은 원본을 다른 회차가 재사용할 때 그 회차의 완료 기록을 확인하지 않음 | 이 회차의 완료 기록이 있어야 재사용해요 |
| 파일 이름이 숨김 파일이 되거나 너무 길 수 있음 | 앞의 점과 공백을 지우고 200바이트로 자르며, 16바이트가 넘는 확장자는 이름과 함께 잘라요 |
| 한 worker 안에서 작업 실행이 겹칠 수 있음 | 실행 하나만 잠금을 쥐어요 |
| 종료 시 강제 중단해도 차단 쓰기는 계속될 수 있음 | 출처 호출을 중단에 반응하게 했어요. 남은 틈은 아래 한계에 적었어요 |
| 파일 객체 비교가 링크나 다시 만든 파일에 속을 수 있음 | 일반 파일만 읽고, 열린 뒤 바뀐 파일은 거절해요. 객체에 생성 시각을 더했어요 |
| 임시 폴더 칸이 비었거나 `.tmp/` 밖이면 이상하게 다룸 | 보류해요. 기록 하나만 생기지 않으면 오류예요 |
| 계속 실패하는 작업이 줄을 영원히 막음 | 실행이 끝나지 않은 채 다섯 번 시작한 작업은 보류해요. 처음 고친 판이 끝난 실행까지 세어 `자막 대기` 작업이 재시작 몇 번에 보류됐어요. 실행이 끝나면 다시 세고, 보류할 때 도는 회차와 단계도 멈춘 것으로 남겨요 |
| 게시물 열기 단계가 다른 회차의 실패로 `실패`가 됨 | 아직 `current`일 때만 바꾸고, 받기 단계는 있을 때만 정리해요 |
| 같은 원본·같은 경로가 두 번 기록될 수 있음 | 부분 고유 색인 둘과 상태 CHECK를 더했어요 |
| 종료 요청 뒤에도 다음 파일을 시작함 | 파일마다 먼저 확인해요 |
| 보류된 파일의 경로나 다른 순간의 상태를 보여줌 | 받은 파일만 경로를 보이고, 상세는 한 트랜잭션에서 읽어요. 멈춘 회차의 미완료 파일은 `보류`로 보여요 |
| `expected_size`가 압축 해제 전 길이일 수 있음 | 문서에 `chunk()`가 내는 길이여야 하고, 모르면 `None`이라고 적었어요 |

### 남은 한계

- 종료 유예가 지나 작업 태스크를 강제로 끝내면, 이미 시작한 차단 쓰기(파일 쓰기·DB 쓰기)는 잠금을 놓은 뒤에도 잠깐 이어질 수 있어요. 다음 시작의 복구가 기록과 디스크를 맞추지만, 그 틈을 막는 장치는 없어요.
- `.tmp/` 폴더와 수신 영역 자체를 처음 만들 때는 폴더를 동기화하지 않아요. 전원이 꺼져 이것이 사라지면 보류가 될 뿐 거짓 성공은 없어요. 전원 차단에 대한 내구성은 시험으로 증명하지 않았어요.
- 작업은 한 번에 하나씩 돌아요. 정상 종료(배포)도 끝나지 않은 시작으로 세서, 한 작업이 배포 다섯 번에 연달아 끊기면 보류돼요.
- 보류된 작업을 사람이 풀거나 다시 시작하는 동작은 아직 없어요.
- 작업을 만드는 화면은 0037이에요. 지금은 API로만 만들어요.
- 실제 출처는 하나도 없어요. 출처별 수신과 비밀값 처리, 실제 응답으로 확인할 일은 0038 이후예요.
- 게시물이 열리지 않으면 미완료 기록은 정리하지만 그 회차는 실패해요.
- 작업 상세의 경로는 서버 컨테이너 안의 경로예요.
- 0035의 수정 후보 판단(`revision_of`)은 아직 잠정 기준이에요. 이제 받은 기록(`subtitle_job_items`의 출처와 회차)이 있으니 0037에서 바꿀 수 있어요.
