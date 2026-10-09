# 0111 웹과 화면이 다시 계산하는 나머지 규칙을 모아요

- 상태: 완료
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
| 회차 범위, 앞자리 0 지우기, 같은 날 공유본 판정, 서버 한도 숫자 | 화면에 회차 범위 5벌, 앞자리 0 지우기 3벌, 공유본 판정 2벌. 올리기 한도(500개, 200 MiB)와 원격 화면 크기(200–4096)를 손으로 옮겨 적었어요. | 서버가 계산해 보내요. 같은 날 공유본 판정은 보는 사람의 시간대를 쓰므로 화면의 한 모듈이에요(사용자 결정, 2026-10-09). |
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

## 결과

### 결정

- 크기 표기는 어디서나 `22 KB`처럼 숫자와 단위를 띄우고, GB 단계를 더해요(사용자 결정, 2026-10-09). 올리기 구역의 `22KB`, `200MB까지`와 서버의 실패 까닭 문구(`200MiB`, `{}MB보다 큰 이미지`)가 이 꼴로 바뀌었어요.
- 회차 범위는 계산만 서버의 한 곳에서 해요(사용자 결정, 2026-10-10). 계산 규칙은 라이브러리 목록이 쓰던 것으로, 숫자 순서이고 같은 회차는 한 번이며 `13.0`은 13이고 `0`은 범위 안에 들고 정수가 아닌 회차는 뒤에 와요. 잇는 기호와 붙이는 글자(`·`, ` · `, 지난 회차 검색 문장의 `, `, `화`)와 작업 줄의 "앞 세 범위 + `외 N개`" 줄임은 자리마다 그대로 둬요.
- 입력하는 동안의 미리보기(회차 대응 창, 지난 회차 검색의 범위 이름표)는 서버가 미리보기 요청으로 계산해요(사용자 결정, 2026-10-09).
- 같은 날 받은 사본에 시각을 붙이는 판정은 화면의 한 모듈에 두고 보는 사람의 시간대를 그대로 써요(사용자 결정, 2026-10-09). 그래서 [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기)의 "서버가 계산해 보내요"에서 이 규칙은 뺐어요.
- 아래는 리드가 정했어요.
  - worker가 살아 있는지는 trss-core `heartbeat`의 함수가 원시 값으로 판정하고, Transmission을 본 값이 아직 새로운지는 trss-collect `store::status`가 판정해요.
  - `in_place`는 웹에 둬요. 수정본 `다시 받기`를 trss-collect의 판정보다 먼저 거절하는 순서가 웹의 것이고, 웹의 사전 검사는 worker의 실행 검사를 대신하지 않아요. 명령 API와 할 일 API가 따로 짜던 다시 받기 제안만 하나로 모았어요.
  - 규칙 미리보기와 수집 주기의 알려진 차이(새 구독은 `past`가 되지 않음, `i64::MAX` 흉내, 이력 20,000개)는 그대로 둬요. 규칙의 마지막 받은 시각만 결함 수정으로 따로 고쳤어요.
  - 요청 ID 검사의 차이는 기록만 하고 바꾸지 않아요. 고르기 작업 만들기(`create`)는 앞뒤 공백을 지우지 않고 128자 제한이 없고, 찾기 작업 만들기(`create_find`)와 자막 올리기의 대상(`target_of`)은 둘 다 해요.
  - 화면이 손으로 옮겨 적은 서버 한도는 화면이 그때 이미 받는 응답의 필드로 보내요. 가져오기 화면의 2 MB 사전 검사만 화면에 남겼어요. 파일을 고르기 전에는 받는 응답이 없고, 큰 파일을 브라우저가 읽지 않게 하는 검사예요.
  - 응답 객체 전체를 견주는 기존 테스트에는 새 필드만 더해요. 명세가 필드 더하기를 허용하기 때문이에요.

### 바꾼 것

