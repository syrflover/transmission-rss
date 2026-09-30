# 0011 규칙을 보관·복원할 때 작품 폴더를 옮겨요

- 상태: 완료
- 출처: [보관과 복원의 폴더 이동](../specs/collection.md#보관과-복원의-폴더-이동), [규칙 보관](../specs/collection.md#규칙-보관)
- 막는 티켓: [0010](0010-app-collect-folder.md)

## 작업

규칙을 보관하면 그 작품 폴더를 수집 폴더에서 보관 폴더로 통째로 옮기고, 복원하면 되돌려요.
분기가 끝난 작품이 `Shows (current)`에 쌓이지 않고 `Shows`로 가게 하려는 거예요.
보관 제안(방영 종료·새 항목 없음 판단, `모두 보관`)은 이 티켓에 넣지 않고, 사용자가 규칙 상세에서 보관·복원할 때의 이동만 다뤄요.

- 옮기기는 웹이 접수하고 worker가 실행하는 명령이에요([웹 명령의 계약](../specs/web-app.md#웹-명령과-상태-갱신), `다시 받기`와 같은 방식). 미디어에 쓰는 것은 worker뿐이라, `docker-compose.trss.yml`에서 worker만 미디어를 읽고 쓰게 하고 웹은 읽기 전용으로 둬요.
- 보관은 규칙을 먼저 끄고 옮기며, 같은 작품 폴더에 받는 수집 중인 규칙이 남아 있으면 옮기지 않고 까닭을 남겨요. 복원은 폴더를 먼저 되돌리고 규칙을 켜요.
- 작품 폴더의 위치는 저장하지 않고 옮길 때 디스크를 보고 정해요. 그래서 도중에 멈춘 이동을 다음 실행에서 이어 가고, 손으로 옮겨 둔 폴더도 알아봐요.
- Transmission이 가진 토렌트 중 그 작품 폴더 안에 받은 것은 `torrent-set-location`(파일 이동)으로 먼저 옮기고, 나머지 파일은 worker가 같은 파일시스템 안의 이름 바꾸기로 옮겨요.
- 목적지에 같은 작품 폴더가 있으면 합치고, 같은 상대 경로의 파일이 하나라도 겹치면 아무것도 옮기지 않아요. 새로 만드는 폴더는 목적지 상위 폴더의 소유자·그룹을 따라요. 겹침·파일시스템·소유자 처리는 사용자 결정이에요.
- 대상은 두 폴더 바로 아래의 작품 폴더로 한정하고, 링크로 두 폴더 밖을 가리키는 경로는 거부해요.
- 규칙 상세에 이동 상태(`보관 폴더로 옮기는 중이에요`·`보관 폴더로 옮겼어요`·`옮기지 못했어요`와 까닭)와 실패 뒤 `다시 옮기기`를 둬요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| `Clevatess/Season 02`만 쓰는 규칙을 보관, 보관 폴더에 `Clevatess` 없음 | 규칙이 꺼지고 `Shows (current)/Clevatess`가 `.trss/`와 함께 `Shows/Clevatess`로 옮겨져요. 그 안에 받은 Transmission 토렌트는 새 위치를 가리키고 시딩을 이어 가요. |
| `Season 02` 규칙을 보관했는데 `Season 03` 규칙이 수집 중 | 규칙만 꺼지고 폴더는 그대로이며, 수집 중인 규칙 때문에 옮기지 않았다고 규칙 상세에 보여요. |
| 보관 폴더에 이미 `Clevatess/Season 01`이 있음 | `Season 02`가 그 옆으로 합쳐지고, 새로 만든 폴더의 소유자가 상위 폴더와 같아요. |
| 양쪽 `Season 02`에 같은 이름의 파일이 있음 | 아무것도 옮기지 않고 겹치는 파일을 까닭으로 보여주며, `다시 옮기기`가 있어요. |
| 보관한 규칙을 복원 | 작품 폴더가 수집 폴더로 돌아온 뒤 규칙이 켜지고, 다음 회차가 같은 폴더에 들어와요. Transmission 토렌트도 돌아온 위치를 가리켜요. |
| 보관 폴더가 비어 있음 | 규칙만 꺼지고 아무것도 옮기지 않아요. |
| 옮기는 중 worker를 멈췄다가 다시 시작 | 다음 실행이 디스크를 보고 남은 것만 옮겨 끝내요. 파일이 두 곳에 나뉜 채 남지 않아요. |
| 작품 폴더가 수집 폴더 밖을 가리키는 링크 | 옮기지 않고 까닭을 남겨요. |
| 웹 컨테이너 | 미디어를 여전히 읽기 전용으로 마운트해요. |

실제 서버의 Transmission에서 `torrent-set-location` 뒤 시딩이 이어지는지는 배포 뒤 한 작품으로 확인해요.

## 결과

### 구현한 것

- 명령 `rule_archive`(`src/worker/commands/rule_archive.rs`): payload는 `{ rule_id, direction: "archive" | "restore" }`이고 명령의 항목(subject)은 규칙 ID예요. 한 규칙에 열린 명령이 있으면 새 명령은 `409`(`current`에 그 명령)예요. 웹(`src/web/commands_api.rs`)은 규칙이 없으면 `404`, 이미 수집 중인 규칙의 복원은 `400`으로 거절하고, 규칙의 상태는 바꾸지 않아요. 상태를 바꾸는 것은 worker뿐이에요.
  - 보관: 규칙을 `archived`로 바꾼 뒤(`ChannelStore::set_rule_state`, 이미 그 상태면 그대로) 옮길지 정해요. 수집 폴더·보관 폴더가 없거나, 저장 폴더가 비었거나(수집 폴더 자체), 수집 폴더 밖이거나(`..` 포함), 모든 채널에서 같은 작품 폴더에 받는 수집 중인 규칙이 있으면 옮기지 않고 `done`·`kept`와 까닭으로 끝나요(`‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.`, 둘 이상이면 `외 N개가`).
  - 복원: 폴더를 수집 폴더로 먼저 옮기고, 옮기지 못하면 규칙을 보관된 채로 두고 `failed`와 까닭으로 끝나요. 옮겼거나 이미 수집 폴더에 있거나 어느 쪽에도 없으면 규칙을 `active`로 바꿔요.
  - 결과는 `moved`(옮김, 또는 이미 목적지에만 있음), `kept`(일부러 두었음, 까닭 포함), `failed`(옮기지 못함, 까닭 포함)예요.
- 옮기기(`src/worker/commands/rule_archive/work_folder.rs`)는 매번 디스크와 Transmission을 보고 남은 것을 해요. 위치는 저장하지 않아요.
  1. 검사(읽기만): 두 폴더가 있고 서로 안에 있지 않으며 같은 `st_dev`, 작품 폴더 이름이 경로 조각 하나이고 링크가 아님, 안쪽에 두 폴더 밖을 가리키는 링크나 다른 파일시스템이 없음, 양쪽에 같은 상대 경로의 파일(양쪽 모두 실제 폴더인 것은 합침)이 없음. 겹치면 파일을 5개까지 적고(`외 N개`) 아무것도 옮기지 않아요.
  2. Transmission: 받는 폴더가 작품 폴더 안인 토렌트마다 `torrent-set-location`(`move: true`)을 보내고, 모두 새 위치를 보고할 때까지 1초마다 확인해요(최대 300초, 넘으면 `failed`). 누가 넣은 토렌트든 옮겨요.
  3. 나머지: `renameat2(RENAME_NOREPLACE)`(`rustix`, 지원하지 않는 커널·파일시스템에서는 확인 뒤 이름 바꾸기)로 목적지에 없는 것은 통째로, 양쪽에 있는 폴더는 그 안을 파일 단위로 옮기고, 비게 된 원래 폴더는 지워요. 검사 뒤 목적지에 생긴 파일은 덮지 않고 원래 자리에 둔 채 `failed`로 알려요.
  - worker는 폴더를 새로 만들지 않아요. 목적지에 없는 것은 이름 바꾸기로 원래 소유자째 옮겨지고, 새 폴더는 Transmission이 자기 토렌트를 옮기며 만든 것뿐이라 Transmission 사용자의 것이에요.
- 수집 주기와 같은 flock 아래에서 명령을 실행하므로, 옮기는 동안 주기는 `Busy`로 건너뛰고 새 회차를 넣지 않아요.
- 멈춘 뒤 이어 가기: 명령은 `running`으로 남아 다음 worker가 다시 집어요(기존 명령 계약, 최대 5번). 각 단계는 원래 자리에서 목적지로만 옮기고 이미 목적지를 보고하는 토렌트는 다시 옮기지 않으므로, 다시 실행하면 남은 것만 옮겨 모두 목적지에 모여요.
- 웹 API(`src/web/rules_api.rs`): `GET /api/rules/{id}`를 더했고, 규칙 보기에 마지막 보관·복원 명령(`archive_move{direction, command}`)이 실려요. `PUT /api/rules/{id}`는 상태를 바꾸는 본문을 `400`으로 거절해요(보관·복원은 명령으로만).
- 화면(`web/src/screens/collect/rules/`): `보관`·`복원` 버튼이 명령을 보내고(`useArchiveMove`, `다시 받기`와 같은 ID·조회·다시 보내기 방식), `ArchiveMoveNotice`가 `보관 폴더로 옮기는 중이에요`·`보관 폴더로 옮겼어요`·`옮기지 못했어요`와 까닭, 복원 쪽 문장, 일부러 두었을 때의 까닭을 보여줘요. 보관에서 옮기지 못했고 규칙이 보관된 채면 `다시 옮기기`가 있어요. 끝나면 규칙을 다시 읽어 목록 캐시를 바꿔요. 옮기는 동안 `보관`·`복원`·`삭제`는 눌리지 않아요. 명령 타입과 보내기는 `web/src/lib/commands.ts`로 옮겨 기록 탭과 같이 써요. 설정 `수집 폴더` 항목의 안내 문장도 옮기기에 맞게 고쳤어요.
- 배포: `docker-compose.trss.yml`에서 `trss-worker`만 미디어를 읽고 쓰게 마운트하고 `trss-web`은 `:ro` 그대로예요. `readme.md`에 마운트와 보관·복원 동작을 적었어요.

### 검증한 것

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(라이브러리 325개, `tests/archive_move.rs` 13개 포함 모든 통합 시험)가 통과했고, 웹은 `npm run typecheck`와 `npm run build`가 통과했어요.
완료 기준은 `tests/archive_move.rs`(시험용 Transmission이 `torrent-set-location`에서 실제 임시 폴더의 파일을 옮겨요)로 확인했어요.

| 완료 기준 | 근거 |
| --- | --- |
| `Clevatess/Season 02`만 쓰는 규칙을 보관 | `archiving_turns_the_rule_off_and_moves_the_work_folder_with_its_torrents`: 접수 뒤 실행 전에는 규칙·폴더 그대로, 실행 뒤 규칙 `archived`, `.trss/`·토렌트 아닌 파일까지 `Shows/Clevatess`로, 봇 토렌트 포함 두 토렌트가 새 위치·`SEEDING`, 다른 작품 토렌트는 그대로, `torrent-add` 없음 |
| `Season 03` 규칙이 수집 중 | `a_folder_another_active_rule_saves_in_stays_until_the_last_rule_is_archived`: `kept`와 남은 규칙 이름, 마지막 규칙을 보관할 때 옮김, 두 규칙을 한꺼번에 보관, 이미 수집 폴더에 있는 복원 |
| 보관 폴더에 `Clevatess/Season 01` | `a_season_merges_beside_the_archived_one_and_new_folders_take_the_parents_owner`: 합쳐지고 비게 된 원래 폴더가 지워지며, worker가 옮긴 폴더는 inode가 같아요(복사가 아님). 새 `Season 02`의 소유자 비교는 아래 "검증하지 못한 것" |
| 양쪽에 같은 이름의 파일 | `the_same_file_on_both_sides_moves_nothing_and_moving_again_works_once_cleared`: 아무것도 옮기지 않고 Transmission에 요청도 없으며 까닭에 파일 경로, 정리 뒤 다시 보관하면 옮김. 목록이 길 때와 검사 뒤 생긴 파일은 단위 시험 |
| 복원 | `restoring_moves_the_folder_back_before_the_rule_is_on_and_the_next_episode_lands_there`: 폴더·토렌트가 돌아온 뒤 규칙이 켜지고, 다음 주기의 새 회차가 같은 폴더로 들어가요 |
| 보관 폴더가 비어 있음 | `without_an_archive_folder_the_rule_is_only_archived`(저장 폴더가 빈 규칙 포함) |
| worker를 멈췄다가 다시 시작 | `a_worker_stopped_after_transmission_moved_finishes_the_rest_on_the_next_run`(Transmission 단계 뒤 중단, 그 사이 주기는 아무것도 넣지 않음), `a_worker_stopped_during_the_renames_finishes_the_rest_on_the_next_run`(두 곳에 나뉜 상태에서 다시 집음). 둘 다 끝에 원래 자리가 비어요 |
| 수집 폴더 밖을 가리키는 링크 | `a_work_folder_that_is_a_link_out_of_the_collect_folder_is_not_moved`, 링크 단위 시험(폴더 자체·안쪽의 밖 링크·끊긴 링크·안쪽끼리의 링크 허용·목적지 링크) |
| 웹 컨테이너 | `docker compose -f docker-compose.trss.yml config`에서 `trss-web`의 `/downloads`는 `read_only: true`, `trss-worker`는 쓰기 가능 |

- 주기와의 겹침: `no_cycle_runs_while_a_folder_moves`(옮기는 동안 주기와 다른 명령 실행이 `Busy`).
- Transmission: `the_renames_wait_until_transmission_reports_the_new_folder`, `transmission_that_never_reports_the_new_folder_fails_the_move_and_moving_again_finishes`, `a_transmission_that_refuses_the_move_leaves_everything_in_place`.
- 다른 파일시스템은 `st_dev`를 바꿔 주는 단위 시험(`folders_on_different_filesystems_are_refused`)으로 확인했어요.
- 웹 API: `an_archive_is_accepted_for_its_rule_once_and_the_rule_is_not_changed_by_the_web`, `a_restore_needs_an_archived_rule_and_a_missing_rule_is_not_found`, `the_state_is_not_saved_by_an_edit_and_the_last_archive_move_is_shown`.
- 브라우저(로컬 `trss-web`·`trss-worker`, 스크래치 DB·임시 미디어 폴더, 위치를 4초 늦게 보고하는 가짜 Transmission): 1280px에서 보관(`옮기는 중` 뒤 `옮겼어요`, 목록 배지 `보관됨`), 겹침으로 `옮기지 못했어요`와 파일 경로·`다시 옮기기`, 정리 뒤 `다시 옮기기`로 옮김, 복원(`수집 폴더로 옮기는 중이에요` 동안 `복원하는 중`·`삭제`가 눌리지 않음, 뒤에 `수집 폴더로 옮겼어요`와 배지 `수집 중`)을 보고 디스크에서 파일 위치를 확인했어요. 390px에서 같은 실패·`다시 옮기기`·옮김을 봤고 가로 넘침이 없었어요(`scrollWidth == innerWidth`).

### 검증하지 못한 것

- 실제 서버의 Transmission에서 `torrent-set-location` 뒤 시딩이 이어지는지는 배포 뒤 한 작품으로 확인해요. 시험은 가짜 Transmission이에요.
- 소유자: 시험은 모두 한 사용자로 돌아서 소유자를 구별하지 못해요(root로 돌리는 시험 없음). worker는 폴더를 만들지 않으므로 root 소유 폴더가 생기지 않지만, 합칠 때 Transmission이 만드는 새 폴더는 목적지 상위 폴더가 아니라 Transmission 사용자(`PUID`)의 것이에요. 미디어 폴더의 소유자가 Transmission의 `PUID`와 같은 지금 배포에서는 둘이 같아요.
- 실제로 다른 파일시스템에 있는 두 폴더, `RENAME_NOREPLACE`를 지원하지 않는 파일시스템(대체 경로), 컨테이너 안의 쓰기 권한은 확인하지 않았어요.
- 웹 화면의 자동 시험은 없어요. 라이트 모드·키보드 조작은 보지 않았어요.

### 남은 일

- 배포 뒤 한 작품을 보관·복원해 토렌트가 새 위치에서 시딩을 이어 가는지 확인해요.
- 이 기능 전에 보관한 규칙은 명령 기록이 없어 `다시 옮기기`가 없고, 폴더는 수집 폴더에 남아 있어요. 옮기려면 복원 뒤 다시 보관해요.
- 보관 제안(`모두 보관`·`N개 보관`)의 자리에 이동 상태를 보이는 일은 보관 제안을 구현할 때 해요.
