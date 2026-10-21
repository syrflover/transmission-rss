# 0153 자막 작업의 효과별 기록을 없애고 단계를 기록 없이 다시 해도 안전하게 해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 끊긴 자막 작업](lighter-structure.md#끊긴-자막-작업), [끊긴 작업 ADR](../adr/0020-interrupted-jobs-wait-for-a-restart.md)
- 막는 티켓: [0151](0151-probe-rename-exchange.md)(서버의 `RENAME_EXCHANGE` 지원), [0152](0152-interrupted-jobs-stop.md)(`중단됨`과 다시 시작)

## 작업

[0152](0152-interrupted-jobs-stop.md) 뒤 `다시 시작`은 효과별 기록과 디스크를 대조해 끊긴 단계를 이어 해요. 이 티켓은 기록 없이 단계를 다시 해도 안전하게 파일을 쓰는 방법을 바꾸고, 기록과 대조 코드를 없애요. 2026-10-21에 효과의 기록만 담는 테이블은 `jobs/store_apply.sql`의 `subtitle_file_effects` 하나이고, 받기 기록(`subtitle_job_files`)과 재배치 제거(`subtitle_relocations`), 교체 계획에는 효과의 상태와 열이 섞여 있어요.

- 파일을 쓰는 단계는 새 파일을 끝까지 쓴 뒤 이름을 바꿔 제자리에 둬요.
- 교체는 새 파일과 지금 파일을 `RENAME_EXCHANGE`로 한 번에 맞바꾸고, 밀려난 파일이 승인한 원래 파일과 다르면 다시 맞바꿔요. 효과별 기록 없이도 경로가 비는 순간을 없애려는 거예요. 서버의 미디어 파일시스템이 이를 지원하는 것은 [0151](0151-probe-rename-exchange.md)이 확인했어요.
- 끊겨서 옆으로 옮긴 채 남은 파일은 자동으로 지우지 않아요. 사용자 자막의 하나뿐인 사본일 수 있어서예요.
- 수신 영역에 받아 둔 묶음은 작업의 `받음` 상태로 알 수 있어서, 기록이 없어도 다시 받지 않아요(명세의 가정).

## 완료 기준

