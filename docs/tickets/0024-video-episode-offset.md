# 0024 새 시즌의 영상 회차 변환을 근거가 맞을 때 자동으로 정해요

- 상태: 완료 (실제 Transmission·AniList 확인과 설정 내보내기의 `episode_auto` 왕복은 못 봤어요. 아래 "검증하지 못한 것")
- 출처: [영상 회차 변환](../specs/collection.md#영상-회차-변환), [자막의 회차 대응](../specs/library.md#자막의-회차-대응)(구분), [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드)(`episode`·`episode_auto`)
- 막는 티켓: 없음(시즌 정보는 [0017](0017-season-info.md), 구독과 시즌의 연결은 [0019](0019-subscription-rule-detail.md)에 있어요)

## 작업

규칙의 회차 변환(`episode`)은 릴리스 회차를 영상 이름의 시즌 회차로 바꾸는 값이에요. 새 시즌의 첫 릴리스 번호가 이전 시즌까지의 AniList 회차 합계 + 1과 같으면 묻지 않고 자동으로 적용하고, 근거가 맞지 않으면 제안으로 보여 사용자가 확인해요.

- 근거: 구독이 이어진 시즌(0019)과 그 작품의 이전 시즌들의 AniList 회차 수(0017), 그 규칙이 처음 본 릴리스의 회차. `- 25`로 시작하고 이전 시즌 합계가 24면 −24, `- 01`이면 0이에요.
- 처음 본 릴리스가 시즌 첫 화가 아니거나, 분할 방영이 같은 시즌 폴더에서 번호를 이어가거나, 이전 시즌의 회차 수를 모르면 자동으로 정하지 않고 규칙 상세에 제안(값과 근거, `적용`)으로 보여요.
- 자동으로 정한 값은 규칙 상세에 `자동` 표시와 근거 문장을 두고, 직접 입력으로 바꾸면 `자동`이 사라지고 다시 자동으로 덮지 않아요. 자동 여부는 저장해요(설정 교환의 `episode_auto`). 마이그레이션이 필요하면 번호는 그때 master의 다음 번호예요.
- 자동 적용은 그 규칙이 받는 첫 항목보다 먼저 정해져야 의미가 있으므로, 주기가 그 규칙의 항목을 받기 전에 판단해요. 이미 받은 항목의 이름은 바꾸지 않아요.
- 자막의 회차 대응과 섞지 않아요(라이브러리 명세의 구분을 따라요).

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 1·2기 합계 24화, 3기 구독의 첫 릴리스 `- 25` | 회차 변환이 −24로 자동 적용되고 규칙 상세에 `자동`과 근거가 보여요. 첫 항목이 `S03E01` 이름으로 받아져요. |
| 새 작품의 첫 릴리스 `- 01` | 회차 변환 0, 묻지 않아요. |
| 처음 본 릴리스가 `- 27`(시즌 첫 화가 아님) | 자동 적용하지 않고 제안으로 보여요. 사용자가 `적용`하거나 값을 적기 전에는 변환 없이 받아요. |
| 이전 시즌의 AniList 회차 수를 모름 | 자동 적용하지 않고 제안·직접 입력으로 남아요. |
| 자동 값을 사용자가 −12로 고침 | `자동`이 사라지고 이후 자동 판단이 그 값을 덮지 않아요. |
| 설정 내보내기·가져오기 | `episode`와 `episode_auto`가 그대로 오가요. |

## 결과

### 구현한 것

- 마이그레이션(`src/store/channels/episode_basis.sql`): `rules.episode_basis`(자동으로 정한 값의 근거 문장)를 더했어요. 23번 마이그레이션이에요(0023의 것은 그 뒤 24번). 옛 DB가 규칙을 그대로 두고 올라오는 것은 `src/store/db.rs`의 시험이 봐요. 근거는 값이 그대로일 때만 남아요. 규칙 저장과 가져오기에서 값이 바뀌거나 `episode_auto`가 꺼지면 같이 지워져요(`src/store/channels/repo.rs`, `import.rs`).
- 저장소(`src/store/channels/episode.rs`): `set_auto_episode`는 앱이 정한 값이 아니고, 앱이 그 규칙을 한 번도 정한 적이 없으며(`rules.episode_decided`), 읽은 버전 그대로일 때만 값·`episode_auto`·근거와 그 전 값(`rules.episode_previous`)을 한 번에 써요. 칸의 값은 따지지 않아요(2026-10-02 결정, 아래). `set_episode`는 `적용`처럼 사용자 값을 저장하고 `자동`을 꺼요. 저장·`적용`으로 값이 사용자의 것이 되면 근거와 그 전 값은 지워지고, 정한 적이 있다는 표시는 남아요.
- 판단(`src/episode_offset.rs`): 처음 본 릴리스 `f`, 이전 시즌 합계 `P`, 시즌 폴더가 이미 가진 영상으로 표로 정해요(표는 모듈 문서에 있어요). `f = P + 1`이고 폴더에 영상이 없으면 `−P`(`f = 1`이면 0)를 자동으로 정해요. 칸이 이미 그 값이면(0과 1은 같은 값으로 봐요, `same_effect`) 바꾸지 않아요. `f > P + 1`이면 `−P` 제안, 이전 시즌 합계를 모르거나 폴더에 영상이 있어 어느 회차인지 말할 수 없으면 값 없는 안내만 해요. `f ≤ P`이고 `f − 1`이 바로 앞 시즌부터 거꾸로 센 시즌들(2기 이후에서 시작)의 회차 합계와 같으면 `−(f − 1)` 제안이에요(2026-10-02 결정, 아래). 그 밖의 `f ≤ P`(번호가 시즌 안에서 다시 시작하는 경우)와 `P = 0`에 `f > 1`인 경우는 아무것도 하지 않아요. 이 모두보다 먼저, `f = 1`이고 그 시즌의 AniList 항목(쿨) 중 앞 쿨들의 영상이 폴더에 모두 있으면 `C + 1` 제안이에요(2026-10-02 결정, 아래). 회차 변환 값 뒤의 조사(`로`·`으로`)는 숫자의 끝 읽기를 따라요(`−30으로`).
- 주기(`src/worker/cycle.rs`, `src/worker/offsets.rs`): 항목을 Transmission에 보내기 전에, 아직 아무 항목도 고르지 않았고 앱이 정한 적이 없는 구독 규칙마다 그 주기의 가장 낮은 정수 회차로 판단하고 값을 정하면 그 주기의 작업에 바로 써요. 이미 받은 항목의 이름은 바꾸지 않아요. 사용자가 지난 항목 하나를 먼저 골라 받는 `receive_once`도 같은 판단을 거쳐요(`src/worker/commands/receive_once.rs`).
- 웹 API: 규칙 응답에 `episode_basis`(그 전 값을 아는 자동 값이면 `… 정했어요 (전에는 −24).`처럼 끝에 붙여요), `episode_previous`와 `episode_suggestion { value, basis }`, `PUT /api/rules/{id}/episode`(`{ version, episode }`, 버전이 어긋나면 `409`와 `current`). 제안은 열 때마다 수집 이력·라이브러리·AniList 합계에서 읽어서, 사용자가 시즌을 이은 뒤에 나타나고 오프셋을 정하면 사라져요.
- 웹 화면(`EpisodeGrounds.tsx`, `RuleDetail.tsx`): `자동` 표시 아래에 근거 문장, 제안(근거 문장, 값, `적용`)을 보여요. 값을 적을 수 없는 안내에는 `적용`이 없어요. 필드를 고치면 둘 다 숨고, `적용`은 스위치처럼 바로 저장해요.
- `되돌리기`(2026-10-02 결정): 웹 명령 `episode_undo`(`{ rule_id, episode }`, `src/web/commands_api.rs`)를 worker가 실행해요(`src/worker/commands/episode_undo.rs`). 규칙이 아직 그 자동 값이고 그 전 값을 알 때만 받아요. worker는 먼저 이름을 바꿀 영상을 계획하고, 한 트랜잭션에서 값을 되살리고(`자동`·근거·그 전 값을 지우고 버전을 올려요) 계획을 저장해요(`src/store/channels/episode_undo.rs`, 23번 마이그레이션의 `episode_undos`·`episode_undo_files`). 그다음 파일마다 이름을 바꾸고 결과를 기록해요. 중간에 멈추면 다음 실행이 남은 파일부터 이어가요. 계획은 그 규칙의 `받음` 항목 중 토렌트 해시가 있는 것에서, 앱 값과 되살린 값으로 각각 trname 이름을 만들어 두 이름이 다른 것만 담아요. Transmission에 토렌트가 있으면 단일 파일 토렌트의 파일 이름이 앱 값의 이름일 때만, 없으면 그 경로에 일반 파일이 있고 Transmission의 어느 토렌트에도 속하지 않을 때만 담고 파일의 신원(`FileIdentity`: 장치·inode·크기·시각)을 함께 저장해요. 규칙 응답의 `episode_undo { command, to, files }`로 진행과 파일마다의 결과를 보여요. 화면은 `되돌리기`를 누르면 명령을 보내고 끝날 때까지 명령과 규칙을 다시 읽어 `영상 N개 중 M개를 살펴봤어요`, 끝나면 `회차 변환을 되돌렸어요`와 바꾸지 못한 파일마다 `이름을 되돌리지 못했어요: 옛 이름 → 새 이름. 까닭`을 보여요(`useEpisodeUndo.ts`).

### 결정

- "그 규칙이 처음 본 릴리스"는 그 규칙이 처음 고른 항목들 중 가장 낮은 정수 회차예요. 수집 이력에서는 `result_at`이 가장 이른 항목이에요. 주기는 항목을 보내기 전에 같은 주기의 항목 중에서 정해요.
- 시즌은 이어진 시즌이 있으면 그것이고, 없으면 규칙의 저장 폴더(`<작품>/Season NN`)가 가리키는 시즌이에요. 티켓의 "구독이 이어진 시즌(0019)"은 규칙이 영상을 받은 뒤에야 생기므로, 그대로면 새 구독의 첫 항목을 `S03E01`로 받는다는 완료 기준의 첫 행이 이뤄질 수 없어서 저장 폴더를 근거로 더했어요. 작품은 수집 폴더 밑에 있는 라이브러리의 작품(수집 폴더는 자동 감시 폴더예요)이고, 이전 시즌 폴더가 라이브러리에 없거나 AniList가 이어지지 않았거나 회차 수가 비면 합계를 모르는 것으로 보고 짐작하지 않아요.
- 아직 이어지지 않은 구독에서 이어짐을 기다리는 보수적인 방식 대신 이 방식을 골랐고, 되돌리려면 `src/episode_offset.rs`의 `gather`에서 저장 폴더 분기를 빼면 돼요.
- (2026-10-02 사용자 결정) 아직 고른 항목이 없는 규칙은 칸의 값과 상관없이 판단해요. 처음에는 오프셋이 `0`·`1`인 규칙만 판단했는데, 이전 시즌 규칙을 복사해 −24가 남은 3기 규칙이 `- 49`로 시작하면 `S03E25`로 받았어요. 이제는 그 −24도 −48로 바꾸고 첫 항목이 `S03E01`이에요(`a_value_carried_over_from_the_previous_season_gives_way_to_the_whole_sum`). 사용자가 첫 항목 전에 적어 둔 값도 같아서, 예전의 "사용자가 적은 값은 덮지 않는다"는 시험은 `an_offset_typed_before_the_first_item_gives_way_to_the_sum`으로 바꿨어요. 대신 그 전 값을 근거 문장에 보여요. 칸이 이미 그 값이면 바꾸지 않아요(`a_field_that_holds_the_sum_already_is_left_as_it_is`). 0과 1은 둘 다 번호를 그대로 두므로 같은 값으로 봐서, 새 작품의 `- 01`은 이제 `1`인 칸을 그대로 두고 `자동` 표시도 없어요(`a_new_works_first_release_is_not_converted_and_nobody_is_asked`의 기대가 바뀌었어요).
- 앱은 규칙 하나를 한 번만 정해요(`rules.episode_decided`). 칸의 값으로 "사용자가 정함"을 가를 수 없게 됐으므로, 앱이 정한 값을 사용자가 고친 뒤에는 아직 받은 항목이 없어도 다시 정하지 않게 하는 표시가 따로 필요했어요(`the_app_decides_once_and_never_at_a_version_it_did_not_read`). 이 마이그레이션 전에 자동으로 정한 규칙은 정한 것으로 올라오고, 그 전 값은 몰라요.
- (2026-10-02 사용자 결정) `되돌리기`는 그 전 값을 되살리고 `자동`을 끄며, 앱은 그 규칙을 다시 정하지 않아요(받은 항목이 없을 때 되돌려도 같아요, `an_undo_before_any_item_keeps_the_app_from_deciding_again`). 앱 값으로 받은 영상은 되살린 값의 이름으로 바꿔요(`S03E01` → `S03E25`, `undoing_puts_the_previous_value_back_and_renames_what_it_named`). 파일을 덮어쓰거나 다른 파일의 이름을 바꾸지 않도록 이렇게 정했어요.
  - 토렌트가 있으면 Transmission의 이름 변경을 써요(시드가 이어져요). Transmission은 목적지에 파일이 있어도 파일을 옮기지 않은 채 성공으로 답하므로, 보내기 전에 목적지를 확인하고, 답을 받은 뒤 옛 이름과 새 이름이 둘 다 있으면 토렌트의 이름을 되돌리고 바꾸지 못한 것으로 기록해요. 토렌트 이름이 계획 때와 다르면(사용자가 바꿨으면) 건드리지 않아요.
  - 토렌트가 없어졌으면 계획 때 저장한 파일 신원과 지금 파일이 같을 때만 `renameat2(RENAME_NOREPLACE)`로 바꿔요(`a_video_whose_torrent_is_gone_is_renamed_on_disk_without_replacing`). 목적지가 생겼으면 커널이 거절해요.
  - 목적지가 있으면 그 파일은 그대로 두고 `이름을 되돌리지 못했어요`와 까닭을 남기고, 나머지 파일은 계속 바꿔요(`a_name_that_is_taken_is_never_renamed_onto_and_the_others_go_on`).
  - 영상 수정본 기록(0025, 폴더와 회차 이름으로 영상을 찾아요)은 두 가지 중 더 안전한 쪽을 골랐어요. 끝난 기록(`done`·`failed`·`skipped` 등)은 파일 이름을 바꾼 기록과 같은 트랜잭션에서 새 이름으로 옮겨요. 옛 이름이나 새 이름에 진행 중인 기록(`receiving`·`verified`·`removing`·`removed`)이 있으면 되돌리기 전체를 거절하고 값도 바꾸지 않아요(`an_undo_waits_for_a_revision_replacement_under_way_and_moves_finished_ones`). 진행 중인 대체는 이전 영상을 지우고 새 영상에 회차 이름을 붙이는 중이라, 그 사이 이름이 바뀌면 대체가 엉뚱한 파일을 지우거나 이름을 잘못 붙일 수 있어요. 대체가 끝나면 다시 되돌릴 수 있어요.
  - 값은 이름을 바꾸기 전에 되살려요. 명령이 도는 동안 주기는 같은 잠금을 기다리므로 섞이지 않고, 그 뒤 받는 항목은 되살린 값으로 이름이 붙어요(위 시험의 세 번째 항목 `S03E27`).
  - 대상은 앱 값이 있는 동안 받은 `받음` 항목이에요. 앱이 값을 정하기 전에 받은 항목은 고른 항목이 없을 때만 정하므로 있을 수 없고, 되살린 뒤 받은 항목은 계획 뒤에 오므로 담기지 않아요. `중복` 항목은 받은 파일이 없어서 빼요.
  - 그 전 값(`episode_previous`)과 정한 적이 있다는 표시(`episode_decided`)는 설정 내보내기에 넣지 않아요. 둘 다 이 설치에서 받은 영상의 이름을 되돌리는 데만 쓰이고, 가져온 설치에는 그 영상과 수집 이력이 없어서 되돌릴 대상이 없어요. 가져오기에서 `episode_auto: true`인 규칙은 정한 것으로 봐요([설정 명세](../specs/settings.md#채널과-규칙-필드)).
- (2026-10-02 사용자 결정) 이전 시즌 일부부터 이어 센 번호는 제안만 해요. 1·2기가 24화씩인 3기가 `- 25`로 시작하면 `2기부터 이어 센 번호로 보여요. 회차 변환을 −24로 할까요?`와 `적용`이에요(`numbers_run_on_from_the_season_before_are_offered_and_never_set`). 범위는 바로 앞 시즌에서 끝나야 하고(`13`이 12·24화 뒤에 오면 1기만 이은 것이라 제안하지 않아요), 여럿이 맞으면 가장 짧은 범위를 제안하고 근거에 다른 범위도 적어요(`several_runs_that_match_offer_the_shortest_and_say_so`). `f > P + 1` 제안처럼 시즌 폴더의 영상은 보지 않아요. 규칙 상세가 이 제안을 보일 때는 이미 변환 없이 받은 영상(`S03E25`)이 폴더에 있기 때문이에요(`numbers_run_on_are_still_offered_once_the_folder_has_their_videos`).
- 이미 받기 시작한 규칙은 앱이 값을 바꾸지 않고, 규칙의 처음 항목과 지금 알려진 시즌 정보에서 맞는 값이 나오면 제안으로만 보여요(0인 값은 제안하지 않아요). `적용`해도 이미 받은 항목의 이름은 바뀌지 않아요.
- (2026-10-02 사용자 결정) 분할 방영의 뒤 쿨이 같은 시즌 폴더에서 `- 01`부터 다시 세면 양수 오프셋을 제안만 해요. 그 시즌에 AniList 항목이 둘 이상 이어져 있고 모두 회차 수를 알며, 폴더가 앞 쿨들(`1..C`)의 영상을 모두 갖고 그 뒤 회차는 갖지 않고, 첫 릴리스가 `- 01`일 때예요. `C`는 영상이 모두 있는 앞 쿨들의 회차 합계예요. 12화 쿨 뒤면 `2쿨을 1화부터 센 번호로 보여요. 1화를 13화로 받도록 회차 변환을 13으로 할까요?`와 `적용`이고, 적용하면 `- 01`이 `S02E13`이 돼요(`a_second_cour_that_restarts_at_one_is_offered_a_start_and_never_given_one`). 앞 쿨의 회차 하나가 없거나(`E07`), 뒤 쿨의 회차가 이미 있거나, 항목이 하나뿐이면 아무것도 하지 않아요(`a_restart_without_every_episode_of_the_first_cour_is_offered_nothing`, 단위 시험 `a_restart_is_offered_nothing_unless_the_earlier_cours_are_whole`).
  - 결정의 예시 문장은 `+12`였지만, 양수 회차 변환은 trname의 `starts_episode_at`처럼 "1화가 몇 화부터인지"라서 12는 `- 01`을 `S02E12`로 만들어요. 결정의 결과(`- 01` → `S02E13`)를 지키려고 값은 `C + 1`(13)로 제안해요. `+13`은 "13을 더한다"로 읽히므로, 문장은 값이 하는 일을 말하고(`1화를 13화로 받도록`) 값은 회차 변환 칸과 제안 상자처럼 부호 없이 써요.
  - 제안은 규칙이 첫 항목을 받은 뒤에 보여요. 그 첫 `- 01`은 변환 없이 `S02E01` 이름을 받으려 하는데, 그 이름은 앞 쿨의 영상이 갖고 있어서 이름을 바꾸지 않고 릴리스 이름 그대로 남아요. `적용`한 뒤 피드에 그 항목이 남아 있으면 다음 주기가 `S02E13`으로 이름을 마저 붙여요.
- 번호를 이어 세는 분할 방영(같은 폴더에 영상이 있고 `f = P + 1`)은 여전히 값 없는 안내만 해요.
- 재시도(실패한 항목을 다시 보내기)는 저장된 오프셋을 그대로 써요. 판단은 주기와 `receive_once`에서만 해요.
- 주기가 규칙을 읽은 뒤 사용자가 그 규칙을 저장했으면 규칙을 다시 읽어 한 번 더 판단해요. 그 저장이 회차 변환 값이나, 이번 주기의 항목을 고르고 놓은 일치 문구·정규식·대소문자·저장 폴더를 바꿨으면 사용자가 정한 대로 두어요(그 항목들은 바뀐 규칙이 고른 것이 아니니까요, `a_rule_whose_match_changes_while_its_first_release_is_read_is_not_decided`). 이렇게 하지 않으면 다른 필드를 고친 저장 하나로 첫 항목이 변환 없이 받아지고, 그 뒤로는 고른 항목이 있어 다시 판단하지 않아요(`a_rule_saved_while_its_first_release_is_read_still_gets_the_offset`, `an_offset_the_user_saves_while_the_first_release_is_read_is_kept`).
- 규칙 상세의 근거·제안을 읽지 못하면(저장소 오류) 그 규칙만 근거·제안 없이 보이고 로그를 남겨요. 채널·규칙 목록이 `500`이 되지 않아요(`a_suggestion_that_cannot_be_read_leaves_the_rule_list_answering`).

### 검증한 것

- `cargo fmt --check`와 `cargo clippy --all-targets -- -D warnings`가 통과했고, `cargo test --no-fail-fast`는 1278개 통과, 실패 0개, 무시 2개였어요(2026-10-02 결정까지 넣은 뒤, 문서 시험을 포함한 27개 실행). 처음 구현 때 첫 `cargo test` 실행에서 `tests/live_watch.rs`의 `a_quiet_hour_reads_no_folder_but_the_safety_net`이 한 번 실패했고, 이 변경이 건드리지 않은 시험이에요. 이어진 `--no-fail-fast` 전체 실행에서는 통과했어요(원인은 따로 조사 중이에요). 웹은 `bun install --frozen-lockfile`, `bun run typecheck`, `bun run build`가 통과했어요.
- 완료 기준의 행과 자동 시험(주기를 통과하는 통합 시험 `tests/episode_offset.rs`는 실제 임시 수집 폴더, 가짜 RSS, 가짜 Transmission, AniList 항목이 이어진 시즌을 써요):
  - 1·2기 합계 24화 → `−24`, `자동`과 근거, 첫 항목 `S03E01`: `the_first_item_of_a_third_season_is_named_from_the_sum_of_the_earlier_ones`(Transmission이 받은 이름과 규칙 응답의 `episode_basis`, 다음 `- 26`이 `S03E02`가 되는 것까지). 판단 자체는 `a_first_release_that_follows_the_earlier_seasons_is_converted_by_their_sum`.
  - 새 작품 `- 01` → 0, 묻지 않아요: `a_new_works_first_release_is_not_converted_and_nobody_is_asked`.
  - 처음 본 릴리스 `- 27` → 제안, `적용` 전에는 변환 없이 받아요: `a_first_release_in_the_middle_of_a_season_is_suggested_and_received_unconverted`(`적용` 호출, 오래된 버전의 `409` 포함).
  - 이전 시즌 회차 수를 모름 → 제안·직접 입력으로 남아요: `earlier_seasons_without_a_known_count_are_never_guessed`. 영상이 이미 있는 시즌 폴더: `a_season_folder_that_has_videos_already_keeps_the_app_from_choosing`.
  - 자동 값을 `−12`로 고침 → `자동`이 사라지고 덮지 않아요: `an_automatic_value_the_user_changes_loses_its_mark_and_stays_changed`, 저장소의 `the_app_decides_once_and_never_at_a_version_it_did_not_read`·`a_save_keeps_the_grounds_of_an_unchanged_value_and_drops_them_with_a_changed_one`.
  - 이미 받기 시작한 규칙, 지난 항목 하나를 먼저 고른 경우: `a_rule_that_picked_items_before_is_not_decided_by_later_ones`, `a_past_item_the_user_picks_first_is_named_with_the_decided_offset`.
  - `되돌리기`(가짜 Transmission이 디스크의 파일을 실제로 쓰고 이름을 바꿔요): 두 항목을 받은 뒤 되돌리면 −24, `자동` 꺼짐, Transmission의 토렌트가 그대로 있고 이름이 `S03E25`·`S03E26`, 세 번째 항목이 `S03E27`(`undoing_puts_the_previous_value_back_and_renames_what_it_named`). 목적지 파일이 있으면 그 파일과 원래 파일 모두 그대로이고 결과에 까닭이 남아요(`a_name_that_is_taken_is_never_renamed_onto_and_the_others_go_on`). 토렌트가 없어진 영상은 디스크에서 바꿔요(`a_video_whose_torrent_is_gone_is_renamed_on_disk_without_replacing`). 진행 중인 수정본 기록이 있으면 거절하고, 끝난 기록은 새 이름으로 옮겨요(`an_undo_waits_for_a_revision_replacement_under_way_and_moves_finished_ones`). 받은 항목 전에 되돌리면 다시 정하지 않아요(`an_undo_before_any_item_keeps_the_app_from_deciding_again`). 값이 바뀐 뒤의 요청은 `400`(`an_undo_of_a_value_that_is_no_longer_there_is_refused`). 저장한 파일 신원을 다시 읽는 것은 `src/revision.rs`의 `an_identity_kept_as_text_reads_back_and_a_rename_keeps_the_file`.
  - 이전 시즌 일부부터 이어 센 번호 → 제안만, `적용` 뒤 다음 항목이 `S03E02`: `numbers_run_on_from_the_season_before_are_offered_and_never_set`, 판단은 `numbers_run_on_from_a_later_season_are_suggested_not_set`·`several_runs_that_match_offer_the_shortest_and_say_so`·`numbers_run_on_are_still_offered_once_the_folder_has_their_videos`.
  - `- 01`부터 다시 세는 뒤 쿨 → 13 제안만, `적용` 뒤 `- 01`이 `S02E13`, `- 02`가 `S02E14`: `a_second_cour_that_restarts_at_one_is_offered_a_start_and_never_given_one`. `E07`이 없는 폴더와 항목 하나인 시즌 → 제안 없음: `a_restart_without_every_episode_of_the_first_cour_is_offered_nothing`. 판단과 13이 trname에서 `S02E13`이 되는 것은 `a_second_cour_that_restarts_at_one_is_offered_where_the_first_ended`, 제외 조건은 `a_restart_is_offered_nothing_unless_the_earlier_cours_are_whole`.
  - 설정 교환: 앱의 내보내기가 아직 없어서(설정 명세의 목표 5) 저장소의 가져오기 경로로만 봤어요: `an_import_keeps_the_grounds_of_an_automatic_value_it_leaves_as_it_is`. 마이그레이션: `rules_from_before_episode_grounds_keep_their_offsets_and_take_grounds`.
- 시험이 실패하는 것을 본 것: 주기의 판단 연결을 꺼 둔 채 통합 시험을 돌렸을 때 세 시험이 실패했고(연결을 되돌리면 통과), 나머지 새 시험은 새 기능에 대한 것이라 고치기 전 실패를 따로 보지 않았어요. 2026-10-02 결정의 시험은 고치기 전 코드에서 4개가 실패했어요. `되돌리기`의 안전장치는 하나씩 꺼 보고 해당 시험이 실패하는 것을 봤어요: 목적지 확인 두 곳을 끄면 `a_name_that_is_taken_…`, 진행 중인 기록의 거절을 끄거나 끝난 기록의 이동을 끄면 `an_undo_waits_for_…`. 이어 센 번호의 제안을 끄면 그 단위 시험 3개와 통합 시험이 실패했어요. 다시 세는 뒤 쿨의 제안을 끄면 `a_second_cour_that_restarts_at_one_is_offered_where_the_first_ended`와 통합 시험이 실패했어요.
- 브라우저(로컬 `trss-web`, 스크래치 DB, 데스크톱과 375px, 2026-10-01): 규칙 상세에서 `자동` 표시와 근거 문장, 값을 직접 고치면 표시와 근거가 사라지고 저장하면 DB의 `episode_auto`·`episode_basis`가 비는 것, 제안 상자(근거, 값, `적용`)와 `적용` 후 DB에 사용자 값(`episode_auto = 0`)으로 저장되는 것, 값 없는 안내(`적용` 없음), 필드를 고치면 제안이 숨고 되돌리면 다시 보이는 것, 375px에서 `scrollWidth == innerWidth`.

### 검증하지 못한 것

- 브라우저의 제안 상자는 제안을 서버가 만든 것이 아니라 응답에 끼워 넣어(`fetch` 가로채기) 봤어요. 서버가 제안을 만드는 쪽은 통합 시험의 규칙 응답으로 봤어요. 자동 값은 DB에 직접 넣었어요.
- 실제 Transmission, 실제 AniList, 실제 수집 폴더로는 확인하지 않았어요. `되돌리기`가 기대는 Transmission의 이름 변경 동작(목적지가 있으면 옮기지 않고 성공으로 답해요)은 libtransmission 소스를 읽고 가짜 Transmission에 옮긴 것이에요.
- `되돌리기`의 화면(진행, 결과, `같은 요청 다시 보내기`)은 브라우저에서 보지 않았어요. `bun run typecheck`와 `bun run build`만 통과했어요.
- 토렌트가 없어진 영상의 파일이 계획과 실행 사이에 바뀐 경우(`FILE_CHANGED`)는 통합 시험이 없어요. 신원 비교 자체는 단위 시험으로 봤어요.
- 토렌트가 없어졌고 수집 이력의 제목에 확장자가 없는 항목(RSS 제목)은 파일 이름을 알 수 없어 계획에서 빠져요.
- 설정 내보내기(YAML)가 아직 없어서 `episode_auto`의 왕복은 가져오기 쪽 저장소 시험으로만 봤어요.
- 브라우저에서 `409` 문구, 다크 모드, 실제 휴대폰·스크린 리더, 376–1099px 너비는 보지 않았어요. 이 저장소에는 웹 자동 시험 도구가 없어요.

### 남은 일

- 사용자가 `0`·`1`을 직접 적은 규칙과 건드리지 않은 규칙을 가르지 못하는 한계는 2026-10-02 결정으로 풀렸어요. 첫 항목 전에는 칸의 값과 상관없이 판단하므로 구분할 필요가 없어졌어요.
- 분할 방영의 양수 오프셋은 `- 01`부터 다시 세는 뒤 쿨만 제안해요(2026-10-02 결정, 위). 같은 폴더에서 번호를 이어 세는 분할 방영은 값 없는 안내로 남아요([0026](0026-past-episode-search.md)의 범위 제안이 이 근거를 쓸 수 있어요).
- 제안은 칸이 `0`·`1`이고 앱이 정한 적이 없는 규칙에만 보여요. 이전 시즌에서 옮겨 온 다른 값(`−12` 등)이 남은 채 받기 시작한 규칙에는 맞는 값이 나와도 제안하지 않아요. 넓힐지는 정하지 않았어요.
