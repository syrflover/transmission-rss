# 0073 보관 파일을 정리하고 용량을 보여줘요

- 상태: 완료
- 출처: [보관 파일의 정리](../../../specs/library.md#보관-파일의-정리), [보관본과 적용본](../../../specs/subtitles.md#보관본과-적용본)과 [폰트](../../../specs/subtitles.md#폰트)의 정리 조건, [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)의 폰트 정리 경쟁, [설정 화면](../../../specs/settings.md#설정-화면)의 파일 용량
- 막는 티켓: [0064](0064-multi-file-packages.md)

## 작업

- 작품 상세의 `파일` 카드에 작품 폴더와 파일 대응, 정리할 수 있는 것(지난 수정본, 영상 대기 자막, 보관만 한 자막과 회차에 붙지 않은 자막)을 보여줘요.
  정리는 파일 하나씩, 대상 파일을 이름으로 밝힌 확인을 거쳐요. 한 목록에서 한꺼번에 지우는 기능은 두지 않아요.
- 정리는 앱이 만들고 소유를 확인한 보관 파일로 제한하고, 남은 보관본과 진행·보류·복구 중인 작업이 참조하지 않는지 확인해요. 등록되지 않은 파일은 이름만 보고 지우지 않아요.
- 다른 자막이 쓰지 않는 폰트와, 그 수정본만 쓰던 첨부·구성 파일을 함께 정리해요. 참조를 확인한 뒤 지우는 사이에 새 참조가 생기지 않게 직렬화해요.
- 설정에 작품별 보관 용량과 정리할 수 있는 항목의 요약을 두고, 누르면 그 작품의 `파일` 카드로 가요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 같은 폰트를 쓰는 두 수정본 가운데 하나를 정리 | 그 자막만 지워지고 폰트는 남아, 다른 보관본을 다시 적용할 수 있어요. |
| 폰트를 혼자 쓰던 수정본을 정리 | 자막과 그 폰트, 그 수정본만 쓰던 첨부가 함께 지워져요. |
| 진행 중이거나 보류한 작업이 참조하는 보관본 | 정리할 수 없고 까닭이 보여요. |
| 정리를 확인하는 순간 새 작업이 그 폰트를 참조함(시험) | 폰트가 남아요. |
| `.trss/` 안의 등록되지 않은 파일 | 정리할 것 목록에 없고 지우지 않아요. |
| 정리 도중 worker를 죽였다 살림(시험) | 거짓 완료가 없고 다른 자산을 잃지 않아요. |
| 설정의 파일 용량 | 작품별 보관 용량과 정리할 수 있는 항목 수가 보이고, 누르면 그 작품의 `파일` 카드로 가요. |

## 결과

### 만든 것 (2026-10-05)

- **마이그레이션 55**(`cleanup.sql`): `subtitle_stored.cleaned_at`(사람이 정리를 확인한 때)과 `subtitle_assets.removed_at`(worker가 파일이 없음을 확인한 때)을 더했어요. 같은 경로를 하나로 묶는 색인 `subtitle_assets_one_path`는 지우지 않은 자산에만 걸려요. 그래서 지운 자산의 행은 묶음·계획의 기록으로 남고, 같은 경로에 새 파일을 보관할 수 있어요. 정리 요청(`subtitle_cleanups`: `asked`·`done`·`held`, 보관본마다 진행 중인 것은 하나)과 파일별 결과(`subtitle_asset_removals`: `named`·`intended`·`done`·`kept`·`held`)를 기록해요.
- **정리할 것**(`place::cleanup`): 영상 옆 적용본이 없고 정리하지 않은 보관본을 `지난 수정본`·`영상 대기`·`보관만 함`·`회차에 붙지 않음`으로 나눠요. 진행 중·보류 작업, 회차 확인이나 교체 승인을 기다리는 작업, 끝나지 않은 파일 효과가 쓰는 보관본과 작품 폴더가 없는 작품의 보관본은 까닭(`blocked`)과 함께 막아요. 영상을 기다리는 줄은 막지 않아요. 함께 지울 파일(`with`)과 남는 파일(`kept`)은 `in_use` 한 함수로 정해요. 다른 보관본이 같은 자막 자산을 쓰거나 잇는 파일, 자막 자산이 없는 묶음(폰트만 올린 묶음)에 든 파일, 끝나지 않은 항목이나 아직 보관하지 않은 자막 줄이나 연결을 모르는 보관본이 있는 작업의 계획이 가리키는 파일, 끝나지 않은 효과의 경로는 남겨요. 연결(`link`)은 지운 자산을 잇지 않아요.
- **확인**(`cleanup::ask`, `POST /api/library/works/{id}/stored/{stored_id}/clean`): IMMEDIATE 트랜잭션 하나에서 보낸 `assets`가 지금의 `with`와 같은지 보고, 정리 요청과 파일 행(`named`)을 쓰고 `cleaned_at`을 적어요. 영상을 기다리던 그 보관본의 줄은 `stored`(`영상을 기다리다 정리했어요`)가 되고, 더 기다릴 줄이 없는 작업은 다시 줄에 서서(`정리한 보관본을 반영해요`) 다음 실행에서 끝나요. 다른 작품의 것이거나 이미 정리했으면 `404`, 막혔으면 까닭과 함께 `409`, 목록이 바뀌었으면 새 항목(`current`)과 함께 `409`예요.
- **worker**(`Placer::run_cleanups`, `run_jobs_once`): 작업을 돌리는 같은 흐름에서, 작업보다 먼저, 같은 hold 아래에서 정리해요. 확인받은 파일만 다시 검사해(`intend`) 그사이 새로 쓰이게 된 파일은 `kept`로 남겨요. 지울 파일은 기록한 크기와 SHA-256이 같을 때만 지우고 폴더를 sync해요. 내용이 다르면 `기록과 내용이 달라 지우지 않았어요`, 작품 폴더나 보관 기준 폴더(`.trss/subtitles`, 앱 데이터의 `subtitle-files`)가 없으면 `작품 폴더를 찾지 못했어요`로 보류해요. 정리 하나가 DB 오류로 실패해도 기록을 남기고 다음 정리로 넘어가요. 파일이 없음을 확인한 뒤에만 `done`과 `removed_at`을 적고, 비게 된 제작자 폴더는 보관 기준 폴더 바로 아래까지 지워요. 정리한 보관본을 적용하려던 줄이 다시 돌면 실패하지 않고 `정리한 보관본이라 적용하지 않았어요`로 끝나요(`apply_row`).
- **보관본을 읽는 곳**: 제작자 폴더 이름은 지운 자산의 경로도 보고 정해요(`asset_paths_under`). 그래서 정리 뒤 남은 폴더와 대소문자만 다른 폴더를 새로 만들지 않아요. 재사용(`assets_under`, `asset`), 같은 바이트 묶기(`stored`), `stored_asset`, `stored_only`, `choose_stored`, `row_paths`, 교체의 `newer_revision`·`stored_place`·`imported`가 정리한 보관본과 지운 자산을 건너뛰어요.
- **API**: 작품 상세에 `storage`(`total`, `cleanable`, `cleaning`)를 더했어요. `GET /api/library/storage`는 보관한 파일이나 표지가 있는 작품을 용량 순으로, 종류(`subtitle`·`font`·`attachment`·`cover`)별 개수·용량과 지금 정리할 수 있는 보관본 수와 함께 줘요.
- **웹**(`StoredCleanup.tsx`, `storage.ts`, `settings/storage/`): `파일` 카드에 `정리할 수 있는 파일` 부분을 더했어요. 보관한 자막·폰트·첨부의 용량, 종류별 묶음, 줄마다 회차·제작자·크기·받은 날(같은 회차를 같은 날 두 번 받았으면 시각까지)과 `정리` 또는 막힌 까닭을 보여줘요. `정리`는 누른 자리에서 지울 파일과 크기·합계, 남는 파일과 까닭을 밝히고 묻고, 목록이 바뀌었으면 새 목록으로 다시 물어요. 맡긴 정리는 `정리를 맡긴 파일`에서 끝날 때까지 3초마다 다시 읽어요. 여러 파일을 한 번에 고르는 기능은 없어요. 설정의 `파일 용량·정리`는 작품마다 용량과 종류, `정리할 파일 n개`를 보여주고, 누르면 `/library/{id}#files`로 가서 `파일` 카드를 펼치고 그 카드로 스크롤해요.
- **명세**: [보관 파일의 정리](../../../specs/library.md#보관-파일의-정리)에 `보관 파일 정리는 이렇게 동작해요`를, [체크포인트와 중단 복구](../../../specs/jobs.md#체크포인트와-중단-복구)에 정리의 직렬화와 재시작 때 파일이 없는 경우를 더했어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 같은 폰트를 쓰는 두 수정본 가운데 하나를 정리 | 시험 `cleaning_one_of_two_revisions_sharing_a_font_keeps_the_font_for_the_other`: 정리한 수정본의 자막만 지워지고 폰트는 `kept`(`다른 자막도 이 파일을 써요`)로 남아요. 다른 보관본을 다시 적용하면 폰트와 함께 영상 옆에 놓여요. |
| 폰트를 혼자 쓰던 수정본을 정리 | 시험 `cleaning_a_revision_alone_with_its_font_and_attachment_removes_all_three`: 자막·폰트·첨부가 디스크에서 사라지고 `removed_at`이 적히고 정리는 `done`이에요. 비게 된 제작자 폴더도 지워지고 기준 폴더는 남아요. 같은 바이트를 두 번 받아 보관본이 하나인 경우(`a_subtitle_received_twice_is_cleaned_with_its_font`)도 자막과 폰트가 함께 지워져요. 개발 환경(2026-10-05)에서 `FX Senshi Kurumi-chan`의 지난 수정본 `dev-1.ass`를 정리하자 `.trss/subtitles/가짜 출처/dev-1.ass`가 사라지고 정리가 `done`이 됐어요. |
| 진행 중이거나 보류한 작업이 참조하는 보관본 | 시험 `a_stored_subtitle_a_running_or_held_job_uses_is_not_cleanable`(진행 중·보류)과 API 시험 `a_stored_subtitle_a_held_job_uses_says_why_it_cannot_be_cleaned`(`blocked`와 `409`)이 확인해요. 작품 폴더나 `.trss/subtitles`가 없으면 `작품 폴더를 찾지 못해 지금은 정리할 수 없어요`예요(`nothing_is_cleaned_while_the_work_folder_is_away`, API `nothing_of_a_work_whose_folder_is_away_is_cleaned`, `nothing_is_cleaned_where_the_work_folder_has_no_stored_subtitles_folder`). 개발 환경에서는 교체 승인을 기다리는 보관본이 `정리` 없이 까닭을 보였어요. |
| 정리를 확인하는 순간 새 작업이 그 폰트를 참조함(시험) | 시험 `a_font_a_new_job_takes_up_after_the_confirmation_stays`: 확인 뒤 worker의 정리 전에 새 작업이 같은 폰트를 재사용해 이으면, 정리는 폰트를 `kept`로 남기고 나머지만 지워요. 연결 전에 끊긴 작업(`a_font_a_job_cut_before_its_links_kept_stays`), 아직 받지 못한 항목이 있는 작업(`a_font_of_a_job_with_an_item_not_received_yet_stays`)의 폰트도 남고, 연결은 지운 자산을 잇지 않아요(`a_job_links_no_removed_file`). 끊긴 작업·받지 못한 항목·연결의 세 시험은 각 보호를 빼면 실패하는 것을 확인했어요. |
| `.trss/` 안의 등록되지 않은 파일 | 시험 `a_file_the_app_did_not_record_is_listed_nowhere_and_stays`: 기록에 없는 파일은 어느 목록에도 없고, 이웃 파일을 정리한 뒤에도 남으며 그 폴더도 남아요. 그 폴더의 이름은 이후 대소문자만 다른 제작자도 그대로 써요(`a_creator_folder_a_cleanup_emptied_keeps_its_name`). |
| 정리 도중 worker를 죽였다 살림(시험) | 시험 세 개가 각 지점을 흉내 내요. 확인만 된 때(`a_worker_killed_after_the_confirmation_takes_the_cleanup_up`), 파일 행이 `intended`이고 파일이 남은 때(`a_worker_killed_with_removals_intended_removes_them_on_its_next_pass`), 파일을 지웠지만 기록 전인 때(`a_worker_killed_after_removing_a_file_before_recording_it_counts_it_removed`)예요. 다시 돌린 정리는 `done`으로 끝나고 다른 자산을 잃지 않아요. 내용이 바뀐 파일(`a_file_changed_on_disk_is_held_and_left_as_it_is`)과 빈 마운트 지점(`an_empty_mount_point_holds_the_cleanup_without_false_done`)은 지우지 않고 보류하며 `done`을 적지 않아요. 정리 하나가 DB 오류로 실패해도 다른 정리는 끝나요(`a_cleanup_that_fails_does_not_stop_the_others`). |
| 설정의 파일 용량 | API 시험 `the_storage_list_shows_each_works_kinds_cover_and_cleanable_count`가 작품별 합계, 종류, 표지 크기, 정리할 수 있는 수를 확인해요. 개발 환경에서 설정의 `파일 용량·정리`가 작품 63개를 용량 순으로 보였고, `Clevatess`·`FX Senshi Kurumi-chan` 줄을 누르면 `#files`로 가서 `파일` 카드가 펼쳐진 채 맨 위에 왔어요. PC 폭과 375px 폭 모두 그랬고, 375px에서 포커스가 카드 제목에 가고 가로 스크롤이 없었어요. |

그 밖에도 확인했어요.

- 영상 대기 보관본: 시험 `a_subtitle_waiting_for_its_video_is_cleaned_and_its_job_settles`는 그 보관본만 쓰던 폰트·첨부까지 지워지고, 기다리던 작업이 끝나며, 뒤에 온 영상이 아무것도 적용하지 않는 것을 확인해요. 개발 환경에서 12화의 영상 대기 보관본 `dev-series.ass`를 정리하자 작업 `c20d06b6`이 `done`이 되고 그 줄은 `stored`(`영상을 기다리다 정리했어요`)가 됐어요. 그 자막 파일은 다른 보관본과 같은 바이트라 남았어요.
- 정리한 보관본의 줄이 다시 돌면 실패가 아니라 `정리한 보관본이라 적용하지 않았어요`로 끝나요(`a_row_of_a_cleaned_copy_back_in_line_ends_stored_not_failed`). 같은 경로에 새 파일을 보관할 수 있어요(`a_removed_path_takes_a_new_file_of_the_same_name`).
- 마이그레이션 55는 54의 DB에서 행을 지키고, 지운 자산의 경로를 새 자산이 쓸 수 있어요(`stored_rows_survive_the_cleanup_migration_and_a_removed_asset_frees_its_path`). worker는 준비된 작업이 없어도 확인한 정리를 해요(`a_confirmed_cleanup_is_carried_out_when_no_job_is_ready`).
- 웹 시험(`storage.test.ts`)은 종류 이름과 순서, 크기·받은 날 문장(같은 날 두 번이면 시각), 확인 목록과 합계, 보낼 본문, `409`·`404`를 읽는 방식, 끝난 정리의 안내, 설정 요약을 확인해요. 휴대폰 폭의 확인 창은 버튼이 44px이고 가로 스크롤이 없었어요.

### 독립 검토

복잡한 불변식 검토(2026-10-05)는 쓰이는 파일을 지우거나, 보여주지 않은 파일을 지우거나, 두 보관 기준 폴더 밖을 건드리는 경로를 찾지 못했어요. 지적은 이렇게 다뤘어요.

- 한 게시물의 다른 항목이 아직 받히지 않은 작업은 앞으로 그 게시물의 폰트를 이을 수 있는데 보호받지 못했어요. 끝나지 않은 항목이 있는 작업도 보호하고, 연결이 지운 자산을 잇지 않게 했어요.
- 빈 마운트 지점에서는 파일이 모두 없어 보여 거짓 `done`이 될 수 있었어요. worker와 확인 모두 `.trss/subtitles`가 있어야 정리해요.
- 정리 뒤 남은 제작자 폴더와 대소문자만 다른 폴더가 새로 생길 수 있었어요. 폴더 이름은 지운 자산의 경로도 보고 정해요.
- 정리 하나의 DB 오류가 뒤의 정리를 모두 막았어요. 기록하고 다음으로 넘어가요.
- 남긴 파일과 보류한 정리의 파일은 다시 정리할 길이 없어요. 한계로 남겼어요.

다시 한 검토는 다섯 수정이 각 시나리오를 닫는다고 봤어요. 확인 단계의 폴더 검사가 작품 폴더만 본다는 남은 지적은 `.trss/subtitles`를 보게 고쳤어요. 검토 전에 개발 환경에서 찾은 결함도 있어요. 같은 바이트를 다시 받아 보관 행 없이 남은 묶음을 폰트만 올린 묶음으로 보아 자막을 지우지 않았어요. 이제 자막 자산이 하나도 없는 묶음만 폰트만 올린 묶음이에요.

### 시험

검토로 고친 코드(2026-10-05, `d04ead2` 위의 작업 트리)에서 `cargo test --workspace`가 2,555개 통과, 실패 0, 무시 13개였어요. 정리 시험(`tests/cleanup.rs`)은 22개, 작품 상세 API 시험은 14개(정리 5개)예요. `cargo clippy --workspace --all-targets -- -D warnings`와 `cargo fmt --all --check`가 통과해요. 웹은 `bun run test` 129개가 통과했고 `tsc -b`가 통과했어요. 개발 환경 관찰은 검토 수정 전후의 두 이미지에서 했어요. 끝난 정리의 안내와 확인 단계의 `.trss/subtitles` 검사는 그 뒤에 고쳐서 시험으로만 확인했어요.

### 한계

- 정리하면서 남긴 파일(그때 작업이 쓰던 것)과 보류한 정리의 파일은 등록된 채 용량에 남지만 다시 정리할 길이 없어요. 데이터를 잃지는 않아요.
- 한 게시물의 다른 항목이 받혔지만 아직 계획되지 않은 틈(압축 파일이 이후 빌드를 기다리는 동안 등)에는 그 게시물의 폰트가 보호받지 못해요. 나중에 계획한 자막은 그 폰트 없이 이어져요. 재검토가 확신 낮음(P3)으로 남긴 경우이고 고치지 않았어요.
- 여러 항목 게시물의 보호는 SQL로 만든 상태로 시험했고, 실제 여러 항목 작업을 끝까지 돌려 보지는 않았어요.
- `#files` 주소로 페이지를 새로 열면 위쪽 내용이 늦게 커져서 스크롤이 카드에 못 미칠 수 있어요. 설정에서 눌러 오는 경로는 확인했어요.
- 비게 된 폴더는 지우지만, 앱 데이터의 `subtitle-files`와 작품의 `.trss/subtitles`는 지우지 않아요.
- 휴대폰 폭은 내장 브라우저의 휴대폰 크기로만 봤어요.

