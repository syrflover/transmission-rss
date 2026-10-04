# 0042 Blogger·Google Drive의 개별 자막과 회차 ZIP을 받아요

- 상태: 완료
- 출처: [출처별 다운로드](../../../specs/subtitles.md#출처별-다운로드)(Blogger·Google Drive 개별 자막, 회차 ZIP), [Blogger·Google Drive 조사](../../../brainstorm/web-gui-subtitles.md#bloggergoogle-drive)
- 막는 티켓: [0038](0038-receive-result-tistory.md)

## 작업

Blogger 게시물에서 연결된 Google Drive 파일을 worker가 HTTP로 받아요(사용자 결정, 2026-10-02. 0033에서 쿠키·로그인 없이 받아졌어요). 게시물에 개별 회차 파일과 회차 ZIP이 함께 있을 수 있으므로, 후보의 회차와 사용자가 고른 범위에 맞는 파일을 고르는 규칙은 0033의 관찰을 따라요.
회차 ZIP에서 고른 범위만 적용하는 일은 결과 목표 4의 묶음 분석이고, 이 티켓은 두 종류를 각각 수신 영역까지 받는 것이에요.
개별 파일 수신의 성공으로 ZIP 처리를 완료로 보지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 개별 자막 링크가 있는 게시물(가짜 출처) | 고른 회차의 파일만 받아요. |
| 회차 ZIP 링크가 있는 게시물(가짜 출처) | ZIP을 받고, 작업 상세에 묶음으로 받았다고 보여요. |
| Drive가 파일 대신 확인 페이지나 오류 HTML을 줌 | 받음이 아니라 분류된 실패예요. |
| 실제 게시물(개별 하나, ZIP 하나) | 실제 서버에서 받은 두 파일의 크기·형식을 확인한 결과가 있어요. |

## 결과

### 만든 것 (2026-10-03)

- **Blogger 출처**: `*.blogspot.com` 게시물의 본문(`.post-body`, 실제 테마 모두 같음)에서 Google Drive 링크와 그 글자를 읽어요. 게시물 수정 시각은 JSON-LD `dateModified`가 있는 테마에서 스냅샷에 남겨요.
- **Drive 수신**: `drive:<파일 ID>`를 키로, https의 `drive.google.com`·`drive.usercontent.google.com`에서만, 쿠키·Referer 없이 받아요. 실제로는 `uc?export=download`가 `drive.usercontent.google.com/download`로 303 리다이렉트돼요. 파일 이름은 다운로드 응답의 `Content-Disposition`에서 와서, 안전한 이름으로 바꿔 바이트보다 먼저 기록해요. 확인·할당량 페이지는 넘기지 않고 `파일 아님`, 없는 파일(404 `text/html`)·로그인 요구는 `원본 없음`이에요. 폴더 링크만 있으면 `자막 대기`예요. Tistory와 Blogger가 Drive 요청 간격을 함께 써요.
- **Tistory 본문의 Drive 링크**: 0038에서 `자막 대기`였던 게시물(felia)을 같은 Drive 수신으로 받아요. Tistory 첨부 블록은 그대로 모두 받아요.
- **회차 고르기**: 링크 글자로 그 회차의 파일을 골라요. 그 회차를 이름한 링크(`15화`, `고양이와 용 08`, `… 4 24.zip`), 회차가 든 범위(`1 ~ 12화`, `1-3`), 폰트 링크, 회차가 없는 압축 파일, 그리고 회차를 말하지 않는 링크가 하나뿐이면 그 링크를 받아요. 아무것도 맞지 않으면 `출처 구조 바뀜`(`게시물에 24화 파일이 없어요`)이에요. 연도·날짜·코덱 숫자(`H.264`)·1000이 넘는 숫자와 띄어 쓴 `3 - 05`의 앞 숫자는 회차로 읽지 않아요. 비교 기준은 화면의 회차 키와 같아요. 규칙 전체는 [공통 수신 결과와 실패 분류](../../../specs/jobs.md#공통-수신-결과와-실패-분류)에 있어요.
- **화면**: 받은 ZIP은 작업 상세에 `ZIP · 묶음으로 받음`, 이름이 폰트인 ZIP은 `ZIP · 폰트 묶음`으로 보여요.
- 원격 파일 이름에서 보이지 않는 서식 문자(방향 바꿈 등)를 지워 확장자를 속일 수 없게 했어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 개별 자막 링크가 있는 게시물(가짜 출처) | `individual_links_give_only_the_chosen_episodes_file_and_the_fonts_once`: 한 게시물의 23·24화 회차가 자기 파일만 받고, 폰트는 한 번만 받아 함께 쓰고, 다른 회차의 Drive 파일은 요청하지 않아요. |
| 회차 ZIP 링크가 있는 게시물(가짜 출처) | `a_zip_of_a_range_is_received_once_for_its_episodes_as_a_zip`(범위 밖 회차는 `게시물에 13화 파일이 없어요`). 개발 환경에서 felia 1화 ZIP이 작업 상세에 `9.2 KB · ZIP · 묶음으로 받음`으로 보였어요. |
| Drive가 확인 페이지나 오류 HTML을 줌 | `a_confirmation_or_error_page_from_drive_is_a_classified_failure`, `a_page_from_drive_instead_of_the_file_is_a_classified_failure`(일곱 가지 응답, 리다이렉트 허용 목록, `confirm` 요청 없음) |
| 실제 게시물(개별 하나, ZIP 하나) | 개발 환경의 worker가 실제 후보로 받았어요: 카이란 13화(Kimi ga Shinu…) `키미시누 13화 미완성.ass` 39,453바이트 ASS, 코코렛 felia 1화(FX 전사 쿠루미, Tistory 본문의 Drive 링크) `삼전닉스 레버리지 01.zip` 9,393바이트 ZIP(CRC 통과, 안에 `FX Senshi Kurumi-chan - 01` SMI 35,186바이트). 리뷰 수정 뒤 다시 빌드해 같은 SHA-256으로 받았어요. 무시해 둔 `real_blogger_posts_are_received_from_drive`로 C소라 `Koori no Jouheki … 15.ass` 48,179바이트, `얼음폰트.zip` 5,011,359바이트, 별명따위 `… 4 24.zip` 11,879바이트도 받았어요. |

실제 Blogger 게시물 16개(C소라·카이란·별명따위·에텔레로사·소소사각·이마이·한1sub과 felia)를 저장해 `the_real_posts_sampled_resolve_to_their_files`로 고정했어요. 그 밖에 `a_tistory_posts_drive_link_is_received_from_drive`, `a_folder_link_waits_and_a_post_of_no_link_has_changed`, `no_download_address_reaches_the_records`, `sources_sharing_one_drive_space_their_drive_requests_together`, `drive_and_the_cdn_set_a_cookie_the_sources_never_send`, `invisible_format_characters_are_dropped_from_names`와 회차 읽기의 단위 시험들이 있어요. 개발 환경의 작업 표에 서명·토큰·쿠키 값이 없었어요. 작업 공간 시험 1,690개가 통과하고(실제 네트워크 시험 2개는 무시), clippy 경고가 없고, 웹 빌드가 돼요.

### 독립 리뷰

원격 파일 이름과 복구, Drive 경계, 회차 고르기, 비밀값을 두고 독립 리뷰를 받았어요. 막는 결함은 없었어요. 이름은 늘 안전한 이름으로 바뀌고, 이름을 먼저 기록해도 재시작 복구의 이름이 어긋나지 않으며, 같은 이름의 두 파일은 다른 경로를 받는 것이 확인됐어요.

| 지적 | 처리 |
| --- | --- |
| 띄어 쓴 `Title 3 - 05`를 3–5화 범위로 읽음 | `화`가 붙을 때만 범위이고, 아니면 뒤 숫자 한 회차예요 |
| 연도·날짜·`H.264`·8자리 숫자 해시를 회차로 읽음 | 표시 없는 그런 숫자는 회차가 아니에요 |
| `03-03`·`24-12`에서 두 숫자를 모두 버림 | 하나씩 읽어요 |
| `24화 (폰트 포함)`을 폰트로만 봄 | 회차를 이름하면 그 회차예요 |
| `drive.google.com/u/0/uc?id=` 꼴을 놓침 | `/u/<n>/`·`/a/<도메인>/`을 떼고 읽어요 |
| 쿠키 시험이 실제 `Set-Cookie` 없이 돌아 아무것도 확인하지 못함 | 시험 서버가 `NID`를 보내고, 다음 요청에 쿠키가 없는지 봐요 |
| 폰트 ZIP도 `묶음으로 받음` | `폰트 묶음`이에요 |
| Tistory와 Blogger가 Drive 간격을 따로 셈 | Drive 하나를 함께 써요 |
| 원격 이름의 방향 바꿈 문자가 확장자를 속일 수 있음 | 보이지 않는 서식 문자를 지워요 |

### 남은 한계

- **보지 못한 Drive 응답**: 확인(바이러스 검사)·할당량·로그인 페이지와 `resourcekey` 링크는 실제 응답을 보지 못하고 알려진 표시로만 가려요. Drive의 403은 공유를 끊은 것으로 보고 `원본 없음`이지만, 할당량 때문의 403일 수도 있어요. 처음 실제로 나오면 확인해야 해요.
- `Content-Length` 없이 오는 Drive 응답을 받는 중에 worker가 멈추면, 기존 복구 규칙대로 그 파일은 보류돼요.
- 회차를 말하지 않는 링크가 둘 이상인 게시물은 고르지 못하고 실패해요. Drive 파일 이름으로 고르면 링크마다 요청이 하나씩 더 들어서 만들지 않았어요. 본 표본에는 그런 게시물이 없었어요.
- 링크 글자를 그대로 믿어요. 에텔레로사 `클레바테스 S2` 4화는 1기 `1-12` 묶음까지 받아요(시즌 표시가 없어서). 어느 파일이 맞는지는 결과 목표 4의 분석이에요.
- 표시 없는 `13.0`은 회차로 읽지 않아요(`AAC2.0`과 가르기 위해서예요).
- Blogger 사용자 도메인 블로그는 주소로 알아보지 못해 `자막 대기`예요.

### 커밋

- `700b612` feat(subtitles): receive a post's Google Drive files for the chosen episode
