# 0104 파일을 안전하게 쓰는 일을 trss-core로 모아요

- 상태: 완료
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

임시 파일에 쓰고, fsync하고, 같은 이름을 덮어쓰지 않게 이름을 바꾸고(`rename_noreplace`), 폴더를 fsync하는 일을 2026-10-07에 5개 크레이트의 10곳 넘게 손으로 썼어요.
`rename_noreplace`만 trss-core에 있고, 그 테스트는 trss-core에 없어요. 폴더 fsync는 trss-jobs의 `area`에 있고, 표지와 trss-archive와 trss-probe가 따로 써요.
fsync하는 범위가 곳마다 달라요.

| 곳 | fsync |
| --- | --- |
| trss-jobs의 배치, 받기, 올리기 | 파일, 두 폴더, 새로 만든 폴더마다 |
| trss-library의 표지 | 파일과 `artwork/` 폴더 하나. 폴더 fsync의 실패는 버려요. |
| trss-collect의 영상 이름 바꾸기(회차 변환 되돌리기, 수정본) | 없어요. |
| trss-browser의 내려받기 | 없어요. 다른 파일 시스템이면 `linkat`이나 복사로 옮겨요. |

ADR 0016대로 trss-core에 폴더 fsync, 덮어쓰지 않는 이름 바꾸기와 두 폴더 fsync, 새로 만들며 쓰고 fsync하는 쓰기를 두고, 위의 곳들이 그것을 써요.

- fsync가 없는 곳(trss-collect의 영상 이름 바꾸기, 서버 브라우저의 내려받기, 표지의 일부)에 fsync를 더할지 이 티켓에서 사용자 결정을 받아요. 미디어 디스크에서 영상 이름을 바꿀 때마다 비용이 생겨요. 더하지 않기로 하면 지금 범위를 그대로 지켜요.
- 서버 브라우저의 다른 파일 시스템 대비(EXDEV, `linkat`, 복사)는 내려받기 볼륨이 다른 파일 시스템이라 의도된 것이에요. 남겨요.
- 표지의 폴더 0700, 파일 0644 권한은 까닭을 찾아 결과 절에 적고, 바꾸면 따로 커밋해요.
- 파일 시스템이 `rename_noreplace`를 지원하지 않을 때의 문구는 지금 trss-collect의 작업 폴더와 서버 브라우저에만 있어요. 같은 부품을 쓰면서 다른 곳의 오류 문구가 바뀌면 결과 절에 적어요.

## 완료 기준

- 위의 쓰기가 trss-core의 부품을 쓰고, 손으로 쓴 복사본은 없어요.
- trss-core에 덮어쓰지 않는 이름 바꾸기, 두 폴더 fsync, 새로 만드는 쓰기의 테스트가 있어요.
- fsync에 대한 사용자 결정과 까닭이 결과 절에 있어요.
- 기존 테스트가 고치지 않고 통과해요. 그 뒤 "자리를 차지한 파일을 덮어쓰지 않아요"를 크레이트마다 다시 확인하는 테스트(2026-10-07에 약 6개)를 ADR 0015대로 정리해요.

## 결과

### 모은 곳

trss-core `files.rs`에 모았어요.

| 부품 | 하는 일 | 지운 복사본 |
| --- | --- | --- |
| `sync_dir`, `sync_dir_fd`, `sync_file` | 폴더나 파일을 fsync해요. | trss-jobs `area::sync_dir` |
| `create_dir_all_synced` | 없는 폴더를 만들고, 만든 폴더마다 그 폴더가 든 폴더를 fsync해요. | trss-jobs의 `make_dirs` |
| `rename_noreplace`, `rename_noreplace_at`, `noreplace_unsupported` | 덮어쓰지 않는 이름 바꾸기예요. `_at`은 서버 브라우저처럼 열어 둔 폴더끼리 옮겨요. `noreplace_unsupported`는 파일 시스템이 지원하지 않는다는 오류를 가려요. | 서버 브라우저의 `renameat2` 호출과 trss-collect 작업 폴더의 오류 가르기 |
| `sync_renamed`, `rename_noreplace_synced` | 이름을 바꾼 뒤 옮겨 간 폴더, 떠난 폴더 순으로 fsync해요. 두 폴더가 같으면 한 번만 해요. | trss-jobs 배치, 재배치, 교체의 손으로 쓴 두 폴더 fsync |
| `write_new` | 새로 만들기만 하는 쓰기(`O_EXCL`, `O_NOFOLLOW`)예요. 내용을 쓰고 fsync하며, 실패하면 만든 파일을 지워요. | trss-jobs 배치의 임시 파일 쓰기, 표지의 임시 파일 쓰기 |
| `files::testing` | 테스트에서만 컴파일돼요. 어떤 파일과 폴더를 fsync했는지 적고, 요청하면 fsync를 실패시켜요. fsync는 디스크에 테스트가 볼 흔적을 남기지 않기 때문이에요. | |

