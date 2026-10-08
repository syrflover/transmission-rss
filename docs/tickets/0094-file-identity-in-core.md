# 0094 같은 파일 알아보기를 trss-core 하나로 모아요

- 상태: 완료 (2026-10-08)
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

"기록한 파일과 지금 파일이 같은 파일인가"를 6곳에서 따로 판단해요. 2026-10-07에 본 곳이에요.

- trss-jobs의 `area`(`object_of`, `same_object`). 저장 형식은 `dev:ino`이고, 예전 생성 시각 꼬리 `:<ns>`도 읽어요. 배치, 교체, 재배치, 받기, 올리기가 써요.
- trss-collect의 `revision`(`FileIdentity`). 저장 형식은 `dev:ino:len:mtime:ctime`이고, 수정본 기록과 회차 변환 되돌리기가 써요. `revisions`의 `owner_of`와 `same_folder`는 같은 순간의 두 stat을 장치와 inode로 견줘요.
- trss-library의 표지(`found_at`). `artwork_files`의 `dev`와 `ino` 열에 저장해요.

재부팅 뒤 장치 번호가 바뀌는 문제와 musl에서 생성 시각을 읽지 못하는 문제를 세 곳이 따로 고쳤어요(`a2f2fa6`, `7eff742`, `c050eba`, `aa2944e`, `6c175f1`).
inode만 견준다는 규칙과, 다른 파일 시스템의 같은 inode를 같은 파일로 볼 수 있는 남은 위험은 사용자 결정이에요(2026-10-06·07). 이 규칙을 trss-core의 한 부품이 지켜요.

- 세 저장 형식을 모두 읽어서 마이그레이션 없이 바꿔요.
- 내용 해시, CRC, 크기, 수정 시각, ctime 같은 추가 검사는 지금 그 검사를 하는 기능 크레이트에 남아요. 견주는 항목이 곳마다 다른 것(trss-jobs의 배치 영상은 수정 시각만, trss-collect는 ctime까지)은 동작이므로 그대로 둬요.
- trss-library의 감시 폴더 발견에 있는 `FileIdentity`(크기와 수정 시각)는 다른 개념이에요. 저장 값은 그대로 두고, 같은 이름이 헷갈리지 않게 타입 이름만 바꿔요.

## 완료 기준

- 같은 파일 판단의 구현이 trss-core에 하나 있고, trss-jobs, trss-collect, trss-library는 그것을 불러요. 예전 판단 함수는 없어요.
- trss-core에 세 저장 형식과 예전 생성 시각 꼬리를 모두 읽는 테스트, 장치 번호만 다른 기록을 같은 파일로 보는 테스트가 있어요.
- 기존 테스트가 고치지 않고 통과해요. 그 뒤 재마운트 테스트 11개 중 inode 규칙만 다시 확인하는 것은 [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)대로 정리하고, 재시작 경로를 확인하는 것은 남겨요. 앞뒤 개수를 결과 절에 적어요.

## 결과

### 바꾼 것

- trss-core에 `file_id` 모듈을 더했어요. `FileId`(장치 번호와 inode)가 같은 파일 판단을 맡아요.
  - `FileId::same_file`은 기록한 파일과 지금 찾은 파일을 inode로만 견줘요. 사용자 결정(2026-10-06·07)의 규칙과 남은 위험을 모듈 문서에 적었어요.
  - `FileId::same_file_now`는 같은 순간에 읽은 두 stat을 장치 번호와 inode로 견줘요. 같은 마운트 상태에서 읽은 값이라 장치 번호가 다른 파일 시스템을 가려 줘요.
  - `FileId::parse`는 세 저장 형식의 앞 두 칸을 읽어요. `dev:ino`, 예전 생성 시각 꼬리가 붙은 `dev:ino:<ns>`, 수정본의 `dev:ino:len:mtime.ns:ctime.ns`예요. 표지의 `dev`·`ino` 열은 `FileId::new`로 읽어요. 쓰는 형식은 그대로예요.
  - `same_recorded_file`은 저장한 글자 둘에 이 규칙을 써요.
- 부르는 곳이에요.
  - trss-jobs: 배치, 교체, 작업 실행기의 `same_object`가 `same_recorded_file`이 됐어요. `area::object_of`는 판단 없이 글자를 쓰는 한 줄로 남아요. 열었는 사이 바뀐 파일을 가리는 `read_facts`와 `open_regular`는 `same_file_now`를 써요.
  - trss-collect: `FileIdentity`가 `FileId`를 가지고, 회차 변환 되돌리기는 `.id().same_file(…)`을 불러요. `unchanged`는 같은 파일 판단에 크기와 ctime 검사를 더한 것이에요. `owner_of`와 `same_folder`는 `same_file_now`를 써요.
  - trss-library: 표지의 `found_at`은 `FileRow::recorded_id()`와 `same_file`로 판단해요.
- 감시 폴더 발견의 `FileIdentity`(크기와 수정 시각)는 `SeenFile`로 이름만 바꿨어요(`a43ccfe`). 이 타입을 이름으로 가져다 쓰는 trss-library와 trss-web의 테스트 두 파일은 이름만 따라 바꿨어요.
- 내용 해시, CRC, 크기, 수정 시각, ctime 검사와 곳마다 다른 견주는 항목은 기능 크레이트에 그대로 있어요. 변하지 않는 값을 담아 두는 표지와 감시 폴더의 `Stamp`, 수정본 `FileIdentity`의 `==`(장치 번호가 다르면 CRC를 다시 읽어요)도 같은 파일 판단이 아니라서 그대로예요.

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요.