| 커밋 | 내용 |
| --- | --- |
| `refactor(core): judge worker liveness in the heartbeat module (0111)` | worker가 살아 있는지와 수집 주기가 멈췄는지를 trss-core `heartbeat`로 옮겼어요. Transmission을 본 값이 새로운지는 trss-collect `TransmissionCounts::look_is_fresh`예요. |
| `refactor(collect): move the rule preview and the overlap flag out of the web (0111)` | 규칙 대입, 미리보기 분류, 겹침 표시를 trss-collect `plan::preview`로 옮겼어요. |
| `fix(web): tell a rule's last received time from the whole history (0111)` | 규칙마다 마지막으로 받은 시각을 채널의 최근 20,000개 항목 대신 trss-collect의 `last_received_of_rules`로 알아요. 마지막으로 받은 항목이 그 밖에 있으면 전에는 `null`이었어요. 테스트를 먼저 써서 이 실패를 재현했어요. |
| `refactor(collect): compile a rule's regex in one place (0111)` | 정규식 검사를 `rss::compile_regex`, `regex_error` 하나로 모았어요. |
| `refactor(web): use the core calendar for the status board's dates (0111)` | 웹의 `civil_date`와 `DAY_MS` 두 벌을 trss-core `calendar`로 바꿨어요. |
| `refactor(collect): put the quarter helpers on Quarter (0111)` | 분기 도우미를 `Quarter::next`, `Quarter::of_anime`으로 옮겼어요. |
| `refactor(jobs): give the shown subtitle format of a name to SubtitleFormat (0111)` | 교체 화면이 이름으로 보이는 자막 형식을 `SubtitleFormat::shown_for_name`으로 옮겼어요. |
| `refactor(library): look up an episode's air time in the library (0111)` | 회차 방영 시각을 `seasons::combine::air_time_of`로 옮겼어요. |
| `refactor(web): build the retry offer of a stopped revision in one place (0111)` | 멈춘 수정본의 다시 받기 제안을 웹의 `in_place::Evidence::retry_check` 하나로 모았어요. |
| `test(core): test worker liveness once, in the heartbeat module (0111)`<br>`test(collect): test the rule preview once, in the plan's preview module (0111)` | 상태·편성 API의 살아 있음 테스트와 규칙·구독 API의 미리보기 분류 테스트를 내렸어요. |
| `refactor(collect): decide an Anissia candidate's revision mark beside its primitives (0111)` | Anissia 후보의 수정본 표시를 trss-collect `revision_by_mapping`, `mark_attributed`로 옮겼어요. |
| `refactor(collect): tell the files of a failed replacement from its row in trss-collect (0111)` | 실패한 교체의 옛 파일과 새 파일 상태를 `Revision::old_video_removed`, `new_video`로 옮겼어요. |
| `refactor(jobs): read the receive failures in one place for the list and the to-do (0111)` | 할 일 API와 할 일 카드가 받기 실패를 trss-jobs `todo::receive_failures` 하나로 읽어요. |
| `refactor: decide a creator's candidates and the jobs made of them outside the web (0111)` | 제작자의 가장 새 게시물과 고르기 검사를 trss-collect `AnissiaStore::newest_of_creator`, `pick`으로, 작업 만들기를 trss-jobs `NewJob::pick`, `NewFind::of`, `NewItem::of`로 옮겼어요. 저장하는 요청 JSON은 바이트까지 같아요. |
| `refactor(jobs): decide whether a remote screen still belongs to its job in Screen (0111)` | 원격 화면의 연결이 아직 그 작업의 것인지를 `Screen::seat` 하나로 정해요. |
| `refactor(jobs): group the open jobs and tell what a job waits for in trss-jobs (0111)` | 작업 목록의 묶음과 기다리는 작업의 순서를 `JobViews::open_groups`, `JobRow::waits_for_check`, `waits_for_placement`로 옮겼어요. |
| `refactor(jobs): tell how a receipt and its unpacking stand in trss-jobs (0111)` | 파일과 풀기 단계의 상태를 `FileRow::shown`, `unpack_status`로 옮겼어요. |
| `refactor(jobs): convert a seen file's change time to milliseconds in one place (0111)` | `mtime`을 밀리초로 바꾸는 계산을 `FileSeen::mtime_ms`로 모았어요. |
| `refactor(jobs): group the stored copies by creator and decide what choosing one does in trss-jobs (0111)` | 보관본을 제작자별로 묶고 고르면 무엇을 하는지를 `by_creator`, `StoredOptions::choice`, `blocked`, `StoredOnly::can_apply`로 옮겼어요. |
| `test(collect): test an Anissia candidate's revision mark once, in trss-collect (0111)`<br>`test(collect): test the files of a failed replacement once, on its row (0111)`<br>`test(collect): test which creators an anime has once, in trss-collect (0111)`<br>`test(jobs): test the job list's groups and waiting order once, in trss-jobs (0111)`<br>`test(jobs): test what came of unpacking an archive once, in trss-jobs (0111)`<br>`test(jobs): test the stored copies' grouping and choice once, in trss-jobs (0111)` | 위 규칙의 웹 테스트를 주인 크레이트로 내렸어요. |
| `refactor(core): compute episode runs once in trss-core episode (0111)` | 라이브러리 목록의 범위 계산을 trss-core `episode`의 `runs`, `segments`로 옮기고, 앞자리 0을 지우는 `shown`을 더했어요. |
| `feat(web): send episode runs and shown episode texts in the answers (0111)`<br>`feat(web): send the shown episode of candidates and receive-failed cards (0111)` | 화면이 범위와 보이는 회차를 계산하던 응답에 그 값의 필드를 더했어요. |
| `feat(web): show the episode runs the server computes (0111)` | 화면이 서버의 범위를 보여주고 잇기만 해요. 보이는 것이 달라진 입력은 [아래](#화면에-보이는-것이-달라진-입력)에 있어요. |
| `refactor(web): show the episode texts the server strips of leading zeros (0111)` | 화면의 앞자리 0 지우기 세 벌을 지우고 서버의 필드를 보여줘요. |
| `feat(web): send the server limits the screens copied by hand (0111)`<br>`refactor(web): take the server limits from the answers instead of copies (0111)` | 화면이 옮겨 적은 서버 한도를 응답 필드로 보내고, 화면의 복사본과 끝난 작업 요청의 `&limit=20`을 지웠어요. |
| `feat(web): preview the episode mapping dialog's input on the server (0111)`<br>`refactor(web): show the mapping dialog's previews from the server (0111)` | 회차 대응 창의 미리보기 요청을 더하고, 화면의 `mapping.ts`를 지웠어요. |
| `feat(web): label the past-search range on the server (0111)`<br>`refactor(web): show the past-search range label from the server (0111)` | 지난 회차 검색의 범위 이름표 요청을 더하고, 화면의 `rangeLabel`, `folderEpisode`를 지웠어요. |
| `refactor(web): keep the weekday names in one screen module (0111)` | 요일 이름 여덟 벌을 `web/src/lib/weekday.ts`로 모았어요. 범위 밖 값의 대체 글자(`""`, `?요일`, `신작`)는 부르는 곳마다 그대로예요. |
| `refactor(web): move the size text out of the to-do screen (0111)`<br>`feat(web): tell sizes with a space and a GB step everywhere (0111)` | 크기 표기를 `web/src/lib/size.ts`로 옮기고, 올리기 구역과 표지 창, 가져오기 화면도 그것을 써요. |
| `refactor(web): keep the same-day time of received copies in one screen module (0111)` | 같은 날 받은 사본에 시각을 붙이는 판정을 `web/src/screens/library/received.ts`로 모았어요. 카드마다 묶는 기준은 그대로예요. |
| `docs(web): list the mapping preview route once in the mapping API table (0111)` | 회차 대응 API의 주석 표에 두 번 적힌 미리보기 줄을 하나로 줄였어요. |
| `feat: name sizes in the server's reasons with a space, as the screen does (0111)` | 서버의 실패 까닭 문구가 크기를 `200 MB`처럼 적어요. 바뀐 문구는 [아래](#화면에-보이는-것이-달라진-입력)에 있어요. |
| `docs: record the result of gathering the web and screen rules (0111)`(이 결과를 담은 커밋) | 이 결과, 명세 검증 표의 테스트 이름, readme의 살아 있음 판정 설명이에요. |

### 규칙이 있는 곳

| 규칙 | 지금 있는 곳 | 웹과 화면에 남은 것 |
| --- | --- | --- |
| worker가 살아 있는지, 수집 주기가 멈췄는지 | trss-core `heartbeat`(`FRESH_FOR_MS`, `running_bound`, `CycleMarks`, `WorkerHeartbeat::is_beating`·`is_hung`, `worker_busy`, `cycle_stalled`). Transmission을 본 값은 trss-collect `store::status`(`DOWNLOADING_FRESH_CYCLES`, `look_is_fresh`) | 기록된 주기가 없으면 멈춤이 아닌 것, 다음 확인 시각, 보는 사람 시간대의 날짜 묶기, 응답 구조체 |
| 규칙 미리보기의 분류와 규칙 대입 | trss-collect `plan::preview`(`preview`, `overlapping_rules`) | 응답 구조체와 변환, 400·404, `subscribing`일 때의 `episode_suggestion`, 정규식 오류 문구 |
| 규칙의 마지막 받은 시각 | trss-collect `last_received_of_rules` | 없음 |
| 정규식이 맞는지 | trss-collect `rss::compile_regex`, `regex_error` | 오류 문구와 상태 코드 |
| 날짜, 분기, 자막 형식, 방영 시각 | trss-core `calendar`, trss-collect `Quarter::next`·`of_anime`, trss-jobs `SubtitleFormat::shown_for_name`, trss-library `air_time_of` | 다가오는 분기인지 보는 `quarter > current` 비교 두 곳 |
| 다시 받기 제안 | trss-web `in_place::Evidence::retry_check` | 이 규칙은 웹의 것이에요. 수집 이력 API는 자기 `can_retry`를 따로 짜요. |
| Anissia 후보의 수정본 표시 | trss-collect `revision_by_mapping`, `mark_attributed` | 입력 읽기와 `RevisionView` |
| 실패한 교체의 파일 상태 | trss-collect `Revision::old_video_removed`, `new_video` | 문구, 작품 폴더 경로 표시, `RetryOffer` |
| 받기 실패 모으기 | trss-jobs `todo::receive_failures` | 없음 |
| 제작자의 후보와 작업 만들기 | trss-collect `AnissiaStore::newest_of_creator`, `pick`, trss-jobs `NewJob::pick`, `NewFind::of`, `NewItem::of` | 요청 본문 검사(빈 목록, 200개, 중복), `post_url`의 scheme 검사, 상태 코드와 문구, 연결 찾기 도우미(`season_link`, `linked_anime`) |
| 원격 화면의 연결이 그 작업의 것인지 | trss-jobs `Screen::seat` | Origin·Host, 503, `ended`의 까닭, hub의 수명 |
| 작업 목록의 묶음과 기다리는 순서 | trss-jobs `JobViews::open_groups`, `JobRow::waits_for_check`, `waits_for_placement` | 끝난 작업의 쪽 크기(`DONE_FIRST`, `DONE_PAGE`, `DONE_PAGE_MAX`), 질의 읽기 |
| 파일과 풀기 단계의 상태 | trss-jobs `FileRow::shown`, `unpack_status` | 상태 이름 글자 |
| `mtime`의 밀리초 | trss-jobs `FileSeen::mtime_ms` | 없음 |
| 보관본의 제작자별 묶음과 고르기 | trss-jobs `by_creator`, `StoredOptions::choice`·`blocked`, `StoredOnly::can_apply` | `{:02}` 회차 글자와 응답 구조체 |
| 회차 범위 | trss-core `episode`(`EpisodeSet`, `runs`, `range_texts`, `segments`) | 자리마다 잇는 기호와 `화`, 작업 줄의 줄임(`web/src/screens/todo/episodeLine.ts`) |
| 앞자리 0 지우기 | trss-core `episode::shown` | 없음 |
| 서버 한도 숫자 | 서버의 각 상수. 응답 필드 `upload_limits`, `cover_max_bytes`, `limits`(원격 화면), `max_job_candidates`, `max_tries`, `max_entries`, `max_page`, `name_max_chars` | 가져오기 화면의 2 MB 사전 검사 |
| 회차 대응 창의 미리보기 | trss-library `mapping::preview`, `POST /api/library/works/{id}/seasons/{n}/anissia/sources/{source}/mapping/preview` | 줄 편집 상태(`rowsOf`)와 배치 |
| 지난 회차 검색의 범위 이름표 | trss-collect `episode_offset::range_label`, `POST /api/rules/{id}/past-search/range`, 결과의 `range_label` | 없음 |
| 요일 이름 | `web/src/lib/weekday.ts` | 부르는 곳마다의 대체 글자, Anissia 탭의 `기타`·`신작` |
| 크기 표기 | `web/src/lib/size.ts`. 서버 문구는 같은 꼴을 각자 적어요. | 없음 |
| 같은 날 시각 | `web/src/screens/library/received.ts`의 `carriesTime` | 카드마다 묶는 기준. `파일` 카드는 같은 회차(놓이지 않은 사본은 같은 이름), `자막` 카드는 같은 제작자·시즌·회차·형식이에요. |

- 두 미리보기 요청은 규칙 미리보기처럼 첫 요청을 바로 보내고, 그 뒤로는 마지막 입력에서 300 ms 뒤에 보내요. 낡은 답은 버리고, 다음 답이 올 때까지 마지막 답을 보여주며, 실패해도 입력을 지우지 않아요. 회차 대응 창의 `저장`은 지금 입력에 대한 답이 와야 눌려요.
- 회차 대응의 저장된 대응(`MappingView`)에 제작자 묶음의 한 줄(`line`)과 창이 처음 고를 선택(`choice`)을 더했어요.

### 화면에 보이는 것이 달라진 입력

| 맞춘 것 | 자리 | 입력 | 전 | 뒤 |
| --- | --- | --- | --- | --- |
| 회차 범위 | 작업과 할 일 줄 | 오름차순이 아니거나 겹친 회차 | `3·2·1화`, `2·2화`, `13·13.0화`, `12·13.0·14화` | `1–3화`, `2화`, `13화`, `12–14화` |
| 회차 범위 | 작업과 할 일 줄 | 정수가 아닌 회차 | `SP·1–2화`, `13.5·14화`, `9.5·10.5화` | `1–2·SP화`, `14·13.5화`, `10.5·9.5화` |
| 회차 범위 | 후보 묶음, 따라 받기 제안 | `0`, `00` | `1–2화 · 0`, `1화 · 00` | `0–2화`, `0–1화` |
| 회차 범위 | 후보 묶음, 따라 받기 제안 | `13.0`, `13.00` | `13–14화`, `13화` | `13.0–14화`, `13.00화` |
| 회차 범위 | 시즌 칸 | `13.0`이나 겹친 회차 | `12·14·13.0`, `13·13.0`, `1·1` | `12–14`, `13`, `1` |
| 앞자리 0 | 작품 상세의 회차 줄 주소 | 손으로 적은 `?episode=03` | 회차 `3`의 줄을 찾았어요 | 찾지 않아요. 앱이 만드는 주소는 보이는 회차를 써요. |
| 회차 대응 미리보기 | 회차 대응 창 | 앞뒤 공백이 있는 회차, i64보다 큰 정수 회차 | 화면 복사본의 결과 | 저장과 작업이 쓰는 trss-library의 결과(놓이지 않음) |
| 크기 표기 | 올리기 구역 | 고른 파일 크기, 한도 문장 | `22KB`, `512B`, `3.4MB`, `200MB까지`, `1GB까지`, `지금 1.2GB예요` | `22 KB`, `512 B`, `3.4 MB`, `200 MB까지`, `1 GB까지`, `지금 1.2 GB예요` |
| 크기 표기 | 표지 창, 가져오기 화면 | 한도 문장 | `10MB까지`, `2MB 이하의 기존 YAML 파일` | `10 MB까지`, `2 MB 이하의 기존 YAML 파일` |
| 크기 표기 | 1 GiB 이상을 보이는 모든 곳 | 1 GiB, 1.5 GiB | `1024 MB`, `1536 MB` | `1 GB`, `1.5 GB` |
| 크기 표기 | 서버: 자막 올리기 한도 | 한도를 넘은 요청 | `1GiB`, `200MiB`, `1KiB` | `1 GB`, `200 MB`, `1 KB` |
| 크기 표기 | 서버: 가져오기 | 큰 본문 | `2MB 이하인지` | `2 MB 이하인지` |
| 크기 표기 | 서버: 자막 올리기의 느린 본문 | 평균 속도가 모자란 요청 | `평균 16KiB/초` | `평균 16 KB/초` |
| 크기 표기 | 서버: 자막 받기와 올리기의 파일 아님 | 200 MiB를 넘는 파일 | `200MiB를 넘어요`, `받은 파일이 200MiB를 넘어 …` | `200 MB를 넘어요`, `받은 파일이 200 MB를 넘어 …` |
| 크기 표기 | 서버: 압축 해제의 거절과 실패 | 한도를 넘은 압축 파일 | `풀린 크기가 …MiB를 넘어요`, `멤버 하나가 200MiB를 넘어요`, `사전 크기가 64MiB를 넘어요`, `메모리 한도(256MiB)` | `… MB를 넘어요`, `멤버 하나가 200 MB를 넘어요`, `사전 크기가 64 MB를 넘어요`, `메모리 한도(256 MB)` |
| 크기 표기 | 서버: 표지 올리기 | 큰 이미지 | `10MB보다 큰 이미지는 받지 않아요.` | `10 MB보다 큰 이미지는 받지 않아요.` |

- 정수가 아닌 회차는 글자 순서로 뒤에 와서 `10.5`가 `9.5`보다 앞서요. 라이브러리 목록이 쓰던 규칙 그대로예요.
- `MB`와 `KB`는 화면의 `sizeText`처럼 1024 단위예요. 값은 바뀌지 않고 단위 글자만 바뀌었어요.
- 이미 저장된 작업과 수정본의 까닭(예전 문구)은 그대로 남아요. 새로 생기는 까닭부터 새 문구예요.

### 파일별 테스트 수

"앞"은 `722d5cb`, "뒤"는 이 결과를 담은 커밋의 `#[test]`·`#[tokio::test]` 개수예요. 표 테스트는 하나로 셌어요.

| 파일 | 앞 | 뒤 | 지움 | 내림 | 줄임 |
| --- | ---: | ---: | ---: | ---: | ---: |
| trss-web `status_api/tests.rs` | 14 | 7 | 0 | 7 | 1 |
| trss-web `status_api.rs`(`date_tests`) | 1 | 0 | 0 | 1 | 0 |
| trss-web `schedule_api/tests.rs` | 18 | 16 | 0 | 2 | 0 |
| trss-web `rules_api/tests.rs` | 39 | 38 | 0 | 2 | 6 |
| trss-web `subscriptions_api/tests.rs` | 29 | 27 | 0 | 2 | 1 |
| trss-web `subscriptions_api/title_tests.rs` | 11 | 11 | 0 | 0 | 3 |
| trss-web `mapping_api/tests.rs` | 9 | 12 | 1 | 0 | 1 |
| trss-web `subtitle_creator_api/tests.rs` | 9 | 8 | 1 | 0 | 1 |
| trss-web `jobs_api/tests.rs` | 10 | 10 | 1 | 0 | 2 |
| trss-web `todo_api/tests.rs` | 7 | 8 | 0 | 0 | 1 |
| trss-web `library_work_api/tests.rs` | 19 | 22 | 0 | 0 | 2 |
| trss-web `subtitle_upload_api/tests.rs` | 20 | 20 | 0 | 0 | 2 |
| trss-library `store/library/overview.rs` | 8 | 4 | 0 | 4 | 0 |
| 화면 `library/detail/mapping.test.ts` | 18 | 0 | 0 | 18 | 0 |

- "내림"은 아래 크레이트에 같은 입력과 기대의 테스트를 먼저 쓰고 지운 것이에요. 위 표의 웹 파일 중 테스트가 늘어난 것은 새 필드와 새 요청의 응답 모양 테스트를 더했기 때문이에요.
- 회차 대응 창의 18가지 경우는 trss-library `mapping/preview/tests.rs`의 앞 18개 테스트로 순서대로 옮겼어요. 지운 경우는 없어요.

테스트를 받거나 더한 파일이에요.

| 크레이트 | 파일 | 앞 | 뒤 |
| --- | --- | ---: | ---: |
| trss-core | `heartbeat.rs` | 4 | 12 |
| trss-core | `calendar.rs` | 3 | 4 |
| trss-core | `episode.rs` | 10 | 20 |
| trss-collect | `plan/preview/tests.rs`(새 파일) | 0 | 13 |
| trss-collect | `store/status/tests.rs` | 17 | 19 |
| trss-collect | `subscriptions/tests.rs` | 3 | 5 |
| trss-collect | `rss/evaluate.rs` | 16 | 17 |
| trss-collect | `store/anissia/captions_tests.rs` | 12 | 17 |
| trss-collect | `store/revisions/tests.rs` | 18 | 19 |
| trss-collect | `episode_offset.rs` | 26 | 27 |
| trss-jobs | `store/rows.rs`, `place/records.rs`, `store/create.rs`, `store/find.rs`, `screen.rs`, `model.rs`, `place/replace/records.rs`, `place/unpack.rs` | 5 | 19 |
| trss-jobs | `tests/it/todo.rs` | 5 | 9 |
| trss-jobs | `tests/it/follow.rs` | 49 | 50 |
| trss-library | `mapping/preview/tests.rs`(새 파일) | 0 | 26 |
| trss-library | `seasons/combine.rs` | 5 | 6 |
| trss-web | `library_api`, `seasons_anissia_api`, `seasons_api`, `channels_api`, `screen_api`의 `tests.rs` | 93 | 100 |
| trss-worker | `tests/it/past_search.rs` | 32 | 36 |
| 화면 | `lib/weekday.test.ts`, `lib/size.test.ts`, `library/received.test.ts`, `library/detail/episodeKey.test.ts`, `todo/episodeLine.test.ts`(모두 새 파일) | 0 | 16 |
| 화면 | `library/detail/upload.test.ts` | 9 | 10 |

### 지운 테스트와 남은 테스트

| 지운 테스트 | 남은 테스트 | 같은 경우인 까닭 |
| --- | --- | --- |
| trss-web `mapping_api/tests.rs` `an_exception_decides_which_candidate_revises_a_subtitle_file_the_user_named` | trss-collect `store/anissia/captions_tests.rs` `a_mappings_offset_moves_the_candidates_episode_to_the_season_files_it_is_compared_with` | 같은 예외와 차이를 표의 줄로 넣어 같은 수정본 표시를 얻어요. |
| trss-web `subtitle_creator_api/tests.rs` `a_season_without_a_creator_named_marks_no_candidate_a_revision` | 같은 표의 "대응 없음, 가진 파일 없음" 줄 | 제작자를 적지 않은 시즌이라는 같은 입력에서 아무 후보도 표시하지 않아요. |
| trss-web `jobs_api/tests.rs` `a_find_job_waits_among_the_ordinary_waits_not_with_the_checks` | trss-jobs `store/rows.rs`의 같은 이름 테스트 | 같은 작업 줄과 같은 순서 기대로 먼저 썼어요. |

### 남은 것

- 화면에 남은 서버 한도 복사본은 가져오기 화면의 2 MB 사전 검사 하나예요(리드 결정).
- 같은 계열의 계산이 남은 곳이에요. 옮기지 않은 까닭을 함께 적어요.
  - trss-jobs `todo/gather.rs`의 `mtime` 밀리초 변환은 `FileSeen`이 아닌 다른 구조체를 `div_euclid`로 바꿔요.
  - 수집 이력 API는 다시 받기 가능 여부(`can_retry`)를 `in_place`와 따로 짜요.
  - 다가오는 분기인지 보는 `quarter > current`가 웹 두 곳에 있어요. `Ord` 비교 하나라 그대로 뒀어요.
- 화면이 더는 쓰지 않는 응답 필드(`season_episodes`, `SearchContext.offset`, `season`)를 아직 보내요. 필드를 지우는 것은 JSON 모양을 바꾸는 일이라 이 티켓에서 하지 않았어요.
- 제목을 정하는 PUT과 `/title` 요청이 미리보기의 `titled`를 바꾸는 연결은 이제 웹 테스트가 확인하지 않아요. 저장소가 `titled_at`을 적는 것은 trss-collect `store/channels/title_tests.rs`가 확인해요.
- 두 미리보기 화면과 크기 표기는 브라우저로 보지 않았어요. 화면 테스트와 typecheck만 통과했어요.

### 확인

- workspace 테스트(`cargo test --locked --workspace -j 4`)는 `722d5cb`의 2,921개 통과에서 3,011개 통과가 됐어요(2026-10-10, `bad6692`). 실패는 0개, 무시는 14개예요.
- 화면 테스트(`npm test`)는 223개에서 222개가 됐고, `npm run typecheck`도 통과했어요.
- `cargo fmt --all --check`와 `cargo clippy --locked --workspace --all-targets -j 4`에 경고가 없었어요.
