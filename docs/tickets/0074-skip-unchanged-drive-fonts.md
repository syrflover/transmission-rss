# 0074 바뀌지 않은 Drive 폰트를 다시 받지 않아요

- 상태: 완료(2026-10-05)
- 출처: [폰트](../specs/subtitles.md#폰트)의 재수신 규칙(사용자 결정, 2026-10-04)
- 막는 티켓: [0064](0064-multi-file-packages.md)(폰트 자산)

## 작업

- Google Drive 개별 폰트 파일은 받기 전에 `HEAD`로 크기와 `Last-Modified`를 읽어요. 같은 제작자 폴더에 그 값으로 보관한 폰트가 있으면 받지 않고 기존 자산을 써요. 값이 다르거나 읽지 못하면 받아요.
  보관할 때의 크기와 `Last-Modified`를 폰트 자산과 함께 남겨야 해요. 목표 3의 Drive 수신([0042](../archive/tickets/3-subtitle-candidates-and-receiving/0042-source-blogger-drive.md))을 고쳐요.
- Naver·Tistory 첨부의 폰트는 받아서 내용으로 비교하고, 같으면 기존 자산을 써요. 확인하려고 다시 받았다고 표시해요.
- 통째 묶음(ZIP)으로만 오는 폰트는 전체를 받고 같은 폰트는 기존 자산을 써요. 화면은 전체 묶음 수신과 실제로 더한 새 자산을 구분해요.
- 작업 상세에 폰트마다 `받지 않음(바뀌지 않음)`·`받아서 같음`·`새로 받음`을 적어요. 로컬 중복 제거를 네트워크 재수신을 막은 것으로 보고하지 않아요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 같은 Drive 폰트를 쓰는 다음 회차(가짜 Drive, 시험) | 그 폰트를 받지 않고 기존 자산을 써요. 가짜 Drive의 전송 기록에 그 폰트가 없어요. |
| Drive 폰트의 `Last-Modified`가 바뀜 | 받아서, 내용이 다르면 번호를 붙인 새 자산으로 둬요. |
| 내용이 같은 Naver 폰트 | 받지만 새 자산을 만들지 않고 `받아서 같음`이에요. |
| ZIP 안의 같은 폰트 | 전체를 받았다고 적고, 새 자산은 0개예요. |
| 실제 Drive 게시물(개발 환경) | 두 번째 받기에서 폰트를 전송하지 않았는지가 기록돼 있어요. 표본에 폰트가 없으면 그렇다고 적어요. |

## 결과

### 만든 것 (2026-10-05)

- **받기 전의 확인**: Drive 개별 파일(`drive:<ID>`, 서버 브라우저가 받은 파일은 빼요)을 받기 전에, 같은 파일의 마지막 수신 기록을 찾아요(`trss_jobs::place::unchanged`). 그 기록이 작업이 보관할 제작자 폴더에 지우지 않은 폰트로 남아 있고, 그 파일이 기록한 크기·SHA-256 그대로일 때만 `HEAD`를 한 번 보내요. 크기와 `Last-Modified`가 같으면 받지 않고, 수신 기록을 경로 없이 `done`으로 남기고 쓸 폰트 자산을 적어요(`unchanged_asset`). `HEAD`가 실패하거나 `Last-Modified`가 없거나 값이 다르면 받아요.
- **비교할 값을 두는 곳**: 작업 절은 크기와 `Last-Modified`를 폰트 자산과 함께 남기라고 적었지만, 따로 열을 두지 않고 수신 기록의 크기와 스냅샷의 `last_modified`를 읽어요. 같은 바이트를 다시 받은 기록이나 받지 않은 기록이 저절로 다음 비교의 기준이 되게 하려는 거예요. 마지막 기록에 `Last-Modified`가 없으면 더 앞의 기록으로 돌아가지 않고 `HEAD` 없이 받아요. 앞의 값은 지금 파일을 말하지 않기 때문이에요.
- **migration 56** `unchanged_fonts.sql`: `subtitle_job_files.unchanged_asset`(`subtitle_assets` 참조, 경로와 함께 쓸 수 없는 `CHECK`)과 색인 셋(파일 키, 계획 줄의 수신 기록, 받지 않은 기록의 폰트)을 더해요.
- **보관**: 받지 않은 기록의 줄은 보관할 때 폰트를 다시 확인하고, 같은 바이트를 다시 받은 것처럼 그 폰트를 써요(묶음 항목, 줄의 자산, 자막과의 연결). 그사이 폰트가 정리됐거나 파일이 바뀌었거나 없어지면, 한 트랜잭션에서 그 기록과 나눠 쓴 기록을 `abandoned`로 두고, 아직 보관하지 않은 줄을 지우고, 받은 항목만 대기로 돌려 다시 받아요(`revoke`). 보류·실패·대기 항목은 상태와 까닭을 그대로 둬요.
- **정리(0073)와의 관계**: 끝나지 않은 작업의 받지 않은 기록이 가리키는 폰트는, 그 줄을 보관하기 전까지 정리가 `진행 중인 작업이 이 파일을 써요`로 남겨요.
- **재시작**: `runner.rs` 모듈 문서의 표에 세 줄을 더했어요. `HEAD`와 기록 사이에서 끊기면 `HEAD`를 다시 보내요. 기록 뒤에 끊기면 기록을 쓰고 보관할 때 다시 확인해요. 그사이 폰트가 없어졌거나 바뀌었으면 다시 받아요. 이어서 돌 때 작업 기록은 `바뀌지 않은 폰트라 받지 않았어요`라고 적고, 바이트를 확인했다고 적지 않아요.
- **API와 화면**: 작업 상세의 파일에 `unchanged`·`new_assets`, 배치 줄에 `font_receipt`(`unchanged`·`same`·`new`)를 더했어요. 화면은 폰트마다 `받지 않음(바뀌지 않음)`·`받아서 같음`·`새로 받음`을, 압축 파일에는 `묶음 전체를 받음 · 새로 보관한 파일 N개`를 적어요(`web/src/screens/todo/fontReceipt.ts`). 같은 바이트를 하나만 보관한 것을 받지 않은 것으로 적지 않아요.
- **명세**: [폰트](../specs/subtitles.md#폰트), [보관 파일 정리](../specs/library.md#보관-파일의-정리), [체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구)에 위 동작을 적었어요.

### 검증한 것

| 완료 기준 | 결과 |
| --- | --- |
| 같은 Drive 폰트를 쓰는 다음 회차 | 시험 `the_next_episodes_unchanged_drive_font_is_not_received_and_its_font_is_used`: 두 번째 작업은 그 폰트에 `HEAD` 하나만 보내고 `GET`은 보내지 않아요. 수신 기록은 경로 없이 폰트 자산을 가리키고, 새 자막이 그 폰트에 이어지며, 폰트 파일은 하나예요. |
| `Last-Modified`가 바뀜 | 시험 `a_changed_last_modified_receives_the_font_and_keeps_new_bytes_numbered`: 다른 바이트는 `Font (2).ttf`와 `새로 받음`이에요. 같은 바이트는 새 자산 없이 `받아서 같음`이고, 그다음 회차는 받지 않고 `Font (2).ttf`에 이어져요. |
| 내용이 같은 Naver 폰트 | 시험 `a_naver_font_is_received_compared_and_not_kept_twice`: `HEAD` 없이 다시 받고, 폰트는 하나이며 `받아서 같음`이에요. |
| ZIP 안의 같은 폰트 | 시험 `a_zip_whose_font_is_kept_is_received_whole_and_adds_no_font`: ZIP을 `HEAD` 없이 통째로 받고, 새로 보관한 파일은 0개예요. |
| 실제 Drive 게시물(개발 환경) | 개발 데이터의 Drive 게시물 두 개(ZIP 하나, ASS 하나)에는 따로 올라온 폰트가 없어 두 번째 받기를 관찰하지 못했어요. 대신 이 기능이 기대는 값을 같은 두 파일로 확인했어요. `HEAD`가 준 크기와 `Last-Modified`가 받을 때의 기록과 같았어요(2026-10-05: 9393바이트 `Fri, 02 Oct 2026 04:15:40 GMT`, 39453바이트 `Tue, 29 Sep 2026 19:53:19 GMT`). 2026-10-03의 관찰([구독 제작자 자동 수신](../specs/subtitles.md#구독-제작자-자동-수신)의 재확인 표)과도 같아요. |

완료 기준 밖에서 더 확인한 것이에요.

- `HEAD`가 503이거나 `Last-Modified`가 없으면 받아요(`a_font_is_received_when_its_head_fails_or_has_no_last_modified`). 보관한 파일이 없거나 정리로 지운 폰트는 `HEAD` 없이 받아요(`a_font_whose_kept_file_is_missing_is_received_with_no_head`, `a_font_a_cleanup_removed_is_received_with_no_head`).
- 재시작: `a_run_cut_before_the_font_was_recorded_asks_its_head_again`, `a_font_found_unchanged_by_a_run_cut_short_is_not_told_as_bytes_checked`, 기다리는 사이 폰트가 바뀌거나 지워진 두 경우(`a_font_not_received_whose_kept_file_*_meanwhile_is_received`).
- 정리가 받지 않은 기록의 폰트를 남기고, 그 작업이 이어서 그 폰트를 써요(`a_cleanup_keeps_the_font_a_job_did_not_receive_until_it_stores_it`). `revoke`는 받은 항목만 대기로 돌려요(`a_font_that_went_away_puts_back_only_the_items_that_received_it`).
- migration 56: 55에서 올린 DB에서 새 열이 비어 있고, `CHECK`와 외래 키가 잘못된 행을 거절해요(`the_migration_adds_the_font_a_receipt_uses_to_receipts_of_earlier_builds`).
- 지키는 코드를 빼면 실패하는지 봤어요. `receive()`의 확인을 빼면 시험 6개가, 읽지 못한 파일을 맞는 것으로 보게 하면 파일이 없는 경우의 시험이 실패했어요. 검토 뒤 고친 세 곳(정리의 보호, `revoke`의 항목, 재시작 문구)도 하나씩 되돌리면 각자의 시험이 실패해요(`probe-out/0074-mutations.txt`, 저장소에 없음).
- 개발 환경(이 작업 트리로 지은 이미지, migration 56 적용, 2026-10-05): Naver 폰트 3개를 받은 작업 상세는 폰트마다 `폰트 · 새로 받음`을, Drive ZIP 작업은 `묶음 전체를 받음 · 새로 보관한 파일 0개`를 보여줬어요. 데스크톱 너비에서만 봤어요.

### 독립 검토

검토 한 번(2026-10-05)이 막는 결함 없이 다섯 가지를 짚었고, 넷을 고쳤어요.

- 정리가 받지 않은 기록이 가리키는 폰트를 보호하지 않았어요. 그 사이 원본이 없어지면 자막이 폰트 없이 이어질 수 있었어요. 정리의 참조 확인에 그 기록을 더했어요.
- 폰트가 없어졌을 때 보류·실패 항목까지 대기로 돌려 까닭을 지웠어요. 받은 항목만 돌려요.
- 인증을 기다리는 항목이 있는 작업에서 다시 받기가 그 대기와 화면을 지울 수 있었어요. 인증 대기는 그대로 두고, 인증 뒤의 실행이 대기로 돌린 항목도 받아요.
- 이어서 돌 때 받지 않은 폰트를 `바이트 확인`으로 적었어요. 문구를 바꿨어요.
- 기록을 찾는 조회와 `revoke`가 계획 표를 색인 없이 읽었어요. 색인을 더했어요.

### 시험

검토 뒤 고친 코드(2026-10-05, `6fccf55` 위)에서 `cargo test --workspace`가 2,570개 통과, 실패 0, 무시 13이었어요(작업 트리의 다른 변경을 뺀, 이 commit과 같은 트리). `cargo clippy --workspace --all-targets -- -D warnings`와 `cargo fmt --all --check`가 통과해요. 웹은 `bun run test` 131개 통과, `bunx tsc -b`가 통과했어요.

### 한계

- 실제 Drive 폰트로 두 번째 받기를 관찰하지 못했어요. 표본에 따로 올라온 Drive 폰트가 없어요.
- 인증 대기를 지키는 수정에는 그 경우를 만드는 시험이 없어요.
- 대기로 돌린 항목은 게시물을 다시 읽고 그때 게시물이 주는 파일을 받아요. 여러 회차가 같은 폰트를 나누면 그만큼 게시물을 다시 읽어요.
- 보관한 폰트 파일이 없어진 채 자산이 남았거나 사람이 덮어쓴 경우, 다시 받은 폰트는 `Font (2).ttf`처럼 번호를 붙여 남아요(기존 이름 규칙).
- 웹 API의 JSON 필드 이름을 확인하는 `trss-web` 시험은 없어요. 통합 시험은 같은 함수(`font_receipt`, `new_assets`)를 직접 불러요.
