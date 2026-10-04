# 0035 Anissia 최근 자막 목록을 30분마다 읽어 자막 후보를 쌓아요

- 상태: 완료 (아래 "결과")
- 출처: [자막 후보 조회](../../../specs/subtitles.md#자막-후보-조회), [지난 회차 ADR](../../../adr/0012-past-subtitles-find-and-upload.md), [자막 출처 연결](../../../glossary.md)
- 막는 티켓: [0034](0034-season-anissia-link.md)(시즌별 후보 조회와 연결 시 읽기)

## 작업

Anissia 자막 정보는 작품마다 제작자 한 명당 마지막 회차 표시·게시물 주소·갱신일 한 줄뿐이고, 제작자가 새 회차를 올리거나 고치면 그 줄을 덮어써요(2026-10-02 확인).
trss는 줄이 바뀔 때마다 관찰로 남겨 회차별 후보 기록을 쌓아요.

- worker가 화면과 상관없이 30분마다 `/anime/caption/recent/<page>`를 빈 쪽이 나올 때까지 모두 읽고, 직전 관찰과 회차 표시·주소·`updDt` 중 하나라도 다른 줄을 모든 Anissia 작품에 대해 관찰로 남겨요(사용자 결정, 2026-10-02). RSS 수집 주기와 별개이며, 기존 Anissia 큐처럼 자기 잠금과 요청 간격을 써요.
- 시즌을 연결할 때와 사용자의 새로고침(웹 명령)에는 그 작품의 `/anime/caption/animeNo/<n>`을 바로 읽어 같은 방식으로 남겨요.
- 관찰은 Anissia 작품·제작자·게시물 주소·회차 표시(원래 문자열)·`updDt`(시간대 없는 값은 `Asia/Seoul`)·처음 본 시각이에요. 제작자와 게시 경로의 출처 맥락은 앱 ID를 가진 자막 출처 연결로 묶고, 제작자 표시명이나 웹사이트 주소를 전역 ID로 쓰지 않아요.
- 줄이 사라져도 관찰은 지우지 않아요. 날짜를 읽지 못한 줄은 관찰 시각으로 정렬하고 해석 실패를 구분해 남겨요.
- 연결한 시즌의 후보 조회 API는 그 시즌의 Anissia 작품 관찰을 내놓고, 같은 회차에 같은 제작자의 자막이 있으면 수정 후보로 표시해요. 화면은 [0037](0037-candidate-section.md)이에요.
- Anissia 줄이 그대로인 게시물의 파일 변경은 다루지 않아요([0033](0033-source-paths-spike.md)의 확인 뒤 따로 정해요).

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 가짜 Anissia 최근 목록 3쪽, 연결한 작품과 연결 안 한 작품의 줄이 섞임 | 한 주기에 세 쪽과 빈 쪽까지 읽고, 두 작품의 줄이 모두 관찰로 남아요. 30분 안에 다시 읽지 않아요. |
| 다음 주기에 줄이 하나도 바뀌지 않음 | 새 관찰이 생기지 않아요. |
| 같은 제작자의 줄이 `3화 게시물 A`에서 `4화 게시물 B`로 바뀜 | 두 후보가 모두 남아요. |
| 같은 줄의 `updDt`만 바뀜(한 게시물을 고침) | 같은 게시물의 갱신으로 남고 이전 관찰도 남아요. 그 회차에 같은 제작자의 자막이 있으면 수정 후보예요. |
| 줄이 목록에서 사라짐 | 기존 관찰이 그대로예요. |
| 연결하지 않은 작품의 관찰이 쌓인 뒤 그 작품의 시즌을 연결 | 연결 즉시 작품별 자막 정보를 읽고, 이전 관찰까지 그 시즌의 후보로 나와요. |
| `updDt` `2026-10-02 21:00:00`을 서버 시간대 `UTC`와 `Asia/Seoul`에서 읽음 | 두 시간대에서 같은 시점(`2026-10-02T12:00:00Z`)으로 남아요. 잘못된 날짜는 해석 실패로 구분돼요. |
| 회차 표시 `0`·`13.5` | 원래 문자열로 남고 숫자 회차로 바꾸지 않아요. |
| `429`·실패·쪽 중간의 실패 | 다음 주기에 다시 읽고 기존 관찰이 남아요. 이미 읽은 쪽의 관찰은 남아요. |

## 결과

### 만든 것 (2026-10-02)

- **실제 응답 모양(2026-10-02 확인)**: `/anime/caption/recent/<page>`는 0쪽부터 세고 `data.content`·`empty`·`last` 등 쪽 정보를 담아요(한 쪽 20줄, 그날 4쪽 70줄·65작품). 빈 쪽은 `200`에 `content: []`예요. 줄은 `animeNo`·`subject`·`episode`(늘 문자열)·`updDt`(시간대 없는 `YYYY-MM-DDTHH:MM:SS`)·`website`·`name`이에요. `/anime/caption/animeNo/<n>`은 같은 줄의 목록이고 모르는 작품이면 빈 목록이에요. 가짜 Anissia는 이 모양으로 답해요.
- **저장(마이그레이션 33)**: 자막 출처 연결 `subtitle_sources`(앱이 만든 ID, Anissia 작품·제작자 이름으로 찾음), 관찰 `caption_observations`(출처·게시물 주소·회차 표시 원문·`updDt` 원문과 해석한 시각·처음 본 시각), 읽기 일정 `anissia_caption_poll`. 관찰은 지우지 않아요.
- **30분 읽기**: worker 안의 `CaptionObserver`가 RSS 주기와 따로, 자기 잠금(`<DB>.anissia-captions.lock`)과 공유 Anissia 요청 간격으로 빈 쪽까지(최대 200쪽) 읽어요. 다음 읽기 시각은 읽기 전에 저장해서 도중에 죽어도 재시작마다 다시 읽지 않고, 지나치게 먼 일정은 믿지 않아요. 한 출처의 직전 관찰과 회차 표시·주소·`updDt` 시각 중 하나라도 다르면 새 관찰이에요. 한 응답에 같은 작품·제작자 줄이 둘이면 가장 새것만 봐요. 주소가 없거나 http(s)가 아니거나 너무 긴 줄은 건너뛰고, 주기마다 읽은 쪽·새 관찰·건너뛴 줄 수를 로그에 남겨요(주소는 남기지 않아요).
- **바로 읽기**: `anissia_captions` 웹 명령(작품마다 열린 명령 하나)이 그 작품의 줄을 읽어요. 작품 상세에서 시즌을 연결할 때, worker가 구독을 시즌에 이을 때, 사용자의 새로고침(`POST /api/commands`, 연결된 작품만)에 만들어지고 worker를 깨워요. 웹 응답은 Anissia를 기다리지 않아요.
- **후보 API**: `GET /api/library/works/{id}/seasons/{n}/anissia/candidates`가 시즌의 Anissia 작품 관찰을 최근순(`updDt` 시각, 해석 실패면 처음 본 시각)으로 내놓고, 마지막 전체 읽기 시각과 최근 새로고침 명령을 함께 줘요.

커밋: `b2d99e1`, `d758b2c`, `38c3970`, `f623e8c`, `318800b`, `b7309a3`, `97b9833`, `682ac7b`, `cbb807c`.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 3쪽과 빈 쪽, 섞인 작품, 30분 안에 다시 읽지 않음 | `a_reading_takes_every_page_to_the_empty_one_and_observes_every_anime`, `nothing_is_read_again_within_thirty_minutes_even_after_a_restart`, `a_reading_cut_short_is_not_started_over_by_a_restart_before_its_period_is_up` |
| 바뀐 줄 없음 | `a_reading_after_which_no_line_changed_adds_nothing` |
| `3화 게시물 A` → `4화 게시물 B` | `a_creator_moving_to_the_next_episode_leaves_both_candidates` |
| `updDt`만 바뀜 | `an_update_time_that_alone_changes_is_the_same_post_updated_and_marks_a_revision`, `an_update_only_change_is_a_revision_candidate_in_the_answer` |
| 줄이 사라짐 | `a_line_that_leaves_the_list_leaves_its_observations` |
| 관찰이 쌓인 뒤 시즌 연결 | `linking_after_observations_exist_shows_the_earlier_ones_at_once_and_asks_the_worker_to_read`, `linking_a_season_reads_the_anime_in_the_worker_and_the_older_lines_become_candidates`, 구독으로 이을 때 `connecting_a_subscription_to_a_season_has_the_animes_subtitle_lines_read_once` |
| `updDt` 시간대·잘못된 날짜 | `a_value_without_a_zone_is_seoul_time_in_either_spelling`, `what_is_not_a_date_and_time_is_not_an_instant`, `an_update_time_without_a_zone_is_seoul_time_and_an_invalid_one_is_kept_and_marked`. 시간대 없는 값은 서버 시간대를 보지 않고 늘 +09:00으로 읽어서, 서버가 UTC든 Asia/Seoul이든 같은 시점이에요. |
| 회차 `0`·`13.5` | `episodes_zero_and_thirteen_and_a_half_are_kept_as_written` |
| `429`·실패·쪽 중간 실패 | `a_429_ends_the_reading_and_the_next_period_reads_again`, `a_429_that_asks_for_longer_than_the_period_holds_the_next_reading_until_then`, `a_failure_is_read_again_next_period_and_leaves_the_observations_there`, `a_failure_in_the_middle_keeps_the_pages_already_read_and_the_next_period_goes_on`, `refresh_reads_the_anime_again_and_a_failed_read_says_so_and_keeps_the_candidates` |

- 시험 1,529개 → 1,580개, 실패 0. `cargo clippy --workspace --all-targets` 경고 0, `cargo fmt --check` 통과. 웹 화면은 바뀌지 않았어요.
- 실제 Anissia에서는 응답 모양만 읽어 봤고(최근 목록 0–5쪽, 작품 두 개), 개발 환경에서 30분 읽기를 돌려 보지는 않았어요.

### 독립 리뷰

`claude-chan:reviewer`가 마이그레이션, 바뀜 판단, 일정과 잠금, 명령 경로와 0031 dispatcher, 후보 API, 로그의 주소 노출을 봤어요. P0·P1은 없었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| worker가 구독을 시즌에 이을 때는 작품의 줄을 읽지 않고, 명세가 이를 근거 없이 예외로 적음 | 연결이 바뀐 패스마다 같은 명령을 만들고 worker를 깨워요. 명세의 예외를 되돌렸어요 |
| 일정을 읽은 뒤에 저장해 도중에 죽으면 재시작마다 다시 읽음 | 읽기 전에 저장해요 |
| 한 응답의 같은 작품·제작자 두 줄이 주기마다 관찰을 쌓을 수 있음 | 가장 새 줄만 봐요 |
| 건너뛴 줄 수가 어디에도 안 보임 | 주기마다 한 줄 로그를 남겨요 |
| 아무것도 증명하지 못하는 `TZ` 시험, 시계가 뒤로 가면 먼 일정에서 멈춤 | 시험을 지우고 먼 일정은 믿지 않아요 |
| 수정 후보 표시가 확정 판단처럼 보임 | API 설명과 명세에 잠정 기준이라고 적었어요 |

### 남은 한계

- **수정 후보는 잠정 기준이에요.** 받은 자막 기록이 아직 없어서 "같은 출처에서 같은 회차 표시를 전에 본 적 있음"으로 판단해요. 티켓의 "그 회차에 같은 제작자의 자막이 있으면"은 받은 자막을 기록하는 [0036](0036-subtitle-jobs-foundation.md) 이후 `revision_of`에서 바꿔요. → [0037](0037-candidate-section.md)에서 받은 자막 기록으로 바꿨어요.
- **자막 출처 연결은 (Anissia 작품, 제작자 이름)마다 하나예요.** 연결하지 않은 작품도 관찰해야 해서 시즌이 아니라 작품 단위예요. 시즌별 연결과 회차 대응은 뒤 티켓(0036·0045·0047)이 이 ID에 붙여요. 제작자 이름의 대소문자·공백이 다르면 다른 출처로 남아요.
- 연결할 때 읽기가 Anissia의 긴 `429` 대기(30초 넘게)에 걸리면 실패로 끝나고 저절로 다시 하지 않아요. 새로고침이나 30분 읽기(최근 목록에 있는 줄만)가 대신해요.
- 30분 읽기와 새로고침 명령이 같은 작품을 몇 ms 차이로 엇갈려 읽으면 예전 상태가 한 번 더 관찰로 남을 수 있어요(데이터가 깨지지는 않아요).
- 주소가 없는 줄(첫 등록의 `0`화 등)은 관찰하지 않아요.
- Anissia 줄이 그대로인 게시물의 파일 변경은 다루지 않아요(티켓 범위 밖).
