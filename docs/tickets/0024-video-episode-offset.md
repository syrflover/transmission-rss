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
- 저장소(`src/store/channels/episode.rs`): `set_auto_episode`는 오프셋이 아직 `0` 또는 `1`이고 앱이 정한 값이 아니며 읽은 버전 그대로일 때만 값·`episode_auto`·근거를 한 번에 써요. 사용자가 적었거나 고친 값은 덮지 않아요. `set_episode`는 `적용`처럼 사용자 값을 저장하고 `자동`을 꺼요.
- 판단(`src/episode_offset.rs`): 처음 본 릴리스 `f`, 이전 시즌 합계 `P`, 시즌 폴더가 이미 가진 영상으로 표로 정해요(표는 모듈 문서에 있어요). `f = P + 1`이고 폴더에 영상이 없으면 `−P`(`f = 1`이면 0)를 자동으로 정해요. `f > P + 1`이면 `−P` 제안, 이전 시즌 합계를 모르거나 폴더에 영상이 있어 어느 회차인지 말할 수 없으면 값 없는 안내만 해요. 번호가 시즌 안에서 다시 시작하는 경우(`f ≤ P`)와 `P = 0`에 `f > 1`인 경우는 아무것도 하지 않아요.
- 주기(`src/worker/cycle.rs`, `src/worker/offsets.rs`): 항목을 Transmission에 보내기 전에, 아직 아무 항목도 고르지 않은 구독 규칙마다 그 주기의 가장 낮은 정수 회차로 판단하고 값을 정하면 그 주기의 작업에 바로 써요. 이미 받은 항목의 이름은 바꾸지 않아요. 사용자가 지난 항목 하나를 먼저 골라 받는 `receive_once`도 같은 판단을 거쳐요(`src/worker/commands/receive_once.rs`).
- 웹 API: 규칙 응답에 `episode_basis`와 `episode_suggestion { value, basis }`, `PUT /api/rules/{id}/episode`(`{ version, episode }`, 버전이 어긋나면 `409`와 `current`). 제안은 열 때마다 수집 이력·라이브러리·AniList 합계에서 읽어서, 사용자가 시즌을 이은 뒤에 나타나고 오프셋을 정하면 사라져요.
- 웹 화면(`EpisodeGrounds.tsx`, `RuleDetail.tsx`): `자동` 표시 아래에 근거 문장, 제안(근거 문장, 값, `적용`)을 보여요. 값을 적을 수 없는 안내에는 `적용`이 없어요. 필드를 고치면 둘 다 숨고, `적용`은 스위치처럼 바로 저장해요.

### 결정

- "그 규칙이 처음 본 릴리스"는 그 규칙이 처음 고른 항목들 중 가장 낮은 정수 회차예요. 수집 이력에서는 `result_at`이 가장 이른 항목이에요. 주기는 항목을 보내기 전에 같은 주기의 항목 중에서 정해요.
- 시즌은 이어진 시즌이 있으면 그것이고, 없으면 규칙의 저장 폴더(`<작품>/Season NN`)가 가리키는 시즌이에요. 티켓의 "구독이 이어진 시즌(0019)"은 규칙이 영상을 받은 뒤에야 생기므로, 그대로면 새 구독의 첫 항목을 `S03E01`로 받는다는 완료 기준의 첫 행이 이뤄질 수 없어서 저장 폴더를 근거로 더했어요. 작품은 수집 폴더 밑에 있는 라이브러리의 작품(수집 폴더는 자동 감시 폴더예요)이고, 이전 시즌 폴더가 라이브러리에 없거나 AniList가 이어지지 않았거나 회차 수가 비면 합계를 모르는 것으로 보고 짐작하지 않아요.
- 아직 이어지지 않은 구독에서 이어짐을 기다리는 보수적인 방식 대신 이 방식을 골랐고, 되돌리려면 `src/episode_offset.rs`의 `gather`에서 저장 폴더 분기를 빼면 돼요.
- `episode_auto = false`는 "아직 안 건드림"과 "사용자가 0이나 1을 적음"을 가르지 못해요(명세의 데이터 모델에 그 구분이 없어요). 그래서 오프셋이 `0`·`1`이고 앱이 정한 값이 아니며 아직 고른 항목이 없는 규칙만 판단해요. 사용자가 고른 항목이 생기기 전에 `0`이나 `1`을 직접 적어 두었는데 첫 릴리스가 정확히 `P + 1`이면 앱이 덮을 수 있어요.
- 이미 받기 시작한 규칙은 앱이 값을 바꾸지 않고, 규칙의 처음 항목과 지금 알려진 시즌 정보에서 맞는 값이 나오면 제안으로만 보여요(0인 값은 제안하지 않아요). `적용`해도 이미 받은 항목의 이름은 바뀌지 않아요.
- 분할 방영처럼 번호가 이어지는 경우의 양수 오프셋은 계산하지 않아요(값 없는 안내만 해요).
- 재시도(실패한 항목을 다시 보내기)는 저장된 오프셋을 그대로 써요. 판단은 주기와 `receive_once`에서만 해요.
- 주기가 규칙을 읽은 뒤 사용자가 그 규칙을 저장했으면 규칙을 다시 읽어 한 번 더 판단해요. 그 저장이 회차 변환 값을 바꿨으면 사용자가 정한 대로 두어요. 이렇게 하지 않으면 다른 필드를 고친 저장 하나로 첫 항목이 변환 없이 받아지고, 그 뒤로는 고른 항목이 있어 다시 판단하지 않아요(`a_rule_saved_while_its_first_release_is_read_still_gets_the_offset`, `an_offset_the_user_saves_while_the_first_release_is_read_is_kept`).
- 규칙 상세의 근거·제안을 읽지 못하면(저장소 오류) 그 규칙만 근거·제안 없이 보이고 로그를 남겨요. 채널·규칙 목록이 `500`이 되지 않아요(`a_suggestion_that_cannot_be_read_leaves_the_rule_list_answering`).

