# 0045 구독 제작자의 새 회차와 수정본을 자동으로 받고 `자막 구독` 할 일을 만들어요

- 상태: 완료
- 출처: [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신), [제작자 자동 수신 ADR](../adr/0009-follow-subtitle-creator.md), [자막의 회차 대응](../specs/library.md#자막의-회차-대응)(구독 제작자 출처의 자동 결정), [할 일](../specs/jobs.md#할-일)
- 막는 티켓: [0035](0035-candidate-observation.md), [0037](0037-candidate-section.md)(받기와 제작자 지정 자리), [0038](0038-receive-result-tistory.md)(실제 수신 경로 하나)

## 작업

- 새 후보 관찰이 구독 제작자의 아직 받지 않은 회차이고, 그 구독 규칙의 `영상 받기`와 `자막 받기`가 모두 켜져 있으면 선택 없이 자막 작업을 만들어요. 인증이 필요하면 `인증 필요`로 멈춰요.
- 구독 제작자의 자막이 이미 있는 회차의 수정 후보(같은 게시물의 갱신일만 바뀜, 또는 같은 회차의 새 게시물)도 같은 조건에서 선택 없이 받아요(사용자 결정, 2026-10-02). 작업은 기존 자막을 바꾸는 수정본임을 기록하고, 받은 뒤의 비교와 `교체 승인`은 결과 목표 4예요. 그전까지 현재 자막은 바뀌지 않아요.
- 다른 제작자의 후보는 자동으로 받지 않고 회차 줄에 조용히 보여요. 이전 제작자의 자막이 있는 회차에 나타난 새 구독 제작자의 후보도 수정 후보가 아니라서 자동으로 받지 않아요.
- 구독 제작자가 정해지지 않은 작품에 후보가 처음 나타나면 작품마다 `자막 구독` 할 일 하나를 만들어요. 카드에는 `1–4화`·`자막 후보 1명`처럼 회차와 후보 수만 두고, `제작자 지정`은 작품 상세의 자막 후보 구역으로 이어져요. 제작자를 정하면 받지 않은 회차부터 자동으로 받아요.
- 구독 제작자 출처의 회차 대응은 Anissia 회차와 시즌 회차의 근거(AniList 회차 수와 규칙의 회차 변환)가 맞으면 묻지 않고 기본 대응을 정해 `자동`과 근거를 남겨요. 근거가 없거나 어긋나면 정하지 않아요(`회차 확인 필요`를 묻는 화면은 결과 목표 4).
- 제작자를 바꾸면 이후 받지 않은 회차부터 새 제작자를 따르고, 이미 받은 자막을 바꾸지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 구독 제작자의 5화 후보가 나타나고 5화 자막이 없음(가짜 Anissia·가짜 출처) | 선택 없이 작업이 생기고 받기까지 가요. 인증이 필요한 출처면 `인증 필요` 할 일이 생겨요. |
| 구독 제작자의 3화 수정본, 같은 제작자의 3화 자막이 이미 있음 | 선택 없이 수정본 작업이 생겨 받기까지 가고, 현재 3화 자막은 그대로예요. |
| 같은 3화 수정본을 두 번 관찰 / 갱신일이 다시 바뀜 | 첫 것은 작업 하나만 생기고, 다시 바뀐 갱신일은 새 수정본 작업이 돼요. |
| 다른 제작자의 3화 후보, 3화 자막은 구독 제작자의 것 | 자동으로 받지 않아요. |
| 다른 제작자의 6화 후보만 있음 | 할 일이 생기지 않고 회차 줄에 조용히 보여요. |
| 제작자 미정 작품에 1–4화 후보 2명이 처음 나타남 | `자막 구독` 할 일 하나가 생기고 카드에 이름이 없어요. 제작자를 정하면 받지 않은 회차의 작업이 생겨요. 메뉴 배지에는 세지 않아요. |
| `영상 받기`가 꺼졌거나 보관된 규칙, `자막 받기`가 `받지 않음`인 구독 | 자동 수신도 `자막 구독` 할 일도 없어요. 자막 후보 구역에서 골라 받기는 할 수 있어요. |
| 자막 13화가 영상 `S02E01`인 근거가 맞는 구독 / 근거가 어긋나는 구독 | 앞의 것은 `자동` 대응과 근거가 남고, 뒤의 것은 대응을 정하지 않아요. |
| 같은 후보를 두 번 관찰하거나 worker가 재시작 | 같은 회차의 작업이 두 번 생기지 않아요. |

## 결과

### 만든 것 (2026-10-03)

- **자동 수신**: 규칙이 `active`이고, `영상 받기`가 켜져 있고, `자막 받기`가 `받지 않음`이 아니고, 시즌의 Anissia 연결이 구독한 작품과 같은 구독만 살펴요. 구독 제작자의 받지 않은 회차와 수정본(같은 게시물의 새 갱신일, 받은 회차의 새 게시물)을 선택 없이 작업으로 만들어요. 작업은 `auto:<관찰 ID>` 명령 ID, `자동` 출처, 수정본이면 `revision_of`(이전에 받은 관찰)를 가져요. 수정본은 받기만 하고 현재 자막은 그대로예요.
- **받지 않는 경우**: 다른 제작자의 회차, 이전 제작자의 자막이 있는 회차의 새 제작자 후보, 다른 출처의 실패하지 않은 작업이 받고 있는 회차, 폴더가 없는(`missing`) 작품, 회차 대응이 미정인 출처의 새 회차예요. 실패한 자동 작업은 다시 만들지 않고 자막 후보 구역에서 골라 받아요.
- **회차 대응(마이그레이션 37, `subtitle_episode_mappings`)**: 작품·시즌·출처마다 하나예요. 구독 제작자의 자연수 회차가 모두 AniList 회차 수 N 안이면 차이 0, 아니면 규칙의 회차 변환이 모두 1–N으로 옮길 때 그 차이, 그 밖은 미정이에요. 근거나 까닭을 함께 남기고, 사용자가 정한 대응은 덮어쓰지 않아요. 작품이 합쳐지면 대응도 남는 작품으로 옮겨요.
- **`자막 구독` 할 일**: 제작자가 없는 구독의 작품마다 하나이고, 회차(`1–4화`)와 후보 수만 보이며 메뉴 배지에 세지 않아요(`GET /api/todo/subtitle-follow`). `제작자 지정`은 작품 상세의 자막 후보 구역으로 가요. 그곳에서 `구독 제작자로 정하기`·`제작자 변경`을 할 수 있어요.
- **살피는 때**: 새 관찰이 생길 때, 작업 실행의 시작과 끝, 시즌이 연결되거나 AniList 항목이 저장될 때, web에서 제작자·구독·시즌 연결·규칙 스위치를 바꾼 뒤예요. 한 구독이 실패해도 다른 구독은 계속 살펴요.
- **화면**: 작업 목록·상세에 `자동`·`수정본` 표시와 이전 수신 작업으로 가는 링크, 자막 후보 묶음에 `자동 · <근거>`·`회차 대응 미정 · <까닭>`이 보여요. 후보 표의 수정 표시도 작업과 같은 회차 키로 비교해요.
- 규칙 전체는 [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)과 [자막의 회차 대응](../specs/library.md#자막의-회차-대응)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 구독 제작자의 5화 후보, 5화 자막 없음 | `a_new_episode_of_the_subscribed_creator_is_received_without_a_pick`, worker 끝까지 `a_reading_that_finds_the_subscribed_creators_new_episode_makes_and_runs_its_job`. 인증이 필요한 출처는 `a_post_that_asks_for_a_check_stops_at_auth`와 web의 `인증 필요` 할 일 시험 |
| 같은 제작자의 3화 수정본 | `a_revision_of_a_received_episode_is_received_and_the_subtitle_in_place_stays` |
| 같은 수정본 두 번 / 갱신일이 다시 바뀜 | `the_same_line_twice_or_a_restart_makes_one_job_and_a_new_update_time_another` |
| 다른 제작자의 3화·6화 후보 | `another_creators_episode_is_never_received_by_itself`, `a_new_creators_episode_where_the_earlier_ones_subtitle_was_received_is_no_revision`, `another_creators_job_that_has_not_failed_holds_the_episode` |
| 제작자 미정, 1–4화 후보 2명 | `an_undecided_work_with_candidates_is_one_suggestion_until_a_creator_is_chosen`, web의 `a_work_without_a_creator_is_one_suggestion_with_no_names_and_no_badge` |
| 꺼졌거나 보관된 규칙, `받지 않음` | `a_paused_or_archived_rule_or_no_subtitles_receives_and_suggests_nothing`, web의 `turning_subtitles_off_takes_the_work_out_of_the_suggestions` |
| 13화 → `S02E01` 근거가 맞음 / 어긋남 | `grounds_that_agree_map_episode_13_to_s02e01_and_grounds_that_do_not_decide_nothing`, `mapping.rs` 단위 시험, `a_mapping_the_user_set_is_kept` |
| 두 번 관찰·worker 재시작 | 위 갱신일 시험(새 `Follow`로 다시 살핌), DB 마이그레이션 시험 `a_database_from_before_auto_receipts_keeps_its_jobs_and_maps_no_source_yet` |

리뷰 수정의 시험(`a_work_whose_folder_is_gone_receives_nothing_until_it_is_back`, `a_subscription_that_cannot_be_looked_at_does_not_stop_the_others`, `connecting_a_followed_subscription_to_its_season_makes_the_creators_jobs_in_that_cycle`, `a_season_entry_stored_later_has_the_worker_look_at_the_subscribed_creator_again`, `saving_the_links_of_a_subscribed_creators_season_makes_the_creators_jobs`, `an_archive_move_that_merges_two_works_keeps_the_subtitle_sources_mappings`, `the_same_episode_written_another_way_is_a_revision_and_a_half_episode_is_not`)은 수정을 빼면 실패하는 것을 확인했어요. 작업 공간 시험 1,742개가 통과하고(실제 네트워크 시험 3개는 무시), clippy 경고가 없고, 웹 빌드가 돼요.

**개발 환경(리뷰 수정 전 빌드)**: FX 전사 쿠루미(코코렛 구독)의 시즌 1은 AniList 항목(206401, 방영 중)의 회차 수가 `null`이라 대응이 `미정 · 시즌 1의 AniList 회차 수를 몰라서 정하지 않았어요`로 기록되고 작품 상세에도 그렇게 보였어요. 대응을 잠시 사용자 대응(차이 0)으로 바꾸고 worker를 다시 띄워도, 코코렛의 유일한 관찰(1화)은 이미 받은 것이라 작업이 생기지 않았어요(중복 없음). 대응은 미정으로 되돌렸어요. 실제 새 관찰로 자동 작업이 생기는 것은 개발 환경에서 보지 못했고, 가짜 Anissia·가짜 출처 시험으로만 확인했어요. `자막 구독` 제안은 제작자 미정 구독이 없어 비어 있었어요.

### 독립 리뷰

마이그레이션, web·worker의 동시 평가, 결정 규칙, 신뢰 경계를 두고 독립 리뷰를 받았어요. 막는 결함은 없었어요. 마이그레이션은 더하기만 하고, 같은 관찰의 작업은 UNIQUE 명령 ID와 IMMEDIATE 트랜잭션으로 하나만 생기며, 새 응답에 서명 주소·토큰이 없고, 정할 수 있는 제작자는 그 작품의 Anissia 줄에 나온 이름뿐인 것이 확인됐어요.

| 지적 | 처리 |
| --- | --- |
| 시즌 연결이나 AniList 회차 수가 나중에 생겨도 다시 살피지 않음 | 시즌 연결·항목 저장·web의 시즌 연결 저장 뒤에 살펴요 |
| 폴더가 없는 작품을 자막이 하나도 없는 것으로 봐서 다른 제작자 자막이 있는 회차까지 받음 | `missing` 작품은 받지 않아요 |
| 대응 저장이 deferred 트랜잭션이라 web·worker 경합에서 실패하고 남은 구독을 건너뜀 | IMMEDIATE, `user` 행은 SQL에서도 막고, 한 구독의 실패는 기록하고 계속해요 |
| 다른 출처의 진행 중 작업이 받는 회차를 또 받음 | 실패하지 않은 작업이면 그 회차를 받는 중으로 봐요 |
| 후보 표의 수정 표시는 회차 글자, 자동 수신은 회차 키로 비교 | 같은 키로 비교해요 |
| 작품이 합쳐지면 대응이 사라짐 | 남는 작품으로 옮기고 사용자 대응을 우선해요 |
| 브라우저 요청 ID가 `auto:`를 차지하면 자동 작업이 조용히 안 생김 | `auto:`로 시작하는 요청 ID는 400이에요 |

### 남은 한계와 열린 결정

- **열린 결정 — 회차 수를 모르는 방영 중 작품**: 실제 FX 작품처럼 방영 중인 AniList 항목은 회차 수가 비어 있을 수 있어요. 지금 규칙에서는 그동안 대응이 미정이라 자동 수신이 일어나지 않아요. 자동 수신의 주 대상이 방영작이라 사용자 결정이 필요해요.
- **열린 결정 — 근거의 우선순위**: 차이 0과 규칙의 회차 변환이 모두 맞으면 차이 0을 써요. 누적 번호의 2쿨 시즌(N=13, 규칙 −12)에서 13화만 보였을 때 13화를 `S02E13`으로 보고, 14화가 보이면 −12로 바뀌어요. 자동 대응은 살필 때마다 다시 정해서, 이미 만든 작업 뒤에 대응이 바뀌어도 다시 평가하지 않아요. 터무니없는 회차 하나(예: `113`)가 보이면 사용자 대응 화면(결과 목표 4)이 생기기 전까지 그 출처는 계속 미정이에요.
- 두 평가가 동시에 같은 회차의 서로 다른 관찰(옛 게시물·새 게시물)을 보면 작업이 둘 생길 수 있어요. 창이 작아 두었어요.
- 같은 Anissia 작품을 두 규칙이 구독하면, 제작자가 다르면 둘 다 받고 작품·시즌이 다르면 두 번째는 조용히 만들지 않아요. 명세가 다루지 않아요.
- 실패한 사용자 선택 작업의 관찰도 자동 작업이 한 번 다시 받아요.
- `인증 필요` 할 일 카드에는 `자동` 표시가 없어요. 작업 목록과 상세에만 있어요.
- `seasons_anissia_api`의 연결 저장 뒤 살피기는 구독이 잡은 시즌을 그 API가 거절해서 지금은 작업을 만들 수 없어요.

### 커밋

- `83b68ab` feat(jobs): auto-receive the subscribed creator's new episodes and revisions
