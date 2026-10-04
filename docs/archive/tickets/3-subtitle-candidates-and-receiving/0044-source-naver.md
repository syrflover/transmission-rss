# 0044 Naver 블로그 첨부를 받아요

- 상태: 완료
- 출처: [출처별 다운로드](../../../specs/subtitles.md#출처별-다운로드)(Naver 블로그 첨부), [Naver 블로그 첨부 조사](../../../brainstorm/web-gui-subtitles.md#naver-블로그-첨부)
- 막는 티켓: [0038](0038-receive-result-tistory.md)

## 작업

Naver 블로그 게시물(내부 프레임 포함)에서 유효한 첨부 경로를 얻어 worker가 HTTP로 실제 파일을 받아요(사용자 결정, 2026-10-02).
0033에서 쿠키·Referer 없이 받아졌고, 첨부 주소의 토큰은 게시물을 읽을 때마다 바뀌었어요. 같은 게시물에 글꼴(TTF)이 함께 올라올 수 있어요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 첨부가 내부 프레임에 있는 게시물(가짜 출처) | 첨부 경로를 얻어 파일을 받아요. |
| 첨부 대신 안부게시판 CAPTCHA·오류 페이지 | 첨부 다운로드와 섞지 않고 분류된 실패예요. |
| 실제 게시물 하나 | 구현한 경로로 실제 서버에서 받은 파일의 크기·형식(예: UTF-16LE SAMI)을 확인한 결과가 있어요. |

## 결과

### 만든 것 (2026-10-03)

- **Naver 출처**: `blog.naver.com`·`m.blog.naver.com`의 글 주소(`/<blogId>/<logNo>`, `PostView.naver`·`.nhn`의 `?blogId=…&logNo=…`, `?Redirect=Log&logNo=…`)에서 내부 페이지 주소를 만들어 바로 읽어요. 첨부 목록은 본문 HTML이 아니라 `aPostFiles[n] = JSON.parse('…')` 대입만 읽고, `aPostBaseInfo`로 이 글(logNo)의 목록만 골라요. 키는 토큰 없는 `naver:<blogId>/<logNo>/<이름>`이에요.
- **받기**: 쿠키·Referer 없는 GET, `download.blog.naver.com`으로만 리디렉션, 200MiB·15분 상한이에요. 서명 주소가 거절되면 내부 페이지를 한 번 다시 읽어 새 주소로 한 번만 받아요. `Content-Length`가 `attachFileSize`와 다르면 `파일 아님`이에요.
- **분류**: 삭제된 글의 알림(`삭제되었거나`)은 `원본 없음`, 그 밖의 알림과 보안 확인(CAPTCHA)·다른 페이지는 `출처 구조 바뀜`이에요. Naver가 악성 코드(`maliciousCodeYn`)나 이용 제한(`punishType`)으로 표시한 첨부는 요청하지 않고 `원본 없음`이며, 표시 값은 알 수 없으면 표시된 것으로 봐요.
- **회차 고르기**: 0042의 규칙을 첨부 파일 이름에 써요. 폰트 확장자(`.ttf`·`.otf`·`.ttc`·`.woff`·`.woff2`)는 이름과 상관없이 폰트이고, `S01E08`은 8화, `S02E01-E12`·`E01-E12`·`ep01-ep12`는 범위예요. 첨부가 없는 글은 본문의 Drive 링크를 받아요.
- **스냅샷**: 이 글의 날짜(`publish_date`, 절대 날짜일 때만)와 `attach_file_size`, 표시된 제한(`blocked`)을 남겨요. 하루가 안 된 글은 날짜가 없어서, 스냅샷 비교(0050)는 날짜가 나중에 생기는 것을 변경으로 보지 않아야 한다고 명세에 적었어요.
- `get_file`이 Tistory에서 `http.rs`로 옮겨 세 출처가 함께 써요. worker가 Naver 출처를 켜요.
- 상세 규칙은 [공통 수신 결과와 실패 분류](../../../specs/jobs.md#공통-수신-결과와-실패-분류)에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 첨부가 내부 프레임에 있는 게시물(가짜 출처) | `an_attachment_inside_the_inner_frame_is_received_with_the_posts_fonts`, `a_zip_of_a_range_and_the_episodes_file_are_both_received`, `a_refused_address_is_read_again_once_and_received` |
| 첨부 대신 안부게시판 CAPTCHA·오류 페이지 | `a_captcha_or_a_page_of_no_post_is_a_failure_with_no_file_asked_for`(파일 요청 없음), 단위 시험 `a_page_that_is_not_the_post_is_a_classified_failure`(삭제 알림, 다른 알림, 선언만 있는 CAPTCHA 페이지) |
| 실제 게시물 하나 | 개발 환경의 worker가 실제 후보 45(공룡이 8화, `Kimi ga Shinu made Koi wo Shitai`)로 `네죽사 1~8화 자막.zip` 106,408바이트(ZIP, SHA-256 `2468e660…`)와 8화 ASS 44,480바이트(SHA-256 `5f3ff574…`)를 받았어요. 작업 `ce915eff-e7d5-4eae-85fc-5ff0276042b5`예요. 개발 DB의 Naver 줄 일곱 개의 실제 내부 페이지를 저장해 `the_real_posts_sampled_choose_their_files`로 고정했어요. 무시해 둔 `a_real_naver_post_is_received`도 있어요. |

그 밖에 `no_download_address_or_token_reaches_the_records`(작업 기록에 서명 주소·토큰 없음), `only_the_posts_own_list_is_read`, `other_mentions_of_the_list_are_not_its_assignment`, `a_page_of_another_post_or_of_lists_of_no_post_has_changed`, `the_flags_fail_closed_and_are_kept_short`, `only_the_posts_own_absolute_date_is_its_publish_date`, `a_flagged_file_is_offered_without_an_address_and_never_fetched`, `an_address_off_the_file_host_is_not_kept`가 있어요. 리뷰 수정 뒤 작업 공간 시험 1,714개가 두 번 통과했고(실제 네트워크 시험 3개는 무시), clippy 경고가 없고, 웹 빌드가 돼요. 리뷰 수정 뒤 실제 다운로드는 다시 하지 않았어요. 저장한 실제 페이지 일곱 개가 수정 전과 같은 파일·크기를 골랐어요.

### 독립 리뷰

서명 토큰, 리디렉션 경계, 첨부 목록 읽기, 실패 분류를 두고 독립 리뷰를 받았어요. 막는 결함은 없었어요.

| 지적 | 처리 |
| --- | --- |
| 대입이 아닌 `aPostFiles[` 언급(`.length` 등)이 페이지 읽기를 멈춤 | 대입만 읽고 나머지는 건너뛰어요 |
| `aPostBaseInfo`가 있는데 이 글과 맞는 것이 없으면 모든 목록을 읽음 | `출처 구조 바뀜`이에요. 기본 정보가 없고 목록이 하나일 때만 그 목록을 읽어요 |
| `maliciousCodeYn`의 모르는 값을 표시 없음으로 봄 | 비었거나 `false`·`n`·`0`일 때만 표시 없음이에요 |
| 맨 선언 `var aPostFiles = []`만으로 게시물로 봄 | 대입이나 기본 정보가 있어야 게시물이고, 아니면 CAPTCHA·알림을 먼저 봐요 |
| 모든 알림을 `원본 없음`으로 봄 | 삭제 알림만 `원본 없음`, 나머지는 `출처 구조 바뀜`이에요 |
| `S02E01-E12`를 12화로 읽음 | 1–12화 범위예요 |
| `punishType` 값을 그대로 스냅샷에 남김 | ASCII 영문·숫자 16자까지예요 |
| 다른 글이나 상대 날짜가 `publish_date`가 됨 | 이 글의 절대 날짜만이에요 |

리뷰 중 한 번 실패한 시험을 찾지 못해, 시각에 기대던 시험 넷을 상태를 기다리도록 바꿨어요(`d740f1e`와 `testing.rs`의 간격 시험).

### 남은 한계

- **보지 못한 Naver 응답**: 비공개 글, 보안 확인 페이지, 악성·제한 표시 첨부, 삭제 알림이 `200`으로 오는 경우는 알려진 표시로만 가려요. 토큰의 최대 유효 시간(20분 이상은 확인)도 몰라요.
- 같은 이름의 첨부가 둘이면 첫 것만 받아요.
- 첨부가 폰트뿐인 글은 본문 Drive 링크로 넘어가지 않고 `출처 구조 바뀜`이에요.
- 띄어 쓴 `EP01 - EP12`는 기존 규칙대로 12화 한 회차로 읽어요.

### 커밋

- `d740f1e` test(jobs): wait for the run's state instead of fixed sleeps before a shutdown
- `7e42c07` feat(subtitles): receive a Naver blog post's attachments for the chosen episode