### 검증한 것

- `cargo fmt --check`와 `cargo clippy --all-targets -- -D warnings`가 통과했고, `cargo test --no-fail-fast`는 1051개 통과, 실패 0개, 무시 2개였어요(24개 실행 파일). 같은 변경의 첫 `cargo test` 실행에서 `tests/live_watch.rs`의 `a_quiet_hour_reads_no_folder_but_the_safety_net`이 한 번 실패했고, 이 변경이 건드리지 않은 시험이에요. 이어진 `--no-fail-fast` 전체 실행에서는 통과했어요(원인은 따로 조사 중이에요). 웹은 `bun install --frozen-lockfile`, `bun run typecheck`, `bun run build`가 통과했어요.
- 완료 기준의 행과 자동 시험(주기를 통과하는 통합 시험 `tests/episode_offset.rs`는 실제 임시 수집 폴더, 가짜 RSS, 가짜 Transmission, AniList 항목이 이어진 시즌을 써요):
  - 1·2기 합계 24화 → `−24`, `자동`과 근거, 첫 항목 `S03E01`: `the_first_item_of_a_third_season_is_named_from_the_sum_of_the_earlier_ones`(Transmission이 받은 이름과 규칙 응답의 `episode_basis`, 다음 `- 26`이 `S03E02`가 되는 것까지). 판단 자체는 `a_first_release_that_follows_the_earlier_seasons_is_converted_by_their_sum`.
  - 새 작품 `- 01` → 0, 묻지 않아요: `a_new_works_first_release_is_not_converted_and_nobody_is_asked`.
  - 처음 본 릴리스 `- 27` → 제안, `적용` 전에는 변환 없이 받아요: `a_first_release_in_the_middle_of_a_season_is_suggested_and_received_unconverted`(`적용` 호출, 오래된 버전의 `409` 포함).
  - 이전 시즌 회차 수를 모름 → 제안·직접 입력으로 남아요: `earlier_seasons_without_a_known_count_are_never_guessed`. 영상이 이미 있는 시즌 폴더: `a_season_folder_that_has_videos_already_keeps_the_app_from_choosing`.
  - 자동 값을 `−12`로 고침 → `자동`이 사라지고 덮지 않아요: `an_automatic_value_the_user_changes_loses_its_mark_and_stays_changed`, `an_offset_the_user_typed_is_never_overwritten`, 저장소의 `the_app_never_replaces_what_the_user_set_or_saw_change`·`a_save_keeps_the_grounds_of_an_unchanged_value_and_drops_them_with_a_changed_one`.
  - 이미 받기 시작한 규칙, 지난 항목 하나를 먼저 고른 경우: `a_rule_that_picked_items_before_is_not_decided_by_later_ones`, `a_past_item_the_user_picks_first_is_named_with_the_decided_offset`.
  - 설정 교환: 앱의 내보내기가 아직 없어서(설정 명세의 목표 5) 저장소의 가져오기 경로로만 봤어요: `an_import_keeps_the_grounds_of_an_automatic_value_it_leaves_as_it_is`. 마이그레이션: `rules_from_before_episode_grounds_keep_their_offsets_and_take_grounds`.
- 시험이 실패하는 것을 본 것: 주기의 판단 연결을 꺼 둔 채 통합 시험을 돌렸을 때 세 시험이 실패했고(연결을 되돌리면 통과), 나머지 새 시험은 새 기능에 대한 것이라 고치기 전 실패를 따로 보지 않았어요.
- 브라우저(로컬 `trss-web`, 스크래치 DB, 데스크톱과 375px, 2026-10-01): 규칙 상세에서 `자동` 표시와 근거 문장, 값을 직접 고치면 표시와 근거가 사라지고 저장하면 DB의 `episode_auto`·`episode_basis`가 비는 것, 제안 상자(근거, 값, `적용`)와 `적용` 후 DB에 사용자 값(`episode_auto = 0`)으로 저장되는 것, 값 없는 안내(`적용` 없음), 필드를 고치면 제안이 숨고 되돌리면 다시 보이는 것, 375px에서 `scrollWidth == innerWidth`.

### 검증하지 못한 것

- 브라우저의 제안 상자는 제안을 서버가 만든 것이 아니라 응답에 끼워 넣어(`fetch` 가로채기) 봤어요. 서버가 제안을 만드는 쪽은 통합 시험의 규칙 응답으로 봤어요. 자동 값은 DB에 직접 넣었어요.
- 실제 Transmission, 실제 AniList, 실제 수집 폴더로는 확인하지 않았어요.
- 설정 내보내기(YAML)가 아직 없어서 `episode_auto`의 왕복은 가져오기 쪽 저장소 시험으로만 봤어요.
- 브라우저에서 `409` 문구, 다크 모드, 실제 휴대폰·스크린 리더, 376–1099px 너비는 보지 않았어요. 이 저장소에는 웹 자동 시험 도구가 없어요.

### 남은 일

- 사용자가 `0`·`1`을 직접 적은 규칙과 건드리지 않은 규칙을 가르는 상태가 데이터 모델에 없어요. 구분이 필요하면 명세의 `episode_auto`를 세 상태로 늘리거나 "사용자가 정함" 표시를 더해야 해요(명세 변경이라 사용자 결정이에요).
- 분할 방영처럼 번호가 이어지는 시즌의 양수 오프셋 제안은 계산하지 않아요([0026](0026-past-episode-search.md)의 범위 제안이 이 근거를 쓸 수 있어요).