- 앞은 통과 2,819개, 실패 0개, 무시 14개였어요.
- 1단계(`c2944a4`) 뒤에는 trss-core의 테스트 9개를 더해 2,828·0·14였어요. 기존 테스트는 위의 이름 바꾸기 말고는 고치지 않았어요. 더한 테스트는 세 저장 형식과 생성 시각 꼬리 읽기, inode가 없는 글자, 장치 번호만 다른 기록은 같은 파일이고 inode가 다르면 아닌 것, `same_file_now`, 실제 파일 시스템의 hard link와 이름 바꾸기예요.
- 테스트를 정리한 뒤에는 2,825·0·14예요.

장치 번호가 바뀐 경우를 다루는 테스트 11개를 [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)대로 정리해서 8개가 남았어요. 규칙을 trss-core에서 바꾸고, 부르는 곳마다 따로 바꿔 보면서 어떤 테스트가 어느 곳에 닿는지 확인했어요.

| 테스트 | 결정 | 까닭 |
| --- | --- | --- |
| trss-jobs `area.rs`의 `an_object_is_the_same_file_whatever_its_device_number` | 지움 | 규칙만 다시 확인했고, 같은 단언이 trss-core에 있어요. |
| trss-collect `revision.rs`의 `a_kept_identity_is_the_same_file_whatever_its_device_number` | 남김, 줄여서 이름을 바꿈 | `same_file` 단언을 빼고, 장치 번호는 무시하고 크기와 ctime은 견주는 trss-collect의 `unchanged`만 확인해요. |
| trss-library `artwork/tests.rs`의 `a_file_is_the_apps_own_whatever_device_number_it_was_mounted_with` | 남김 | 재시작 뒤의 정리와 복구 경로이고, `found_at`을 쓰는 유일한 곳이에요. |
| trss-jobs `runner.rs`의 `a_fetched_file_is_found_by_its_object_whatever_device_number_it_was_mounted_with` | 남김 | 작업 실행기가 이어 하는 경로이고, 그곳이 trss-core를 부르지 않으면 실패해요. |
| trss-jobs `runner.rs`의 `a_failure_whose_bytes_got_another_device_number_is_finished_by_the_next_start` | 지움 | 위 테스트와 같은 부르는 곳 하나에 닿고, 재마운트가 없는 같은 시나리오는 다른 테스트가 확인해요. |
| trss-jobs `relocate.rs`의 `a_copy_whose_device_number_changed_since_it_was_applied_is_taken_off` | 남김 | 재배치는 바이트만 견주고 파일 식별을 읽지 않아요. 그 결정을 지키는 테스트예요. |
| trss-jobs `replace.rs`의 `an_approved_plan_whose_files_got_another_device_number_is_carried_out` | 지움 | 아래 테스트가 이 테스트가 닿는 곳에 모두 닿고 더 닿아요. |
| trss-jobs `replace.rs`의 `killed_then_mounted_again_with_another_device_number_carries_the_replacement_on` | 남김 | 이어 하는 경로이고, 결과를 기록하는 곳에 닿는 유일한 테스트예요. |
| trss-jobs `place.rs`의 `a_prepared_store_is_found_by_its_object_whatever_device_number_it_was_mounted_with` | 남김 | 배치의 한 부르는 곳에 닿는 유일한 테스트예요. |
| trss-worker `episode_offset.rs`의 `an_undo_cut_short_knows_its_videos_whatever_device_number_they_were_mounted_with` | 남김 | 재시작 뒤 이어 하는 경로이고, 디스크 이름 바꾸기의 `unchanged`에 닿는 유일한 테스트예요. |
| trss-worker `episode_offset.rs`의 `a_waiting_torrent_is_renamed_whatever_device_number_its_file_was_mounted_with` | 남김 | 재시작 뒤 이어 하는 경로이고, Transmission 이름 바꾸기 전의 확인에 닿는 유일한 테스트예요. |

### 달라진 것과 남은 일

- 읽기가 조금 엄격해졌어요. 예전 `same_object`는 둘째 칸을 글자로 견줘서 숫자가 아닌 글자도 같으면 같은 파일로 봤어요. 이제는 장치 번호와 inode가 숫자여야 하고, 앞자리 0이 붙은 같은 inode도 같은 숫자로 읽어요. 실제 기록은 언제나 숫자라서 달라지는 경우는 없다고 봐요.
- 표지 행에 `ino`만 있고 `dev`가 없으면, 예전에는 같은 파일이 될 수 있었지만 이제는 다른 파일로 봐요. 두 열을 쓰는 곳은 한 `UPDATE`에서 함께 쓰므로 그런 행은 생기지 않아요.
- 장치 번호를 무시하는 동작이 테스트되지 않은 곳이 처음부터 넷 있었어요. trss-jobs `place/mod.rs`와 `place/replace/mod.rs`의 한 곳씩, trss-collect `commands/episode_undo.rs`의 두 곳이에요. 규칙을 바꿔도 장치 번호 테스트가 실패하지 않아요. 이 곳들은 이제 trss-core의 규칙을 부르므로, 규칙 자체는 trss-core의 테스트가 지켜요.
- trss-jobs `area.rs`의 `an_object_is_the_file_s_device_and_inode`는 trss-core의 글자 테스트와 겹쳐요. 저장하는 글자를 지키는 테스트라서 남겼어요.