- 쓰는 곳은 trss-jobs의 배치, 정리, 재배치, 교체, 받기, 올리기, trss-library의 표지, trss-collect의 영상 이름 바꾸기(`revision::rename_video`), 서버 브라우저의 내려받기예요.
- trss-archive와 trss-probe는 trss-core에 의존하지 않아요. 둘이 함께 쓰는 trss-archive의 `sync_dir`을 하나만 두고 trss-probe가 그것을 불러요. trss-probe는 자기 `renameat2` 호출과 파일 fsync를 남겼어요. 서버 디스크에서 그 호출들이 하는 일을 직접 측정하는 도구이기 때문이에요.
- trss-jobs의 받기(`runner.rs`)와 올리기(`upload.rs`)는 await 사이에 이어 쓰는 비동기 쓰기라서, tokio의 `create_new`와 `sync_all`을 그대로 써요. trss-core의 막는(blocking) fsync를 부르면 런타임을 막기 때문이에요. 이름 바꾸기와 폴더 fsync는 trss-core를 거쳐요. 받기가 받는 영역부터 한 단계씩 다시 fsync하는 순서와 올리기가 작업 폴더와 그 위 폴더를 fsync하는 범위도 그대로예요. 완료 기준의 "손으로 쓴 복사본은 없어요"에서 이 두 비동기 쓰기만 예외예요.

### fsync를 더한 곳

사용자 결정(2026-10-08)대로 fsync가 없던 곳에 더했어요. 테스트는 `files::testing`으로 어떤 경로를 fsync했는지 확인해요. 데이터가 실제로 디스크에 닿았는지는 테스트로 볼 수 없어요.

| 곳 | 커밋 | 확인하는 테스트 | 비용 |
| --- | --- | --- | --- |
| trss-collect가 직접 하는 영상 이름 바꾸기(회차 변환 되돌리기, 수정본 대체에서 토렌트가 가리키지 않는 파일) | `fix(collect): sync the folders after renaming a video itself` | `episode_undo::renaming_tests::a_video_renamed_on_disk_has_its_folder_synced_and_a_failing_sync_does_not_undo_it`, `revisions::replacing_tests::a_new_video_renamed_on_disk_has_its_folder_synced_and_a_failing_sync_does_not_undo_it` | 이름 바꾸기마다 같은 폴더 fsync 한 번이에요. 한 폴더의 파일 N개를 되돌리면 그 폴더를 N번 fsync해요. `spawn_blocking` 안에서 해요. |
| 서버 브라우저의 내려받기 | `fix(browser): sync a download's bytes and folders when it is moved out` | trss-browser `files::tests`의 `a_moved_file_and_both_folders_are_synced`, `a_refused_file_syncs_no_folder_and_a_failing_sync_of_its_bytes_leaves_it_where_it_was`, `a_copy_across_filesystems_is_synced_before_it_takes_its_name` | 파일 fsync 한 번과 폴더 fsync 두 번이고, 대상 폴더를 새로 만들면 만든 폴더마다 한 번 더해요. 버리는 내려받기도 같은 비용을 내요. |
| 표지의 폴더와 이름 바꾸기 | `fix(library): sync the artwork folders and fail a publish whose rename is not durable` | `artwork::tests::a_published_image_has_its_bytes_and_its_folders_synced`, `a_publish_whose_folder_cannot_be_synced_fails_and_leaves_nothing` | 만든 폴더마다 그 폴더가 든 폴더를 fsync하고, 이름을 바꾸면 `artwork/`와 `.staging/`을 fsync해요. |

Transmission이 RPC로 하는 이름 바꾸기에는 fsync를 더할 수 없어요. 받은 이름 바꾸기(trss-collect `receive.rs`), 수정본 대체와 회차 변환 되돌리기의 `torrent_rename_path`가 그것이에요.

### fsync가 실패할 때

결정에 없던 부분이라 아래처럼 정했어요(2026-10-09).