- [x] 파일을 쓰는 단계는 새 파일을 끝까지 쓴 뒤 이름을 바꿔 제자리에 둬요. 이름을 바꾸기 전과 뒤에 끊는 테스트가, 원래 파일이나 새 파일 중 하나가 제자리에 있는 것을 확인해요.
- [x] 교체는 `RENAME_EXCHANGE`로 맞바꾸고, 밀려난 파일이 승인한 원래 파일과 다르면 다시 맞바꿔요. 승인한 뒤 지금 파일이 바뀐 경우에, 다시 맞바꿔서 바뀐 파일이 제자리에 남는 것을 테스트가 확인해요.
- [x] 재배치의 제거나 교체가 끊겨 옆으로 옮긴 파일이 남으면, trss는 그 파일을 지우지 않아요. 테스트가 확인해요.
- [x] 앱 데이터 폴더의 `receive/.partial/`과 `subtitle-files/.tmp/`는 시작할 때 비우고, 작품 폴더의 `.trss/staging/`은 한 실행이 그 폴더에 처음 쓰기 전에 비워요. 예전 릴리스의 `.trss/tmp/`와 `receive/.tmp/`의 파일은 지우지 않아요. 테스트가 확인해요.
- [x] 받기·보관·적용·교체·재배치의 단계마다, 끊긴 뒤 `다시 시작`하면 기록 없이 그 단계를 다시 해서 끝나는 것을 테스트가 확인해요. `받음`인 묶음은 다시 받지 않아요.
- [x] 보관이나 들이기가 파일을 공개한 뒤 기록하기 전에 끊겼다가 다시 하면, 그 파일을 보관본으로 기록하고 같은 바이트를 다시 쓰지 않아요. 같은 이름에 바이트가 다른 파일이 있으면 다음 번호로 써요. 테스트가 두 경우를 확인해요.
- [x] 효과별 기록 테이블과 기록·디스크 대조 코드가 없어지고, 마이그레이션이 테이블을 지워요.
- [x] 업그레이드 때 실행 도중이던 작업은 `중단됨`이 돼요. 실행 도중의 작업과 효과별 기록이 든 지금 DB를 마이그레이션하는 테스트가 확인해요.
- [x] 같은 커밋에서 [작업과 인증 명세의 체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구)를 기록 없이 다시 하는 방식으로 고쳐 써요. 교체·재배치·임시 파일을 적은 다른 명세의 절도 고쳐요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요. 지운 테스트는 없어진 기록과 대조만 확인하던 것이어야 하고, 결과에 그 까닭과 짝지어 적어요.
- [x] 구현이 [끊긴 작업 ADR](../adr/0020-interrupted-jobs-wait-for-a-restart.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.

## 결정

### 끊긴 보관을 다시 할 때 같은 바이트의 파일

보관이나 교체 때의 들이기가 파일을 공개한 뒤 기록하기 전에 끊기면, 다시 할 때 앱이 고를 이름 후보에 기록 없는 파일이 있고 길이와 SHA-256이 같으면 그 파일을 보관본으로 기록해요(사용자 결정, 2026-10-21). 그대로 두고 다음 번호로 새로 쓰면, 화면과 보관 파일 정리에 나오지 않는 파일이 남고 보관본 이름에 `(2)`가 붙어서예요. 바이트가 다르면 지금처럼 다음 번호로 써요. 영상 옆 적용본은 이미 같은 방식(`ADOPTED`)이에요.

아래는 이 티켓을 시작하며 조사한 코드에 맞춰 정한 것이에요(2026-10-21). 사용자가 바꿀 수 있어요.

### 임시 파일과 옮긴 파일의 자리

- 작품 폴더에 쓰는 새 임시 파일은 `.trss/staging/`에 두고, 한 실행이 그 작품 폴더에 처음 쓰기 전에 비워요.
- 받기 시도의 임시 폴더는 `receive/.partial/<시도>/`이고, 앱 데이터 폴더의 `subtitle-files/.tmp/`와 함께 시작할 때 비워요.
- 교체와 재배치가 옮긴 파일은 `.trss/aside/<새 ID>`에 두고, 끊겨서 남으면 자동으로 지우지 않아요.
- 예전 릴리스의 `.trss/tmp/`와 `receive/.tmp/`는 비우지 않아요.

명세는 처음에 `.trss/tmp/`와 `receive/.tmp/`를 비운다고 적었어요. 그런데 `.trss/tmp/`에는 교체와 재배치가 옮긴 원래 자막과 보호 복사본이 있어서, 비우면 옮긴 파일을 지우지 않는다는 결정과 부딪혀요. `receive/.tmp/`에는 다시 시작한 뒤에도 남아야 하는 사이트 확인 다운로드(`check-…`)와 올리기 준비 파일이 있어요. 그래서 비울 곳을 새 폴더로 나눴어요.

### 교체

새 사본을 `.trss/staging/`에 끝까지 쓴 뒤 `.trss/aside/<새 ID>`로 옮기고, 지금 파일과 `RENAME_EXCHANGE`로 맞바꿔요. 밀려난 파일의 길이·SHA-256·inode가 승인한 원래 파일(`subtitle_replacement_paths`)과 다르면 다시 맞바꾸고 계획은 낡은 것이 돼요. 같으면 적용본과 계획의 완료를 기록한 뒤 옮긴 파일을 지워요. 관리하지 않던 원래 자막을 `제작자 알 수 없음`으로 들이기는 맞바꾸기 전에 원래 자리의 파일에서 해요. 맞바꾼 뒤에 들이면 둘 사이의 끊김이나 오류가 하나뿐인 사본을 들이지 못한 채 `.trss/aside/`에 남기기 때문이에요(2026-10-22 검토 뒤 고침: 처음에는 맞바꾼 뒤 밀려난 파일에서 들였어요).

- 목적지에 파일이 없는 추가는 지금처럼 덮어쓰지 않는 이름 바꾸기로 공개해요.
- 함께 지울 다른 경로는 `.trss/aside/<새 ID>`로 옮겨 승인한 파일인지 확인하고, 다르면 되돌려요. 계획의 완료를 기록한 뒤 지워요.
- 보호 복사본은 없어요. 밀려나거나 옮긴 원래 파일이 그 일을 해요.
- 끊긴 뒤 다시 하면 계획을 처음부터 다시 확인해요. 맞바꾼 뒤 기록 전에 끊겼으면 목적지의 새 사본은 적용본으로 알아보고(`ADOPTED`), 옮긴 원래 파일은 `.trss/aside/`에 남아요. 파일을 잃지 않는 것과 확인한 것만 기록하는 것을 지키고, 남은 경로는 다음 비교가 다시 다뤄요.

### 받기

- 받는 동안 작업 상세에 `받는 중`을 보이려고 `intended` 수신 행은 남겨요. 실행이 시작할 때 그 작업에 남은 `intended` 행을 지우고 처음부터 받아요. `fetched` 상태와 `temp_dir`·`object`·`expected_size`는 쓰지 않아요.
- `receive/<작업>/`은 앱만 쓰는 곳이라, 그 작업이 받기 전에 어느 수신 기록도 가리키지 않는 파일을 지워요. 공개와 `done` 기록 사이에 끊기면 남는 파일이에요.
- 명세의 가정에서 `받음`은 받기를 마친 수신(`done`)으로 읽어요. 그런 수신은 다시 받지 않아요.

### 다른 작업의 끝나지 않은 효과

다른 작업이 쓰는 중인 이름과 회차를 피하던 `busy_targets`와 효과 표의 유일 색인은 없애요. 실행은 한 번에 한 작업뿐이고, 기록 없이 다시 하는 단계에는 반쯤 공개된 상태가 남지 않아서예요. 그래서 다른 작업의 끊긴 적용이 회차를 가리던 드문 경우에, `보관만` 하던 것이 일반 비교와 교체 계획이 돼요.

### 재배치의 제거

제거는 `planned`에서 옮기고 확인한 뒤 지우고 `done`이 돼요. `intended`·`set_aside`(화면의 `지우는 중`)와 `folder`·`aside` 열은 쓰지 않아요. 옮긴 뒤 지우기 전에 끊기면 옮긴 파일은 `.trss/aside/`에 남고, 다시 하면 원래 경로에 파일이 없으므로 지금처럼 `이미 없어요`로 끝나요.

### 작업 상세와 정리가 읽던 효과

- 작업 상세의 `새로 받음`·`받아서 같음`과 묶음의 새 파일 수는 효과 대신 배치 계획 줄의 `made`에서 읽어요. 보관이 자산을 기록하는 트랜잭션에서 적고, 마이그레이션 66이 효과를 지우기 전에 `done` 상태의 `store` 효과가 있던 줄을 채워서 화면은 그대로예요.
- 보관 파일 정리가 끝나지 않은 작업의 파일을 지키는 것은, 효과 대신 그 작업의 배치 계획 행으로 해요.
- 교체가 자기 단계끼리 효과 표로 넘기던 상태는 한 실행 안에서 넘겨요.

### 시작 횟수와 보류

`attempts`와, 다섯 번 시작해도 끝나지 않은 작업을 보류하는 것은 남겨요. 같은 프로세스 안에서 오류로 끝난 실행이 작업을 다시 집기 때문이에요. 보류할 때 끝나지 않은 효과를 함께 보류하던 부분만 없어요. 끊긴 효과 때문에 생기던 `held`(작업, 수신, 배치 계획 행, 교체 계획, 재배치 제거)는 더 생기지 않아요.

### 마이그레이션

- `subtitle_file_effects`를 지우고, 효과만 담던 열(`subtitle_job_files`의 `temp_dir`·`object`·`expected_size`, `subtitle_relocations`의 `folder`·`aside`)과 읽는 곳이 없는 `subtitle_applied.object`를 지워요.
- 배치 계획 줄에 `made`를 더하고 `done` 상태의 `store` 효과가 있던 줄을 1로 채운 뒤에 효과 표를 지워요.
- `intended`·`fetched` 수신 행은 지워요. 끝나지 않은 재배치 제거(`intended`·`set_aside`·`held`)는 `kept`로 끝내요. 그 파일은 예전 `.trss/tmp/`에 그대로 있어요.
- 이미 `held`인 작업과 교체 계획은 그대로 둬요.
- 상태 CHECK는 고치지 않아요. 남는 행은 마이그레이션이 바꾸므로, 다른 테이블이 참조하는 테이블을 새로 만들지 않아도 돼요.

## 결과

2026-10-22에 자막 작업의 효과별 기록과, 다시 시작할 때 그 기록과 디스크를 견주던 코드를 없앴어요. 이제 받기·보관·적용·교체·재배치 제거는 끊긴 뒤 `다시 시작`하면 그 단계를 기록 없이 다시 해요. 파일은 `.trss/staging/`에 끝까지 쓴 뒤 덮어쓰지 않는 이름 바꾸기로 제자리에 두고, 교체는 `RENAME_EXCHANGE`로 맞바꿔요. 마이그레이션 66이 효과 기록 표와 효과만 담던 열을 지우고, 예전 릴리스가 남긴 상태를 끝내요. 구현이 [끊긴 작업 ADR](../adr/0020-interrupted-jobs-wait-for-a-restart.md)과 맞아서 ADR은 고치지 않았어요. 단계마다 다시 하는 순서는 [작업과 인증 명세의 체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구)와 [자막 명세의 교체 계획과 반영](../specs/subtitles.md#교체-계획과-반영)에 있어요.

### 끊긴 곳마다 다시 하는 방식

아래 표의 테스트는 모두 `trss-jobs`의 `tests/it/`에 있어요.

| 단계와 끊긴 곳 | 다시 하면 | 확인한 테스트 |
| --- | --- | --- |
| 받기, 받는 도중 | 남은 `intended` 수신 행을 지우고 처음부터 받아요. 남은 바이트는 보지 않아요. | `runner.rs` `a_cut_attempt_is_forgotten_and_its_file_received_anew_whatever_bytes_it_left` |
| 받기, 공개 뒤 `done` 기록 전 | `receive/<작업>/`에서 어느 수신 기록도 가리키지 않는 파일을 지우고 다시 받아요. | `runner.rs` `a_file_published_before_its_record_is_removed_and_received_anew_under_its_name` |
| 받기를 마친 뒤 | `done`인 수신은 다시 받지 않아요. | `interrupted.rs` `a_restarted_job_whose_package_is_in_the_receive_area_does_not_fetch_it_again` |
| 보관, 공개 전 | `.trss/staging/`을 비우고 다시 써요. | `place.rs` `a_store_cut_before_publishing_is_done_again_once_after_its_staging_folder_is_emptied` |
| 보관, 공개 뒤 기록 전 | 앱이 고를 이름 후보의 기록 없는 파일이 길이와 SHA-256까지 같으면 그 파일을 보관본으로 기록하고, 다르면 그 파일을 두고 다음 번호로 써요. | `place.rs` `a_store_cut_after_publishing_records_the_file_it_left_without_writing_it_again`, `a_file_of_other_bytes_under_the_name_after_a_cut_is_left_and_the_next_number_taken` |
| 교체 때의 들이기, 공개 뒤 기록 전 | 보관과 같아요. | `replace.rs` `an_import_finds_the_file_an_import_cut_short_published_and_records_it` |
| 적용, 공개 뒤 기록 전 | 영상 옆의 같은 바이트 파일을 적용본으로 기록해요(`ADOPTED`). | `place.rs` `an_apply_renamed_before_its_record_is_recorded_once` |
| 교체, 맞바꾸기 전 | 계획을 처음부터 다시 해요. 이미 들인 원래 자막은 한 번만 기록해요. | `replace.rs` `a_replacement_cut_before_its_swap_is_carried_out_anew_and_leaves_old_folders_alone`, `a_replacement_cut_after_importing_the_original_imports_it_once_and_replaces_once` |
| 교체, 맞바꾼 뒤 기록 전 | 계획은 낡은 것이 되고, 새 사본은 적용본으로 기록해요(`ADOPTED`). 밀려난 원래 파일은 `.trss/aside/`에 남아요. | `replace.rs` `a_replacement_cut_after_its_swap_records_the_new_copy_and_keeps_the_original_aside` |
| 교체, 앞선 적용본을 옮긴 뒤 기록 전 | 바로 위와 같고, 옮긴 적용본도 `.trss/aside/`에 남아요. | `replace.rs` `a_replacement_cut_after_taking_off_an_earlier_copy_leaves_that_file_aside` |
| 교체, 기록 뒤 옮긴 파일을 지우기 전 | 계획은 끝났고, 옮긴 파일은 `.trss/aside/`에 남아요. | `replace.rs` `a_replacement_cut_after_its_records_leaves_its_files_aside` |
| 재배치 제거, 옮기기 전 | 다시 살펴보고 지워요. | `relocate.rs` `a_removal_cut_before_its_rename_takes_the_copy_off_when_it_is_done_again` |
| 재배치 제거, 옮긴 뒤나 지운 뒤 기록 전 | 원래 경로가 비어 있어 `이미 없어요`로 `done`이 돼요. 옮긴 파일은 `.trss/aside/`에 남아요. | `relocate.rs` `a_removal_cut_after_its_rename_or_its_removal_ends_done_and_leaves_what_is_aside` |

교체는 맞바꾼 뒤 밀려난 파일의 길이·SHA-256·inode를 승인한 원래 파일과 견주고, 다르면 다시 맞바꿔 바뀐 파일을 제자리에 둬요. 테스트는 테스트 빌드에만 있는 훅 `trss_core::files::testing::before_rename_of`로 맞바꾸기 바로 전에 지금 파일을 바꿔요. `replace.rs`의 `a_subtitle_changed_right_before_the_swap_is_swapped_back_and_the_new_copy_goes`와 `an_unmanaged_subtitle_changed_right_before_the_swap_stays_and_its_import_is_kept`가 바뀐 파일이 제자리에 남고 새 사본이 지워지는 것을 확인해요. 맞바꾸기 자체는 `place/aside.rs`의 단위 테스트 `a_file_changed_after_approval_is_swapped_back_and_stays_in_its_place`가 확인해요.

같은 훅으로, 적용이 공개하기 바로 전에 다른 파일이 그 이름을 차지하면 그 파일을 두고 교체 승인을 기다리는 것도 확인해요(`place.rs` `a_file_that_takes_the_name_right_before_an_apply_publishes_is_kept_for_a_replacement_to_approve`).

### 임시 파일과 옮긴 파일

| 폴더 | 비우는 때 | 확인한 테스트 |
| --- | --- | --- |
| 작품 폴더의 `.trss/staging/` | 한 실행이 그 폴더에 처음 쓰기 전. 지우지 못한 항목은 로그에 남기고 둔 채 이어 해요. | `trss-jobs` `tests/it/place.rs` `a_store_cut_before_publishing_is_done_again_once_after_its_staging_folder_is_emptied`, `an_entry_of_the_staging_folder_that_cannot_be_removed_stays_and_the_store_goes_on` |
| 앱 데이터 폴더의 `receive/.partial/` | 시작할 때, 웹을 열기 전 | `trss` `tests/it/worker_process.rs` `the_start_empties_the_receive_attempts_folder_before_the_web_serves`, `trss-jobs` `tests/it/interrupted.rs` `the_start_empties_the_receive_attempts_folder_and_leaves_the_rest_of_the_area` |
| 앱 데이터 폴더의 `subtitle-files/.tmp/` | 시작할 때, 웹을 열기 전 | `trss` `tests/it/worker_process.rs` `the_start_empties_the_stored_files_temporary_folder_before_the_web_serves` |
| 작품 폴더의 `.trss/aside/` | 비우지 않아요. | `trss-jobs` `tests/it/replace.rs` `a_replacement_cut_after_its_records_leaves_its_files_aside`, `tests/it/relocate.rs` `a_removal_cut_after_its_rename_or_its_removal_ends_done_and_leaves_what_is_aside` |
| 예전 릴리스의 `.trss/tmp/` | 비우지 않아요. | `trss-jobs` `tests/it/place.rs` `a_store_cut_before_publishing_is_done_again_once_after_its_staging_folder_is_emptied`, `tests/it/replace.rs` `a_replacement_cut_before_its_swap_is_carried_out_anew_and_leaves_old_folders_alone` |
| 예전 릴리스의 `receive/.tmp/` | 비우지 않아요. | `trss-jobs` `tests/it/interrupted.rs` `the_start_empties_the_receive_attempts_folder_and_leaves_the_rest_of_the_area` |

### 마이그레이션 66

마이그레이션 66(`jobs/no_effect_records.sql`)은 효과 기록 표와 효과만 담던 열을 지우고, 받는 도중이던 수신 행과 실패한 수신의 경로를 지우고, 지우는 도중이던 재배치 제거와 보류한 제거를 `kept`로 끝내요. 표를 지우기 전에 배치 계획 줄의 `made`를 효과에서 채워서, 작업 상세의 `새로 받음`·`받아서 같음`은 예전 작업에도 그대로 보여요. 자세한 규칙은 [작업과 인증 명세의 체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구) 끝에 있어요.

- `trss-core` `src/db.rs`의 `the_effect_records_go_and_what_an_earlier_release_left_under_way_ends`와 `a_plan_row_made_its_file_when_it_had_a_done_store_effect`가 바뀌는 행을 확인해요. `trss-jobs` `tests/it/interrupted.rs` `a_job_an_earlier_release_left_running_with_its_effects_is_interrupted_by_the_upgrade`는 실행 도중의 작업과 효과 기록이 든 DB를 올린 뒤 그 작업이 `중단됨`이 되는 것을 확인해요.
- 2026-10-22에 리뷰어가 개발 DB(스키마 65)의 사본에 `made`를 더하기 전의 마이그레이션 66을 돌렸어요. 스키마 66이 됐고 `foreign_key_check`는 비었고 `integrity_check`는 `ok`였어요. 스키마는 지운 표와 열, 다시 만든 `subtitle_relocations`만 달랐고, 다른 표의 행은 남은 열로 견주어 모두 같았어요.
- 개발 데이터에는 마이그레이션이 바꿀 행이 없었어요. 수신 8개는 `done` 7개와 경로 없는 `abandoned` 1개였고 재배치는 없었어요. 그래서 리뷰어가 예전 상태의 수신·재배치 행과 `intended` 효과를 넣은 사본에도 돌렸고, 각 행이 위 규칙대로 바뀌었어요(2026-10-22).
- 같은 날 `made`를 더한 마이그레이션을 같은 개발 DB 사본에 돌렸어요. 스키마 66이 됐고 `foreign_key_check`는 비었고 `integrity_check`는 `ok`였어요. 배치 계획 7줄의 `made`가 모두 1이어서, 예전 효과로 셈한 값과 같았어요. 원본은 이 PC의 `dev/local/measure/0153/made-migration-dev-copy.txt`에 있어요.

### 바뀐 동작

결정에 적은 것 말고도 드문 경우의 동작이 달라졌어요. 명세의 결정에도 적었어요.

- 재배치 제거는 사본이 제자리에 없는지를, 보관본이 그 회차로 돌아왔는지보다 먼저 봐요. 그래서 사본이 없어지고 보관본이 그 회차로 돌아온 제거는 `kept`가 아니라 `done`으로 끝나고 재배치를 새로 맞춰요. 다시 적용하려면 사람이 확인해야 해요. 기록이 없으면 끊긴 실행이 사본을 옮긴 것과 사람이 지운 것을 가리지 못해서예요.
- 마이그레이션 뒤에는 예전 릴리스가 지우다 남긴 사본을 나중의 재배치가 다시 계획할 수 있어요. 제거는 바이트부터 견주므로 다른 파일을 지우지 않아요.
- 끊긴 적용이나 교체를 다시 하면, 그 줄의 기록에 평소의 적용 문장 대신 `ADOPTED` 문장(`이 회차에 같은 자막이 이미 있어 그 파일을 적용본으로 기록했어요`)이 남아요.
- 교체에는 `RENAME_EXCHANGE`가 꼭 있어야 해요. 지원하지 않는 파일시스템에서는 교체가 모두 실패하고, 다른 방법으로 하지 않아요. 서버의 미디어 파일시스템은 [0151](0151-probe-rename-exchange.md)이 확인했어요.
- 관리하지 않던 원래 자막은 맞바꾸기 전에 들여요. 그래서 맞바꾸기 바로 전에 파일이 바뀌어 다시 맞바꾸면, `제작자 알 수 없음`에 들인 보관본이 하나 더 남아요.

### 남은 한계

- 교체가 앞선 적용본을 `.trss/aside/`로 옮긴 뒤 기록하기 전에 끊기면, 그 적용본의 기록은 없는 파일을 가리킨 채 살아 있어요. 보관 파일 정리의 `APPLIED` 규칙이 그 보관본을 계속 지켜서 정리할 수 없어요. 대소문자를 가리는 파일시스템에서 대소문자만 다른 이름으로 바꾸는 교체가 이름을 바꾼 뒤 끊긴 경우도 같아요.
- `.trss/aside/`에 남은 파일은 화면에 보이지 않아요. ADR대로 사람이 다뤄요.
- 마이그레이션이 실패한 수신의 경로를 지우므로, 그 바이트는 그 작업이 다시 받기를 시작할 때만 지워져요. 다시 받지 않는 작업의 바이트는 `receive/<작업>/`에 남아 디스크만 차지해요.
- 예전 릴리스에서 교체 효과가 끝나지 않은 채 승인된 계획은 업그레이드 뒤 낡은 계획이 되고, 새 자막은 첫 적용본으로 놓여요. 그 원래 파일과 보호 복사본은 예전 `.trss/tmp/`에 남아요. 예전 흐름은 영상 옆을 바꾸기 전에 들였으므로 들이지 못한 사본은 없어요. 서버에 그런 효과가 있는지는 [0162](0162-deploy-lighter-structure.md)가 올리기 전에 서버 DB 사본에서 세요.
- 앱 데이터가 다른 두 trss가 같은 미디어 폴더를 쓰면, 한쪽이 `.trss/staging/`을 비우며 다른 쪽이 쓰는 중인 파일을 지울 수 있어요. 지원하지 않는 구성이에요.

### 지운 테스트와 까닭

0153 전의 테스트 가운데 37개를 지우고, 4개를 고쳐 쓰고, 2개는 이름만 바꾸고, 1개에서는 한 경우를 뺐어요. 지운 테스트는 모두 없어진 효과·수신 기록과 디스크의 대조를 확인하거나, [결정](#다른-작업의-끝나지-않은-효과)으로 없앤 다른 작업의 효과 피하기를 확인하던 것이에요. 기능 규칙을 확인하는 다른 테스트는 모두 남아 통과하고, 고쳐 쓴 테스트는 넣은 기록 대신 끊긴 실행으로 같은 규칙을 확인해요. 표의 테스트는 따로 적지 않으면 `trss-jobs`의 `tests/it/`에 있어요.

| 테스트 | 확인하던 것 | 지금 확인하는 테스트 |
| --- | --- | --- |
| `runner.rs` `an_intent_with_no_bytes_is_abandoned_and_received_anew` | 대조가 임시 파일 없는 `intended` 수신을 `abandoned`로 끝냄 | `runner.rs` `a_cut_attempt_is_forgotten_and_its_file_received_anew_whatever_bytes_it_left`(바이트 없음) |
| `runner.rs` `bytes_known_to_stop_short_are_received_again_and_whole_ones_are_published_as_they_are` | 대조가 임시 바이트를 기록한 `expected_size`와 견줌 | 같은 테스트(짧은 바이트, 다 받은 바이트) |
| `runner.rs` `bytes_with_no_announced_length_or_more_than_it_are_held_not_received_again` | 대조가 보증할 수 없는 바이트의 수신을 보류함 | 같은 테스트(파일보다 긴 바이트). 작업의 보류는 `runner.rs` `a_done_file_is_reused_after_its_bytes_are_checked_and_held_when_they_changed` |
| `runner.rs` `a_rename_done_before_its_record_is_found_by_its_object_and_bytes` | 대조가 공개된 `fetched` 파일을 기록한 객체와 바이트로 알아봄 | `runner.rs` `a_file_published_before_its_record_is_removed_and_received_anew_under_its_name` |
| `runner.rs` `a_fetched_file_is_published_but_a_copy_with_the_same_bytes_at_its_path_is_held` | `fetched` 수신의 대조 | 같은 테스트 |
| `runner.rs` `a_fetched_file_is_found_by_its_object_whatever_device_number_it_was_mounted_with` | 수신 대조가 객체를 inode로만 견줌 | 없음. 수신은 객체를 기록하지 않아요. inode 비교는 `trss-core` `file_id`의 테스트가 확인해요. |
| `runner.rs` `an_unfinished_receipt_is_settled_from_the_disk_even_when_the_post_no_longer_opens` | 대조가 게시물을 열기 전에 돎 | `runner.rs` `what_a_cut_run_left_is_cleared_even_when_the_post_no_longer_opens` |
| `runner.rs` `an_empty_temporary_file_is_no_bytes_even_when_nothing_was_announced` | 대조가 빈 임시 파일을 읽는 방식 | `runner.rs` `a_cut_attempt_is_forgotten_and_its_file_received_anew_whatever_bytes_it_left`(빈 파일) |
| `runner.rs` `recovered_bytes_that_are_a_web_page_fail_as_not_a_file_and_are_received_anew` | 대조가 되살린 바이트를 확인함 | 같은 테스트(웹 페이지가 남은 경우). `done` 전의 확인은 `tistory.rs` `an_error_page_instead_of_the_attachment_fails_as_not_a_file_with_its_reason` |
| `runner.rs` `a_failure_recorded_before_its_bytes_went_is_finished_by_the_next_start` | 경로를 가진 `failed` 수신의 바이트를 지우는 `finish_failed` | 없음. 실패는 경로를 기록하지 않아요. |
| `runner.rs` `a_failure_whose_bytes_were_left_is_finished_even_when_the_post_is_gone` | 게시물을 열기 전의 `finish_failed` | 없음. `finish_failed`와 함께 없어졌어요. |
| `runner.rs` `bytes_that_cannot_be_removed_hold_the_receipt_instead_of_counting_as_gone` | 대조가 되살린 바이트를 지우지 못하면 수신을 보류함 | 없음. 확인 전에 바이트를 되살리거나 공개하지 않아요. |
| `runner.rs` `a_file_published_before_its_record_is_removed_and_received_anew_under_its_name`의 `legacy` 경우 | 경로를 적은 `fetched` 수신 행 | 마이그레이션 66이 그 행을 지워요. `trss-core` `src/db.rs` `the_effect_records_go_and_what_an_earlier_release_left_under_way_ends` |
| `place.rs` `a_store_intended_before_a_kill_is_done_anew_once` | 대조가 `intended` 보관 효과를 버리고 `.trss/tmp/`의 임시 파일을 지움 | `place.rs` `a_store_cut_before_publishing_is_done_again_once_after_its_staging_folder_is_emptied` |
| `place.rs` `a_prepared_store_is_published_and_one_already_renamed_is_found_by_its_object` | 대조가 `prepared` 보관을 공개하거나 이미 공개한 파일을 객체로 알아봄 | 위 테스트(공개 전), `place.rs` `a_store_cut_after_publishing_records_the_file_it_left_without_writing_it_again`(공개 뒤, 바이트로 알아봄) |
| `place.rs` `a_prepared_store_is_found_by_its_object_whatever_device_number_it_was_mounted_with` | 보관 대조가 객체를 inode로만 견줌 | 없음. 교체가 쓰는 비교는 `trss-core` `file_id.rs` `a_recorded_file_is_the_same_file_whatever_its_device_number` |
| `place.rs` `a_prepared_effect_whose_files_are_not_what_it_wrote_is_held` | 대조가 목적지에 같은 바이트의 다른 객체가 있는 보관을 보류함 | 사용자 결정으로 그 파일을 보관본으로 기록해요. `place.rs` `a_store_cut_after_publishing_records_the_file_it_left_without_writing_it_again` |
| `place.rs` `a_name_another_jobs_store_is_about_to_take_is_taken_whatever_its_case` | 다른 작업의 끝나지 않은 보관이 쓸 이름을 피함 | 없음. 결정으로 없앴어요. 디스크에 있는 이름은 `place.rs` `a_name_taken_by_other_bytes_is_numbered_and_the_same_bytes_are_one_file` |
| `place.rs` `an_episode_another_jobs_apply_is_about_to_take_keeps_it` | 다른 작업의 끝나지 않은 적용이 가리는 회차에 `보관만` 함 | 없음. 결정으로 없앴어요. |
| `place.rs` `a_job_held_after_its_starts_holds_its_effects_and_frees_their_names` | 시작 횟수로 보류할 때 보관 효과도 보류하고 이름을 비움 | 시작 횟수의 보류는 `runner.rs` `a_job_started_too_many_times_is_held_instead_of_blocking_the_line`, 재배치는 `relocate.rs` `a_relocation_held_for_its_starts_keeps_the_copies_it_did_not_take_off` |
| `replace.rs` `killed_after_claiming_the_effects_carries_the_plan_out_anew` | 대조가 교체의 `intended` 효과를 버리고 계획을 처음부터 함 | `replace.rs` `a_replacement_cut_before_its_swap_is_carried_out_anew_and_leaves_old_folders_alone` |
| `replace.rs` `killed_after_the_protective_copy_carries_the_plan_out_anew` | 대조가 `prepared` 효과를 버리고 새 사본의 임시 파일과 보호 복사본을 지움 | 같은 테스트. 보호 복사본은 없어요. |
| `replace.rs` `killed_after_setting_the_old_copy_aside_publishes_the_new_one` | 대조가 `set_aside` 제거(목적지가 빔)에서 공개로 이어 감 | `replace.rs` `a_replacement_cut_after_its_swap_records_the_new_copy_and_keeps_the_original_aside`. 맞바꾸기는 목적지를 비우지 않아요. |
| `replace.rs` `killed_after_setting_aside_but_before_its_record_is_found_set_aside` | 대조가 기록 없이 옮긴 파일을 계획의 객체로 찾음 | 같은 테스트 |
| `replace.rs` `killed_after_publishing_before_its_record_records_it_once` | 대조가 목적지의 `prepared` 적용을 객체로 기록함 | 같은 테스트(`ADOPTED`) |
| `replace.rs` `killed_after_the_plan_was_done_cleans_up` | 다시 시작이 끝난 계획의 보호 복사본과 옮긴 파일을 효과에서 찾아 지움 | `replace.rs` `a_replacement_cut_after_its_records_leaves_its_files_aside`. 끊긴 실행이 옮긴 파일은 지우지 않아요. |
| `replace.rs` `killed_while_cleaning_up_removes_what_is_left` | 파일 하나가 이미 없는 같은 정리 | 같은 테스트 |
| `replace.rs` `a_job_held_before_its_clean_up_keeps_its_replacement_done` | 보류가 끝난 계획의 정리 효과를 건드리지 않음 | `replace.rs` `a_job_held_for_its_starts_leaves_its_approved_plan_and_its_files` |
| `replace.rs` `killed_after_importing_before_its_record_records_the_import_once` | 대조가 `prepared` 들이기 효과를 기록함 | `replace.rs` `an_import_finds_the_file_an_import_cut_short_published_and_records_it` |
| `replace.rs` `killed_after_setting_an_earlier_applied_copy_aside_before_its_record_ends_the_replacement` | 대조가 기록 없이 옮긴 앞선 적용본의 제거를 기록하고 계획을 끝냄 | `replace.rs` `a_replacement_cut_after_taking_off_an_earlier_copy_leaves_that_file_aside` |
| `replace.rs` `an_old_copy_set_aside_that_is_not_the_one_compared_is_held_with_its_copies` | 대조가 옮긴 파일이 견준 파일이 아닌 계획을 보류함 | 한 실행 안에서 맞바꾼 뒤 확인하고 되돌려요. `replace.rs` `a_subtitle_changed_right_before_the_swap_is_swapped_back_and_the_new_copy_goes`, `place/aside.rs` `a_file_changed_after_approval_is_swapped_back_and_stays_in_its_place` |
| `replace.rs` `a_job_held_while_its_old_copy_is_aside_holds_the_plan_and_its_effects` | 시작 횟수로 보류할 때 승인된 계획과 효과도 보류함 | 고쳐 씀: `replace.rs` `a_job_held_for_its_starts_leaves_its_approved_plan_and_its_files`(계획은 승인된 채 남아요) |
| `replace.rs` `killed_then_mounted_again_with_another_device_number_carries_the_replacement_on` | 끊긴 곳마다 효과 대조가 다시 마운트해도 객체를 inode로 견줌 | `replace.rs` `an_approved_plan_mounted_again_with_another_device_number_is_carried_out`(계획의 객체만 남아요) |
| `replace.rs` `no_room_for_the_copies_stops_before_anything_beside_the_video_changes` | 자리가 없으면 영상 옆이 바뀌기 전에 멈춤 | 이름 바꿈: `no_room_for_the_new_copy_stops_before_anything_beside_the_video_changes`. 새 사본만 써서 `.trss/staging/`을 읽기 전용으로 만들고, 효과 행 단언을 뺐어요. |
| `replace.rs` `an_unmanaged_subtitle_that_cannot_be_imported_is_swapped_back_and_the_plan_fails` | 들이지 못한 원래 자막을 되돌리고 계획이 실패함 | 이름 바꿈: `an_unmanaged_subtitle_that_cannot_be_imported_fails_the_plan_before_anything_changes`. 들이기가 맞바꾸기 전이라 되돌릴 것이 없어요. |
| `relocate.rs` `a_copy_renamed_aside_before_its_record_said_so_is_removed` | 대조가 `intended` 제거의 사본을 기록한 `.trss/tmp/<ID>.aside`에서 찾아 지움 | `relocate.rs` `a_removal_cut_after_its_rename_or_its_removal_ends_done_and_leaves_what_is_aside`(`이미 없어요`로 `done`, 파일은 `.trss/aside/`에 남음) |
| `relocate.rs` `another_file_found_aside_goes_back_on_its_path` | 대조가 기록한 자리의 다른 파일을 되돌림 | 실행 안의 확인은 `place/aside.rs` `a_file_changed_before_the_rename_is_put_back_where_it_was`, 바뀐 사본은 `relocate.rs` `a_copy_a_person_changed_stays_where_it_is` |
| `relocate.rs` `a_copy_set_aside_and_already_removed_is_done` | 파일이 없는 `set_aside` 제거를 `done`으로 기록함 | `relocate.rs` `a_removal_cut_after_its_rename_or_its_removal_ends_done_and_leaves_what_is_aside`(지운 뒤) |
| `relocate.rs` `a_path_a_removal_is_taking_off_is_busy_for_other_effects` | `intended` 제거의 경로를 다른 효과가 피함(`busy_targets`) | 없음. 결정으로 없앴어요. |
| `relocate.rs` `a_copy_intended_but_not_renamed_is_taken_off_anew` | 옮기기 전에 끊긴 제거를 다시 함 | 고쳐 씀: `relocate.rs` `a_removal_cut_before_its_rename_takes_the_copy_off_when_it_is_done_again`(넣은 `intended` 기록 대신 끊긴 실행으로) |
| `relocate.rs` `a_copy_being_taken_off_does_not_count_as_applied_when_its_episode_comes_back` | `set_aside` 사본을 적용본으로 세지 않음 | 고쳐 씀: `relocate.rs` `a_copy_a_cut_run_moved_aside_is_applied_again_when_its_episode_came_back`(제거를 끝내는 기록이 재배치를 새로 맞춰요) |
| `relocate.rs` `a_relocation_cut_short_too_often_holds_its_removals_under_way` | 보류가 `set_aside` 제거를 보류하고 계획한 제거는 둠 | 고쳐 씀: `relocate.rs` `a_relocation_held_for_its_starts_keeps_the_copies_it_did_not_take_off`(끝나지 않은 제거는 모두 `kept`) |
| `relocate.rs` `a_removal_an_earlier_release_left_under_way_is_kept_and_not_planned_again` | 예전 `intended`·`set_aside` 제거 행을 읽는 방식 | 마이그레이션 66이 그 행을 `kept`로 바꿔요. `trss-core` `src/db.rs` `the_effect_records_go_and_what_an_earlier_release_left_under_way_ends`, `interrupted.rs` `a_job_an_earlier_release_left_running_with_its_effects_is_interrupted_by_the_upgrade` |
| `relocate.rs` `a_copy_whose_device_number_changed_since_it_was_applied_is_taken_off` | `subtitle_applied.object`의 장치 번호가 바뀌어도 바이트로 지움 | 없음. 읽는 곳이 없던 그 열을 지웠고, 재배치는 길이와 SHA-256만 견줘요. |

### 확인

- 2026-10-22에 `made`를 더한 코드로 `cargo test --locked --workspace -j 4`가 통과 3,146개, 실패 0개, 건너뛴 것 11개였고, 테스트 바이너리의 시간 합은 44.85초였어요. [0158](0158-generated-types-library.md) 뒤의 3,144개보다 2개 많아요. 원본 로그는 이 PC의 `dev/local/measure/0153/`에 있어요.
- 같은 날 `cargo clippy --workspace --all-targets --all-features -j 4`에 경고가 없었고, 웹의 `npm test`(285개)와 `npm run typecheck`가 통과했어요.
- 테스트가 잘못된 코드를 잡는지 코드를 일부러 바꿔 봤어요(2026-10-22). 다시 맞바꾼 뒤 새 사본 대신 제자리의 파일을 지우게 하자 `replace.rs`의 다시 맞바꾸는 테스트 둘이 실패했어요. 다시 쓴 자산을 `made`로 적게 하자 테스트 7개가, 새로 만든 자산을 `made`로 적지 않게 하자 `place.rs`의 단언이 실패했어요.
- 0153을 시작하기 전 커밋과 견주면, 테스트 파일을 뺀 Rust와 웹 소스에서 2,754줄을 지우고 2,601줄을 더해 153줄이 줄었어요. 테스트 파일에서는 1,502줄을 지우고 1,372줄을 더했어요. 모든 줄을 셌고, 소스 파일 안의 단위 테스트와 마이그레이션 테스트는 소스 쪽에 들어 있어요(2026-10-22).

### 리뷰

`stora:complex-reviewer`가 단계마다 끊긴 곳과, 같은 프로세스 안에서 오류 뒤 다시 집는 경우를 따라갔어요. 파일을 잃거나 덮어쓰지 않는지, 경로가 비지 않는지, 확인하지 않은 것을 기록하지 않는지를 봤고 막는 결함은 없었어요. 나온 것 여섯 가지 중 셋과 테스트 빈틈 하나를 고쳤고, 셋은 남은 한계로 적었어요.

- 관리하지 않던 원래 자막을 맞바꾼 뒤에 들여서, 그사이에 끊기거나 DB 오류가 나면 하나뿐인 사본이 들여지지 않은 채 `.trss/aside/`에 남을 수 있었어요. 오류일 때는 다시 맞바꾸기도 건너뛰었어요. 맞바꾸기 전에 원래 자리에서 들이게 고쳤어요([결정의 교체](#교체)).
- `.trss/staging/`의 항목 하나를 지우지 못하면 그 작품 폴더의 보관·적용·교체가 모두 실패했어요. 지우지 못한 항목은 두고 이어 하게 고쳤어요.
- 작업 상세의 `새로 받음`을 자산과 묶음의 날짜와 순서로 짐작해서, 다른 작업이 그사이 같은 바이트를 보관한 경우 등에 `받아서 같음`을 `새로 받음`으로 보였어요. 보관이 `made`를 적게 고쳤어요.
- 다시 맞바꾸는 경로에 통합 테스트가 없었어요. 테스트 빌드에만 있는 훅과 테스트를 더했어요.
- 남은 한계로 적은 셋은 끊긴 교체가 남기는 살아 있는 적용본 기록, 실패한 수신의 바이트, 예전 릴리스의 끝나지 않은 교체 효과예요.

리뷰어는 앞의 두 수정과 훅을 다시 읽어 고쳐진 것을 확인했고, 훅을 쓴 테스트는 위의 바꿔 보기로 확인했어요. `made` 수정은 다시 리뷰하지 않았어요.

### 확인하지 않은 것

- 단계를 실행하는 도중에 실제로 `trss`를 죽였다 띄운 적은 없어요(2026-10-22). 테스트는 끊긴 실행이 남기는 디스크와 DB를 만들거나, 훅으로 그 자리에서 파일을 바꿔요.
- 같은 프로세스 안에서 오류로 끝난 실행이 작업을 다시 집는 경우는 테스트하지 않았어요(2026-10-22). 다시 시작한 뒤와 같은 길을 가요.
- 적용이 스테이징과 공개 사이에 회차 대응이 바뀐 것을 알아보는 확인은 테스트하지 않았어요(2026-10-22). 예전에는 넣은 `prepared` 효과로 닿던 곳이에요.
- 들이기가 끊긴 자리에 바이트가 다른 파일이 있는 경우와, 들이기가 원래 파일이 바뀐 것을 보고 계획을 낡은 것으로 끝내는 경우는 테스트하지 않았어요(2026-10-22). 앞의 것은 보관과 같은 이름 고르기를 써요.
- 대소문자만 다른 이름으로 바꾸는 교체가 끊긴 경우, 실패한 제거가 계획을 보류하는 경우, 동기화나 다시 맞바꾸기가 실패해 결과를 모르는 보류는 통합 테스트가 없어요(2026-10-22). 맞바꾸기와 동기화 실패는 `place/aside.rs`의 단위 테스트가 확인해요.
- 서버 DB에는 마이그레이션 66을 돌리지 않았어요(2026-10-22). [0162](0162-deploy-lighter-structure.md)가 올리기 전에 해요.
- 개발 환경에서 `trss`를 띄워 화면을 보지 않았어요(2026-10-22). 화면에서 바뀐 것은 재배치의 `지우는 중`이 없어진 것뿐이고, 웹 테스트와 typecheck로만 확인했어요.
- `RENAME_EXCHANGE`는 서버의 xfs 작품 폴더에서만 확인했어요(2026-10-21, [0151](0151-probe-rename-exchange.md)). NFS나 SMB 같은 다른 파일시스템에서는 보지 않았어요.
