# 0111 웹과 화면이 다시 계산하는 나머지 규칙을 모아요

- 상태: 대기
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07에 trss-web과 화면이 기능 크레이트의 규칙을 다시 계산하는 곳이 있었어요. 할 일 집계는 [0106](0106-todo-aggregation-in-jobs.md)에서 다루고, 나머지를 여기서 모아요.

| 규칙 | 지금 | 둘 곳 |
| --- | --- | --- |
| worker가 살아 있는지, 수집 주기가 멈췄는지 | trss-web의 상태 API에 있고 편성 API가 그것을 불러요. 상수와 기록은 trss-core의 하트비트에 있어요. 2026-10-02의 수정 두 번(`24024b5`, `86189e4`)이 두 API에 똑같이 들어갔어요. | trss-core의 하트비트 |
| 규칙 미리보기의 분류(내 것, 이전, 제외, 지난 것)와 규칙 대입 | trss-web의 규칙 API. 평가는 trss-collect의 `ChannelPlan`을 함께 써요. | trss-collect |
| 정규식이 맞는지 | trss-web의 가져오기 API, 규칙 API, trss-collect의 평가 | trss-collect |
| 날짜, 분기, 자막 형식, 회차 방영 시각 | 웹의 `civil_date`(trss-core의 날짜 계산과 같아요), 분기 도우미(분기 타입은 trss-collect), 확장자에서 자막 형식(trss-subtitles와 같아요), 회차 방영 시각 두 벌 | 그 타입이나 규칙의 주인 크레이트 |
| 회차 범위, 앞자리 0 지우기, 같은 날 공유본 판정, 서버 한도 숫자 | 화면에 회차 범위 5벌, 앞자리 0 지우기 3벌, 공유본 판정 2벌. 올리기 한도(500개, 200 MiB)와 원격 화면 크기(200–4096)를 손으로 옮겨 적었어요. | 서버가 계산해 보내요 |
| 요일 이름, 크기 표기 | 화면에 요일 이름 6벌(짧은 것과 긴 것, 일요일 먼저와 월요일 먼저), 크기 표기 2벌(`22 KB`와 `22KB`) | 화면의 한 표기 모듈 |

- 웹의 응답은 기존 필드의 모양을 지키고, 화면이 쓰던 값은 필드를 더해 보내요. 타입은 손으로 맞춰요.
- 미리보기와 실제 처리는 같은 trss-collect의 인터페이스를 써서 결과가 같아요.
- 크기 표기처럼 하나로 맞추면 화면이 달라지는 것은 따로 커밋하고 결과 절에 적어요.
- 규칙 미리보기의 분류 테스트(trss-web `rules_api/tests.rs` 8개, `subscriptions_api/tests.rs` 1개, `subscriptions_api/title_tests.rs`의 미리보기 부분)는 `build_preview`를 옮길 때 trss-collect의 표 테스트 하나로 내려요. 웹에는 `POST /api/rules/preview`의 응답 모양, 404, 400, `subscribing`일 때만 오는 `episode_suggestion`을 확인하는 테스트를 남겨요([0101](0101-receive-line-tests.md)에서 정함).
- 2026-10-09 [0101](0101-receive-line-tests.md)에서 두 곳을 더 찾았어요. 옮길지는 여기서 정해요.
  - 웹의 "이미 놓였는지" 규칙(`in_place`)은 수정본 `다시 받기`를 trss-collect의 판정보다 먼저 거절하고, 그 순서를 웹이 가져요.
  - 규칙 API는 규칙마다 마지막으로 받은 시각을 `channel_items`에서 다시 계산해요. 보관 제안 API는 trss-collect의 `last_received_of_rules`를 써요.
- 2026-10-09 [0110](0110-job-line-tests.md)에서 자막 작업 줄기의 아홉 곳을 더 찾았어요. 그 웹 테스트는 남겼고, 옮길지는 여기서 정해요.
  - Anissia 후보의 수정본 표시: `seasons_anissia_api.rs`가 회차 대응, 예외, 오프셋 순으로 후보의 회차를 정한 뒤 trss-collect의 `revision_by_attribution`·`revision_by_attributed_episode`를 불러요. 웹 테스트는 `mapping_api`의 `an_exception_decides_which_candidate_revises_a_subtitle_file_the_user_named`와 `subtitle_creator_api`의 `the_creators_later_candidate_of_an_episode_with_such_a_file_is_a_revision_candidate`, `the_sources_mapping_decides_which_episode_of_the_season_a_candidate_is`, `a_season_without_a_creator_named_marks_no_candidate_a_revision`이에요.
  - 실패한 교체의 옛 파일과 새 파일의 상태(`todo_api::failure_of`): `a_failed_replacements_files_are_told_by_its_row`.
  - 올린 자막의 제작자가 그 시즌의 후보 중 하나여야 하는 것(`subtitle_upload_api::target_of`): `the_creator_is_one_of_the_seasons_or_unknown`.
  - 원격 화면의 연결이 아직 그 작업의 것인지 정하는 조건이 `screen_api.rs`의 `socket`과 `screen_api/hub.rs`의 `watch_binding` 두 곳에 있어요. 웹 테스트는 `a_socket_from_another_site_for_a_job_not_waiting_or_for_an_ended_run_is_refused`, `another_check_in_the_same_run_ends_the_socket_and_is_connected_to_anew`, `a_socket_ends_when_the_run_is_no_longer_the_jobs_or_the_browser_goes`예요.
  - 작업 목록의 묶음과 기다리는 작업의 순서(`jobs_api.rs`의 `groups`, `waiting_rank`, `DONE_FIRST`, `DONE_PAGE`).
  - 고르기 작업의 후보 검사(그 시즌의 후보이고, 겹치지 않고, 한 제작자의 것)와 찾기 작업이 제작자의 가장 새 게시물을 고르는 것(`jobs_api.rs`의 `create`, `create_find`).
  - 파일과 풀기 단계의 상태를 화면의 상태로 바꾸는 것(`jobs_api.rs`의 `file_view`, `unpack_view`).
  - 교체 화면이 파일의 `mtime`을 밀리초로 바꾸는 계산(`jobs_api/replacement.rs`)이 trss-jobs `todo/changes.rs`와 두 벌이에요.
  - 작품 화면이 보관본을 제작자별로 묶고 고르면 무엇을 하는지 정하는 것(`library_work_api.rs`의 `creators_of`).

## 완료 기준

- 표의 규칙이 각각 둘 곳에 하나 있고, trss-web과 화면에 다시 계산하는 코드가 없어요.
- 기존 테스트가 고치지 않고 통과해요. 그 뒤 trss-web의 상태·편성 API 테스트 중 살아 있음 판정을 다시 확인하는 것(2026-10-07에 9개, 약 330줄)을 ADR 0015대로 정리해요.
- 화면에 보이는 값이 같아요. 다르게 맞춘 표기는 결과 절에 있어요.