- **trss-collect**: 영상 이름을 바꾼 뒤 폴더 fsync가 실패하면 로그만 남기고 이름 바꾸기를 마친 것으로 봐요. 실패로 보면 회차 변환 되돌리기와 수정본 기록이 이미 바뀐 이름을 바뀌지 않았다고 적기 때문이에요. 다시 fsync하지는 않아요.
- **서버 브라우저**: 파일 내용의 fsync가 실패하면 내려받은 파일을 그 자리에 두고 오류를 돌려줘요. 옮긴 뒤 폴더 fsync가 실패하면 로그만 남기고 옮긴 파일을 돌려줘요. 오류로 돌려주면 자막 확인과 WinPNG 흐름이 이미 놓인 파일을 "가져오지 못했어요"로 알리고, 같은 이름의 다음 내려받기가 막히기 때문이에요.
- **표지**: 버리던 `artwork/` 폴더 fsync의 실패를 이제 오류로 봐요. 이름을 바꾼 뒤 폴더를 fsync하지 못하면 방금 놓은 파일을 지우고 기록을 잊은 뒤, 파일을 쓰지 못했을 때처럼 실패해요. 이 fsync는 선택이 그 이름을 가리키기 전에 이름이 남는다는 것을 보장하려는 것이기 때문이에요. 그 파일은 아직 `staging` 기록이라 선택이나 웹이 가리키지 않고, 이름도 방금 새로 만든 것이라 다른 파일을 지울 일이 없어요.

### 다른 점마다 정한 것

- **두 폴더 fsync의 순서**: 배치는 옮겨 간 폴더를 먼저, 재배치와 교체는 떠난 폴더를 먼저 fsync했어요. 지금은 모두 옮겨 간 폴더가 먼저예요. 둘 다 실패할 때 알리는 오류만 달라질 수 있어요.
- **`O_NOFOLLOW`**: 이제 배치의 임시 파일도 `O_NOFOLLOW`로 만들어요. `O_EXCL`과 함께면 더 바뀌는 것이 없어요.
- **오류 문구**: 바뀐 문구가 없어요. 파일 시스템이 덮어쓰지 않는 이름 바꾸기를 지원하지 않는다는 문구는 trss-collect의 작업 폴더에만 있고, 서버 브라우저는 문구 없이 링크와 복사로 넘어가요. 두 곳은 오류를 가리는 부분만 함께 써요.
- **권한과 임시 파일 정리**: 그대로예요. 표지 파일은 0600으로 만들어 fsync 전에 0644로 바꾸고, 표지 폴더는 0700이에요. 배치와 표지 모두 `write_new`가 만든 뒤 실패한 임시 파일을 지워요. 디스크에 남는 것은 전과 같아요.

### 표지의 권한

폴더 0700과 파일 0644의 까닭은 기록에 없어요. 두 값은 `ff9bfe1`에 주석, ADR, 명세 문장 없이 들어왔어요. 명세의 "비공개 임시 위치"가 폴더의 0700과 처음 만들 때의 0600을 설명할 수 있고, 마지막에 0644로 바꾸는 것은 umask와 상관없이 같은 권한을 주려는 것으로 보여요. 결정대로 바꾸지 않았어요.

### 테스트 정리

"자리를 차지한 파일을 덮어쓰지 않아요"는 이제 trss-core의 `a_rename_never_replaces_whatever_is_at_the_name`, `a_synced_rename_onto_a_taken_name_moves_and_syncs_nothing`, `a_new_file_never_takes_the_place_of_what_is_there`가 확인해요.

