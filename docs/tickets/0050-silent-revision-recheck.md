# 0050 받은 회차의 파일 정보를 다시 읽어 Anissia에 안 보이는 수정본을 찾아요

- 상태: 진행 중 (Drive·Tistory·Naver는 끝났고, erulabo는 인증 수신(0041) 뒤에 더해요. 아래 "결과")
- 출처: [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)(받은 뒤 14일 재확인), [제작자 자동 수신 ADR](../adr/0009-follow-subtitle-creator.md), [게시물의 파일 정보를 다시 읽는 비용](../brainstorm/web-gui-subtitles.md#게시물의-파일-정보를-다시-읽는-비용)
- 막는 티켓: [0045](0045-follow-creator-auto-receive.md)(수정본 작업). 출처마다 그 출처의 수신 티켓(0038·0041–0044)이 끝난 뒤에 그 출처를 더해요.

## 작업

구독 제작자에게서 받은 회차는 받은 뒤 14일 동안 하루 한 번 그 게시물의 파일 정보를 다시 읽고, 받은 때와 다르면 그 게시물을 다시 받아 0045의 수정본 작업으로 넘겨요(사용자 결정, 2026-10-02).
Anissia 줄이 그대로인 채 게시물과 파일이 고쳐지는 일이 0033 표본에서 흔했기 때문이에요.

- 받을 때 출처가 인증 없이 주는 파일 정보를 함께 남겨요. Drive는 `HEAD`의 `Last-Modified`·`Content-Length`, Tistory 일반 첨부와 WinPNG 이미지는 파일 주소에 `Range: bytes=0-0`을 건 응답의 전체 크기, Naver는 `aPostFiles`의 `attachFileSize`, erulabo는 게시물의 `dateModified`예요. 서명 URL·토큰은 저장하지 않고 다시 읽을 때 게시물에서 새로 얻어요.
- 파일 값이 달라졌을 때만 다시 받아요. 게시물 수정 시각만 바뀐 것은 글만 고친 것일 수 있어 다시 받지 않아요. erulabo는 파일 값을 볼 수 없으므로 게시물 수정 시각이 바뀌면 다시 받기를 `인증 필요` 할 일로 올려요.
- erulabo는 관찰도 남겨요(사용자 결정, 2026-10-02). 0041이 인증 수신 때 남긴 Drive 파일 ID로 `HEAD`를 읽어 수정 시각·크기를 기록하고, 다음 인증 수신 때의 ID·값과 함께 비교할 수 있게 둬요. 관찰 값이 바뀌어도 자동으로 받지 않고, 게시물 수정 시각과 함께 볼 수 있게 기록만 해요.
- 다시 받은 파일이 지금 자막과 바이트가 같으면 교체할 것이 없다고 기록만 해요.
- 재확인은 worker의 정기 처리이며 출처 호스트마다 요청 간격을 지켜요. 구독 규칙의 `영상 받기`·`자막 받기`가 모두 켜진 동안만 해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 받은 지 사흘 된 회차, Anissia 줄은 그대로이고 가짜 Drive 파일의 크기가 바뀜 | 그 게시물을 다시 받는 수정본 작업이 생기고 현재 자막은 그대로예요. |
| 게시물 수정 시각만 바뀌고 파일 크기는 같음(가짜 Tistory) | 다시 받지 않아요. |
| erulabo 게시물의 `dateModified`가 바뀜(가짜 출처) | `인증 필요` 할 일이 생기고 자동으로 받지 않아요. |
| erulabo 회차의 저장된 Drive ID로 읽은 `HEAD`의 크기가 바뀌고 게시물은 그대로(가짜 출처) | 관찰 기록만 늘고 할 일·수신이 생기지 않아요. 로그와 설정 내보내기에 ID가 없어요. |
| 받은 지 15일 지난 회차 | 다시 읽는 요청이 없어요. |
| 같은 날 worker가 재시작 | 그날 이미 읽은 회차를 다시 읽지 않아요. |
| 다시 받은 파일이 지금 자막과 바이트가 같음 | 교체할 것이 없다는 기록만 남고 `교체 승인` 할 일이 없어요. |
| 실제 Drive·Tistory·Naver 게시물 각 하나 | 실제 서버에서 다시 읽은 값과 받은 때의 값을 비교한 결과가 있어요. |

## 결과

### 만든 것 (2026-10-03)

- **매일 다시 읽기**: worker가 한 시간마다 살펴, 구독 제작자의 출처로 받은 회차의 가장 새 수신을 받은 뒤 14일 동안 하루 한 번(23시간 반 뒤부터) 다시 읽어요. Drive는 `HEAD`의 크기와 `Last-Modified`, Tistory는 파일 주소에 `Range: bytes=0-0`을 건 응답의 전체 크기, Naver는 `aPostFiles`의 `attachFileSize`예요. 출처 호스트의 요청 간격을 지키고, 구독 규칙의 `영상 받기`·`자막 받기`가 모두 켜진 동안만 읽어요.
- **다를 때만 다시 받기**: 파일 값이 받은 때와 다르면 그 게시물을 다시 받는 자동 수정본 작업(`recheck:<항목>:<값 요약>`)을 만들어요. 게시물 수정 시각만 바뀐 것은 보지 않아요. 받은 파일의 키가 사라지고 게시물에 받은 적 없는 파일이 있으면 첨부를 바꾼 것으로 보고 받아요.
- **바꿀 것이 없음**: 다시 받은 파일이 이전 수신과 바이트까지 같으면 그 회차에 `unchanged_from`(이전 작업)을 기록하고, 작업 상세의 회차별 결과에 `바꿀 것이 없음 · 받은 파일이 이전에 받은 것과 같아요`를 보여요. 그 뒤의 읽기는 같은 변화로 매일 작업을 만들지 않아요.
- **하루 한 번**: 읽기 직전의 시각으로 그날의 읽기를 차지해서, 재시작이나 두 worker가 겹쳐도 같은 날 두 번 읽지 않아요. 도중에 끊긴 읽기는 차지를 돌려놓아요.
- **기록(마이그레이션 41)**: `subtitle_item_rechecks`(항목마다 읽은 시각·횟수·결과·읽은 값·만든 작업)와 `subtitle_job_items.unchanged_from`이에요. 서명 URL·토큰·쿠키는 남기지 않아요.
- 웹 API는 `auto:`·`recheck:`로 시작하는 요청 ID를 받지 않아요. 고르기와 올리기 모두 같아요.
- 규칙 전체는 [구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)의 재확인 표에 있어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 사흘 뒤 Drive 파일 크기가 바뀜 | `a_drive_file_whose_size_changed_three_days_after_makes_a_revision_job_and_the_subtitle_stays` |
| Tistory 수정 시각만 바뀜 | `a_tistory_post_whose_modified_time_changed_with_the_same_total_is_not_received_again` |
| Naver | `a_naver_attachment_is_read_from_the_page_and_a_size_change_makes_a_revision` |
| erulabo의 `dateModified`·Drive ID 관찰 | 하지 않았어요. erulabo 수신(0041)이 아직 없어요. |
| 15일 지난 회차 | `a_receipt_fifteen_days_old_is_not_read`, `hourly_looks_read_a_received_episode_fourteen_times_in_fourteen_days` |
| 같은 날 재시작 | `a_restart_the_same_day_reads_nothing_twice_and_the_next_day_reads_again`, `passes_that_start_at_different_times_do_not_claim_one_day_twice` |
| 바이트가 같음 | `a_revision_with_the_same_bytes_records_that_there_is_nothing_to_replace`, `a_file_saved_again_every_day_with_the_same_bytes_makes_one_job_not_one_a_day`, `a_size_that_differs_between_the_receipt_and_the_recheck_makes_one_job_not_one_a_day`. 개발 환경(서버 데이터 복사본)에서 받은 회차 하나에 `unchanged_from`을 잠시 넣어 작업 상세에 문장과 이전 작업 링크가 보이는 것을 확인하고 되돌렸어요. |
| 실제 Drive·Tistory·Naver 게시물 | 무시된 실제 네트워크 시험 `real_posts_are_read_again_and_agree_with_what_was_received`가 2026-10-03에 통과했어요. 받은 바이트 수(39,453, 11,724, 106,408, 44,480)와 Drive의 `Last-Modified` 문자열이 다시 읽은 값과 같았어요. |
| 첨부 교체·사라짐·실패 | `a_tistory_attachment_deleted_and_attached_again_is_received_again`, `a_drive_file_gone_with_a_new_link_in_the_post_is_received_again`, `a_post_that_still_offers_its_other_files_but_not_the_missing_one_is_only_missing`, `a_file_that_is_gone_is_recorded_makes_no_job_and_is_read_again_only_within_the_window`, `a_failing_site_is_recorded_and_tried_again_the_next_day` |
| 0047·0048과의 경계 | `an_upload_of_the_subscribed_creator_is_never_read_again`, `a_receipt_that_revised_a_file_the_user_named_is_read_again_like_any_other`, 올리기 API의 `recheck:` ID 거절 시험 |

작업 공간 시험 1,863개가 통과하고(실제 네트워크 시험 4개는 무시), web node 시험 9개가 통과하고, clippy 경고가 없고, 웹 빌드가 돼요. 개발 환경에서 마이그레이션 41이 서버 데이터 복사본에 적용됐어요. 실제 게시물이 고쳐지는 것을 지켜보지는 않았어요.

### 독립 리뷰

마이그레이션, 하루 한 번의 차지, 신뢰 경계, 바이트 비교를 두고 리뷰를 받았어요. 막는 결함은 없었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| 시작 시각이 다른 두 worker가 같은 날 한 항목을 둘 다 읽음 | 읽기 직전의 시각으로 차지하고, 1시간 넘게 앞선 차지만 시계가 되돌아간 것으로 봐요 |
| 받은 값과 다시 읽은 값이 늘 다르면 매일 작업이 생김 | 바이트가 같았던 재확인 뒤에는 그 읽기의 값과 견주고 14일을 새로 시작하지 않아요 |
| 첨부를 지우고 다시 붙인 수정이 `사라짐`으로만 남음 | 게시물에 받은 적 없는 파일이 있으면 받아요 |
| 14번째 읽기가 거의 일어나지 않음 | 23시간 반 뒤부터 읽어요 |
| 웹의 고르기가 `recheck:` ID를 받음 | 거절해요 |
| 차지가 이전 결과를 지우지 않음 | 지우고, 끊긴 읽기는 되돌려요 |
| `바꿀 것이 없음` 기록과 항목 완료가 따로 저장됨 | 한 트랜잭션이에요 |

0047·0048을 합친 뒤 경계를 다시 리뷰받았어요. 막는 결함은 없었고 아래를 고쳤어요.

| 지적 | 처리 |
| --- | --- |
| 올리기 API가 `recheck:` ID를 받음 | 앱이 쓰는 ID 판단(`trss_jobs::is_app_command`)을 고르기와 함께 써요 |
| 올린 작업과 제작자를 붙인 수정본의 재확인을 지키는 시험이 없음 | 시험 두 개와 명세 문장을 더했어요 |
| `unchanged_from`이 화면에 안 보임 | 작업 상세의 회차별 결과에 보여요 |

### 남은 한계

- erulabo의 게시물 수정 시각 확인과 Drive ID 관찰은 0041 뒤에 더해요. WinPNG 이미지도 읽지 않아요.
- 같은 크기로 고친 Tistory·Naver 파일은 찾지 못해요. 바이트가 같았던 재확인 뒤에는 Drive도 같은 크기의 변화를 찾지 못해요.
- Drive는 옛 파일이 남은 채 링크만 새 파일로 바꾼 수정을 찾지 못해요.
- 회차의 작업 가운데 보류·대기 중인 것이 있으면 그동안 읽지 않아요.
- 새 줄을 보고 만든 자동 수신 작업과 재확인 작업이 짧은 틈에 같은 회차에 둘 다 생길 수 있어요(같은 게시물을 두 번 받을 뿐 기록은 깨지지 않아요).
- 작품을 합치면 이전 수신은 옛 작품 ID를 가져서 재확인에서 빠져요.

