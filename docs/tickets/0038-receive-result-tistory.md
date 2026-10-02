# 0038 공통 수신 결과를 정하고 Tistory 일반 첨부를 받아요

- 상태: 완료
- 출처: [출처별 다운로드](../specs/subtitles.md#출처별-다운로드)(Tistory 일반 첨부), [작업 결과](../specs/jobs.md#작업-결과), [웹 앱 공통 명세의 구현 순서](../specs/web-app.md#구현-순서)(공통 결과·실패 표현을 정한 뒤 출처 병행)
- 막는 티켓: [0033](0033-source-paths-spike.md)(실패 분류와 단계 목록 초안), [0036](0036-subtitle-jobs-foundation.md)

## 작업

여섯 경로가 함께 쓸 수신 결과와 실패 표현을 정하고, 2026-09-26 표본에서 사람의 조작 없이 받을 수 있었던 Tistory 일반 첨부로 첫 경로를 끝까지 이어요.

- 출처 하나의 수신은 작업에 `받음`(수신 영역의 파일과 크기·SHA-256), `인증 필요`, 또는 0033의 실패 분류 가운데 하나를 돌려줘요. 만료·오류 HTML처럼 자막 묶음이 아닌 응답은 받음으로 기록하지 않아요.
- 출처별 작업 단계(`게시물 열기`·`받기` 등)는 0033의 단계 목록을 따르고 출처마다 다를 수 있어요.
- Tistory 게시물에서 일반 첨부의 URL을 얻고 필요하면 다시 얻어 받아요. 압축 내부 처리는 결과 목표 4의 묶음 분석이에요.
- 이후 출처 티켓(0041–0044)은 이 결과 형태에 경로만 더해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| Tistory 일반 첨부 게시물(가짜 출처) | 작업이 `게시물 열기` → `받기`를 거쳐 수신 영역에 ZIP이 있고 크기·SHA-256이 기록돼요. |
| 첨부 대신 오류 HTML이 옴(가짜 출처) | 받음이 아니라 실패로 끝나고, 분류와 이유가 작업 상세에 있어요. |
| 첨부 URL이 만료됨(가짜 출처) | 게시물에서 다시 얻어 받거나, 다시 얻지 못하면 만료로 실패해요. |
| 실제 Tistory 게시물 하나(Anissia 최근 자막에서 고름) | 실제 서버에서 받은 파일의 크기와 형식을 확인한 결과가 있어요. |
| 공통 결과 | 여섯 경로가 쓸 결과·실패 분류가 작업과 인증 명세에 있고, 작업 상세와 할 일이 그것으로 표시돼요. |

## 결과

### 만든 것 (2026-10-03)

- **공통 수신 결과**: 출처에서 파일 하나를 받으려는 시도는 `받음`(형식 ZIP·ASS·SRT·SMI·그 밖, 크기, SHA-256, 파일 정보 스냅샷)이나 실패 분류 하나로 끝나요. 실패 분류는 `원본 없음`·`만료`·`파일 아님`·`출처 구조 바뀜`·`네트워크 실패`이고, 응답이 있었으면 HTTP 상태·`Content-Type`·응답 크기를 함께 남겨요. 규칙 전체는 [공통 수신 결과와 실패 분류](../specs/jobs.md#공통-수신-결과와-실패-분류)에 있어요.
- **받은 바이트 검사**: 공개하기 전에(재시작 복구 포함) 빈 파일, 웹 페이지, CRC가 맞지 않는 ZIP, 이름이 약속한 형식과 다른 바이트를 `파일 아님`으로 보고 지워요. ZIP 안의 멤버가 자막인지는 보지 않아요. 한 게시물의 폰트 ZIP도 정상 파일이고, 멤버를 가르는 것은 결과 목표 4의 분석이에요. 검사 자체가 죽으면 지우지 않고 보류해요.
- **다시 시도**: `네트워크 실패`는 같은 실행 안에서 2초·10초 뒤 두 번 더 시도해요. `만료`는 게시물을 한 번 다시 읽어 새 주소로 받아요. 다시 읽은 게시물에 그 파일이 없으면 `원본 없음`, 또 거절되면 `만료`예요.
- **Tistory 일반 첨부**: `*.tistory.com` 게시물의 `.fileblock` 첨부를 모두 받아요. 쿠키와 Referer 없이, https의 Tistory CDN(`*.kakaocdn.net`·`*.daumcdn.net`)에서만 받고, 리다이렉트도 그 호스트로만 따라가요. 파일 하나는 200MiB, 15분까지예요. 같은 호스트에는 1초 간격으로 요청해요.
- **다른 곳에 있는 자막**: 첨부가 없고 본문에 Google Drive 링크나 WinPNG 이미지가 있으면 실패가 아니라 `자막 대기`로 그 까닭을 적고 기다려요. 본문은 실제 스킨에서 본 `.tt_article_useless_p_margin`·`.contents_style`·`#article-view`로 찾아요.
- **비밀값**: 서명 주소는 게시물을 연 때부터 파일을 받을 때까지 메모리에만 있어요. 기록·로그·API 응답·`Debug` 출력에 남지 않아요.
- **저장(마이그레이션 36)**: 작업 파일에 형식·실패 분류·HTTP 상태·`Content-Type`·응답 크기·스냅샷(JSON), 작업 회차에 실패 분류를 더했어요.
- **화면**: 작업 상세의 파일 줄에 `11 KB · ZIP`, 실패한 파일·회차·작업 머리·작업 목록에 분류 표시(`원본 없음` 등)와 이유, HTTP 상태가 보여요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| Tistory 일반 첨부(가짜 출처) | Tistory 모양의 로컬 서버로 `a_tistory_post_goes_through_open_and_receive_and_leaves_its_zip_with_size_and_sha256`. `게시물 열기` → `받기`를 거쳐 수신 영역에 ZIP이 있고 크기·SHA-256이 기록돼요. |
| 첨부 대신 오류 HTML | `an_error_page_instead_of_the_attachment_fails_as_not_a_file_with_its_reason`. 개발 환경에서 가짜 출처의 `/missing/` 게시물은 작업 상세에 `원본 없음 · 게시물이 없어요 (404)`로 보였어요. |
| 첨부 URL 만료 | `an_expired_address_is_read_from_the_post_again_and_received`, `an_address_refused_again_after_reading_the_post_fails_as_expired`, `a_file_gone_from_the_post_read_again_fails_as_missing` |
| 실제 Tistory 게시물 하나 | 개발 환경의 worker가 실제 후보(스모모 491, 담배 고양이 12화)로 작업을 받았어요: `Yani Neko - 12.zip` 11,724바이트(0033 기록과 같음), ZIP, CRC 통과, 안에 `Yani Neko - 12.srt` 31,792바이트, SHA-256 `1470019a…6fec`, HTTP 200 `application/octet-stream`. 리뷰 수정 뒤 다시 빌드해 같은 바이트를 받았어요. 무시해 둔 `a_real_tistory_post_is_received`로 492도 받았어요(11,379바이트). 이 확인을 위해 개발 데이터의 시즌 하나를 3440에 잠시 연결했다가 되돌렸어요. |
| 공통 결과 | 명세의 [공통 수신 결과와 실패 분류](../specs/jobs.md#공통-수신-결과와-실패-분류), `a_tistory_receipt_shows_its_format_and_failures_by_class_and_no_signed_address`, 개발 환경의 작업 상세·작업 목록 화면이에요. |

그 밖에 실제 Drive 게시물(코코렛 felia 1187, FX 전사 쿠루미 1화)이 `자막 대기`(`자막이 Google Drive 링크로 올라와 있어요…`)로 기다리는 것을 개발 환경에서 봤어요. 시험으로는 `a_missing_post_fails_as_missing_and_a_drive_post_waits_for_a_source`, `every_file_of_a_post_is_received_once_for_the_episodes_it_serves`, `a_network_failure_is_tried_again_twice_then_fails_as_network`, `a_shutdown_during_a_retry_wait_stops_the_run_at_once`, `no_signed_address_reaches_the_records_or_the_log`, `a_file_past_its_byte_limit_fails_as_not_a_file_and_leaves_no_bytes`, `recovered_bytes_that_are_a_web_page_fail_as_not_a_file_and_are_received_anew`, `a_failure_recorded_before_its_bytes_went_is_finished_by_the_next_start`, `a_failure_whose_bytes_were_left_is_finished_even_when_the_post_is_gone`, `bytes_that_cannot_be_removed_hold_the_receipt_instead_of_counting_as_gone`, 리다이렉트의 `a_redirect_is_followed_on_the_cdn_with_no_referer_and_stopped_elsewhere`, 마이그레이션 `a_database_from_before_receive_results_keeps_its_receipts_and_checks_the_new_columns`가 있어요. 개발 환경의 작업 표 다섯 개와 웹·worker 로그에 `signature=`·`credential=`이 없었어요. 작업 공간 시험 1,660개가 통과하고(실제 네트워크 시험 1개는 무시), clippy 경고가 없고, 웹 빌드가 돼요.

### 독립 리뷰

복구·체크포인트, 마이그레이션 36, 비밀값, Tistory 출처, 바이트 검사를 두고 독립 리뷰를 받았고, 고친 뒤 지적마다 다시 확인받았어요. 막는 결함은 없었고 0036의 복구 보장(이 시도의 바이트만 지움, 검사 실패가 공개 파일·완료 기록을 남기지 않음, 다시 시도가 겹치지 않음)은 지켜졌어요.

| 지적 | 처리 |
| --- | --- |
| 리다이렉트 때 reqwest가 이전 주소 전체를 `Referer`로 보내 서명이 새어 나감 | 자동 Referer를 끄고 리다이렉트는 허용 호스트로만 따라가요 |
| 게시물의 아무 주소(LAN 포함)로 요청하고, 크기·시간 한도가 없음 | https Tistory CDN만, 파일 하나 200MiB·15분 |
| WinPNG를 페이지 전체 이미지에서 찾음(스킨 이미지에 속음) | 본문 안에서만 찾고, 본문이 없으면 `출처 구조 바뀜` |
| 바이트를 지운 뒤 실패를 기록해 그 사이 멈추면 보류가 됨 | 실패를 먼저 기록하고 지운 뒤 경로를 비워요. 다음 시작이 남은 바이트를 마저 지워요. 재확인에서 게시물이 사라지면 이 정리가 빠지는 틈이 나와, 게시물을 열기 전에도 정리하게 했어요 |
| 지우기 실패를 무시함 | 지운 것이 확인될 때만 지워진 것으로 보고, 아니면 보류해요 |
| 조작된 ZIP 끝 레코드가 큰 할당을 일으킬 수 있음 | 멤버 수를 먼저 읽어 거절하고 파일 크기를 200MiB로 묶어요. zip 크레이트가 더 넓게 찾는 경우의 작은 틈은 남았어요(아래) |
| 큰 길이가 음수로 저장돼 CHECK를 깨뜨림 | 맞지 않는 크기는 저장하지 않아요 |
| 압축된 응답을 그대로 파일로 저장할 수 있음 | `identity`가 아닌 `Content-Encoding`은 `파일 아님` |
| 검사기가 죽으면 바이트를 지우고 분류 없이 실패함 | 보류해요 |
| 작업의 실패 분류가 첫 실패 회차와 다를 수 있음 | 첫 실패 회차의 분류를 따라요 |
| WebVTT를 SRT로 판별 | `그 밖`이에요 |

### 남은 한계

- **WinPNG와 Google Drive는 `자막 대기`예요.** 받는 것은 [0043](0043-source-tistory-winpng.md)과 [0042](0042-source-blogger-drive.md)예요. worker가 시작할 때마다 이런 작업의 게시물을 한 번 다시 읽어요.
- **한 게시물의 첨부를 모두 받아요.** 시리즈 전체 ZIP과 회차별 SMI 24개가 함께 있는 게시물(isulbi 40)에서는 회차 하나를 골라도 25개를 받아요. 어느 파일이 그 회차인지는 결과 목표 4의 분석이에요.
- `*.tistory.com` 주소가 사용자 도메인으로 리다이렉트되는 블로그는 `출처 구조 바뀜`이에요. 사용자 도메인의 Tistory 블로그는 주소로 알아보지 못해 `자막 대기`예요.
- 실제 CDN이 서명 주소를 리다이렉트하는지, 월 말 뒤의 실제 만료 응답은 보지 못했어요. 만료 처리는 서명을 바꾼 주소의 404 `text/html`(150바이트, 0033 때는 552바이트)로만 확인했어요. 응답 크기는 분류 신호로 쓰지 않아요.
- 15분 한도는 `네트워크 실패`라 두 번 더 시도해요. 파일 하나가 최악에는 45분쯤 걸려요.
- 이름이 `.srt`인 WebVTT 파일은 `파일 아님`이에요(이름이 약속한 형식과 다름).
- zip 크레이트는 끝 레코드를 파일 전체에서 찾아서, 일부러 숨긴 멤버 수에는 할당이 1GB 안팎까지 커질 수 있어요(200MiB 상한 안의 악의적 파일만).
- 검사기가 죽는 경우의 보류는 시험이 없어요(죽게 만들 입력이 없어요).
- 서버 컨테이너 밖 휴대폰·VPN 경로는 보지 않았어요.

### 커밋

- `6187424` feat(jobs): settle the common receive result and receive Tistory attachments