- 지운 것: trss-collect `commands::rule_archive::work_folder::tests::a_rename_never_replaces_a_file_or_a_directory`예요. trss-core의 `a_rename_never_replaces_whatever_is_at_the_name`이 같은 경우예요.
- 줄인 것: trss-browser `tests/it/pool.rs`의 `a_download_is_moved_into_the_given_folder_by_its_suggested_name`에서 두 번째 내려받기 부분을 뺐어요. trss-browser `files::tests::a_file_moves_and_nothing_is_replaced`가 같은 경우예요.
- 남긴 것: 자리를 차지한 이름을 기능마다 다르게 다루는 테스트 5개예요. trss-jobs `place/files.rs`의 `a_copy_is_checked_and_published_without_replacing`, trss-library `artwork::tests::a_taken_place_is_never_overwritten`, trss-browser `files::tests::a_file_moves_and_nothing_is_replaced`, trss-collect `renaming_tests::a_name_that_is_taken_is_never_renamed_onto_and_the_others_go_on`, `replacing_tests::row_7_a_taken_episode_name_is_never_renamed_over`예요.
- 2026-10-07에 약 6개로 셌던 테스트 대부분은 [0101](0101-receive-line-tests.md)에서 이미 옮기거나 지웠어요.
- [RSS 수집 명세의 검증 표](../specs/collection.md#검증-표)가 인용한 테스트는 옮기거나 지우지 않았어요.

### 검증한 것

2026-10-09에 커밋마다 `cargo test --locked --workspace -j 4`로 확인했어요. 알려진 흔들리는 테스트도 실패하지 않았어요.

| 커밋 | 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작 | 2,824 | 0 | 14 |
| `refactor(core): sync folders and publish files without replacing in trss-core` | 2,840 | 0 | 14 |
| `refactor(library): write the staged artwork file with trss-core's create-new write` | 2,840 | 0 | 14 |
| `refactor(probe): sync folders with trss-archive's sync_dir` | 2,841 | 0 | 14 |
| `fix(collect): sync the folders after renaming a video itself` | 2,843 | 0 | 14 |
| `refactor(collect): tell a filesystem without the no-replace rename in one place` | 2,843 | 0 | 14 |
| `fix(browser): sync a download's bytes and folders when it is moved out` | 2,847 | 0 | 14 |
| `fix(library): sync the artwork folders and fail a publish whose rename is not durable` | 2,849 | 0 | 14 |
| `refactor(jobs): sync a recovered receive file with trss-core's sync_file` | 2,849 | 0 | 14 |
| `test: test the rename that replaces nothing once, in trss-core` | 2,848 | 0 | 14 |
| `perf(core): sync a folder once when a rename stays inside it`(이 결과를 담은 커밋) | 2,849 | 0 | 14 |
| 리뷰 뒤 고친 것을 fix 커밋에 합친 뒤 | 2,849 | 0 | 14 |

- `cargo fmt --all --check`는 깨끗하고, clippy에 새 경고가 없어요.
- trss-collect의 fsync 테스트 하나는 fsync를 빼면 실패하는 것을 확인했어요.
- 리뷰 뒤 고친 것을 합친 첫 실행에서 trss-worker `archive_move::a_subscriptions_start_receives_the_ticked_items_into_the_work_folder_that_came_over`가 한 번 실패했고, 다시 돌리면 통과했어요. 0104와 관계없이 한 규칙의 받기 둘이 함께 도는 결함이라 [0130](0130-receive-a-rule-in-turn.md)으로 따로 고쳤어요.
- 2026-10-09에 stora:reviewer가 열 커밋을 검토했어요. refactor 커밋에서 fsync하는 범위가 줄어든 곳, 오류 문구, 임시 파일 정리, `O_EXCL`과 `O_NOFOLLOW`, 권한, 다른 파일 시스템 대비, 오류 가르기를 봤고, 동작이 바뀐 곳은 찾지 못했어요. 지적 가운데 아래를 고쳐 해당 fix 커밋에 합쳤어요.
  - 서버 브라우저가 옮긴 뒤의 폴더 fsync 실패를 오류로 돌려주던 것을 위의 결정대로 고쳤어요.
  - 서버 브라우저가 대상 폴더를 새로 만들 때 그 폴더가 든 폴더를 fsync하지 않던 것을 `create_dir_all_synced`로 고쳤어요.
  - 표지의 폴더 fsync 실패 테스트는 이름을 바꾸기 전에 실패해서 새 분기에 닿지 않았어요. 두 폴더를 미리 만들게 고쳤고, 그 분기를 빼면 테스트가 실패하는 것을 확인했어요.
  - 표지의 이름 바꾸기와 두 폴더 fsync를 `spawn_blocking` 안에서 하게 했어요.
  - fsync 횟수를 정확히 세던 단언 두 개를 "한 번 이상"으로 바꿨어요. 기록기는 파일을 장치와 inode로 세므로, inode를 다시 쓰는 파일 시스템에서는 앞 테스트의 fsync가 섞일 수 있기 때문이에요.

### 남은 것

- trss-collect가 작품 폴더를 수집 폴더와 보관 폴더 사이에서 옮기는 이름 바꾸기(`rule_archive/work_folder.rs`)에는 fsync가 없어요. 영상이 아니라 폴더를 옮기는 일이라 결정의 범위 밖으로 보고 남겼어요.
- trss-subtitles `auth.rs`의 `write_whole`은 작은 파일을 쓰고 fsync 없이 덮어쓰는 이름 바꾸기를 해요. 잠깐 쓰는 데이터라 건드리지 않았어요.
- trss-jobs 올리기의 임시 파일은 새로 만들기만 하는 쓰기가 아니라 `File::create`로 만들어요. 바꾸지 않았어요.
- 표지의 정리는 fsync가 실패할 때 끝까지 보장하지 않아요. 놓은 파일을 지우지 못하거나 그 사이 멈추면 기록 없는 `artwork/<id>.<ext>`가 남고, 정리(`cleanup`)는 그런 파일을 찾지 않아요. `ensure_dir`도 폴더를 만든 뒤 그 폴더가 든 폴더의 fsync가 실패하면, 다음 호출은 폴더가 있다고 보고 다시 fsync하지 않아요.
- 서버 브라우저는 버리는 내려받기도 내용과 폴더를 fsync하고, 그 내용의 fsync가 실패하면 WinPNG 읽기가 실패해요. 내용을 fsync하려고 파일을 읽기로 열므로, 같은 파일 시스템 안에서 옮길 때도 읽기 권한이 필요해졌어요. 받은 파일은 뒤에서 어차피 읽으므로 새로 막히는 경우는 찾지 못했어요.
- musl 빌드와 docker, `dev/measure`는 돌리지 않았어요.
