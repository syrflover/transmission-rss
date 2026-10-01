# 0026 규칙 상세에서 지난 회차를 검색해 골라 받아요

- 상태: 완료 (실제 Transmission으로는 확인하지 않았고, 실제 nyaa는 검색 RSS 한 번만 읽었어요. 아래 "검증하지 못한 것")
- 출처: [지난 회차 검색](../specs/collection.md#지난-회차-검색), [채널과 규칙 필드](../specs/settings.md#채널과-규칙-필드)(`past_search`), [수집 화면](../specs/collection.md#수집-화면)(채널 탭의 검색 형식)
- 막는 티켓: [0024](0024-video-episode-offset.md)(범위 제안의 회차 변환), [0025](0025-replace-video-revisions.md)(수정본·`버전 미상` 판정)

## 작업

RSS에서 빠진 지난 회차를 규칙 조건으로 만든 nyaa 검색 RSS를 한 번 읽어 확보해요. 규칙 상세의 `지난 회차 검색`에서 범위 확인 → 검색 결과 미리보기 → 받기의 세 단계로 진행하고, 상시 채널로 남기지 않아요.

- 검색어: 채널의 지난 회차 검색 형식(`[SubsPlease] {match} 1080p`)에 규칙의 일치 문구를 넣어요. 형식이 없으면 일치 문구로 채운 검색어를 그 자리에서 고쳐 한 번 검색해요. 채널 탭에서 형식을 보고 고쳐요.
- 범위: `릴리스 13–24 → S02E01–12`처럼 릴리스 회차와 폴더 회차의 대응을 trss가 제안하고(AniList 회차 수, 0024의 회차 변환, 구독 뒤 처음 본 RSS 항목의 회차), 근거가 없으면 사용자가 적어요.
- 미리보기: 결과는 실제 규칙 판정을 거치고, 범위 밖은 접어 개수만, 빠진 회차는 범위에서 이미 있는 회차를 뺀 것이에요. 폴더·Transmission에 없는 개별 회차만 기본 선택하고, 이미 있는 회차와 배치는 해제, 같은 회차 수정본이 여럿이면 가장 높은 것만, 다른 릴리스로 이미 받은 회차는 회차 번호와 작품 폴더의 실제 파일로 판단하고, 버전을 모르는 기존 영상과 CRC가 다른 결과는 `버전 미상`(0025의 판정)이에요. 이력의 지난 항목과 같은 미리보기 구성 요소를 써요.
- 추가 검색: 첫 결과가 75개로 차서 잘렸을 수 있으면, 범위에서 아직 없는 회차를 묶어 `{작품} - (1000|1001|1002)` 형태로 간격을 두고 순서대로 더 검색해 같은 미리보기에 합쳐요. 회차 표기는 첫 결과의 실제 제목에서 읽은 형식을 써요. 한 묶음의 회차 수와 요청 간격은 구현 때 유한한 값으로 정해 티켓에 남겨요. HTML 검색 페이지는 쓰지 않아요.
- 받기: 사용자가 확인한 항목만 Transmission에 추가해요(웹 명령 → worker). 검색 요청은 웹이 하든 worker가 하든 nyaa에 대한 요청 간격을 한 곳에서 지켜요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| `[SubsPlease] Sayonara Lara 1080p`처럼 한 번에 다 오는 검색(가짜 nyaa) | 범위 안의 빠진 회차만 기본 선택되고, 확인한 것만 Transmission에 추가돼요. |
| 1기·2기·수정본 `14v2`·공식/비공식 배치가 섞인 결과 | 범위 밖(다른 시즌)은 접혀 개수만, 배치는 해제, `14`와 `14v2` 중 `14v2`만 선택돼요. |
| 이미 폴더에 있는 회차 / 다른 릴리스로 받은 회차 | 표시되되 선택이 해제돼요. 토렌트 해시 거부에 기대지 않아요. |
| 버전을 모르는 기존 영상과 CRC가 다른 결과 | `버전 미상`으로 표시되고 해제돼요. |
| 75개로 찬 첫 결과(가짜 nyaa) | 빠진 회차를 묶은 추가 검색이 간격을 두고 이어지고, 결과가 같은 미리보기에 누락·중복 없이 합쳐져요. 장편 표본으로 확인해요. |
| 채널에 검색 형식이 없음 | 일치 문구로 채운 검색어를 그 자리에서 고쳐 검색할 수 있어요. |
| 검색을 끝내거나 떠남 | 상시 채널이 생기지 않고, 수집 이력에는 받은 항목만 남아요. |
| 휴대폰 너비 | 세 단계와 미리보기에 가로 스크롤이 없어요. |

## 결과

### 구현한 것

- 검색(`src/past_search/`): 웹이 검색어를 만들고(`query.rs`), 채널 주소에서 `q`를 검색어로 바꿔(`p`는 빼고, 나머지 쿼리 값은 그대로) nyaa 검색 RSS를 읽고(`client.rs`), 결과를 규칙 판정과 같은 규칙으로 가려(`judge.rs`, `release.rs`) 미리보기를 만들어요. 판정은 릴리스 회차를 회차 변환(0024의 `episode_offset`)으로 폴더 회차에 옮긴 뒤 작품 폴더의 실제 파일(`Show S02E05.mkv`, `.part`)과 이력(규칙·채널의 받음·중복 기록)으로 해요. 토렌트 해시는 쓰지 않아요.
- 범위 제안(`range.rs`): AniList 회차 수, 회차 변환, 구독 뒤 처음 본 RSS 항목의 회차 순으로 `릴리스 13–24 → S02E01–12`를 제안하고, 근거가 없으면 비워 둬요.
- 추가 검색(`run.rs`): 첫 결과가 75개로 차면 범위에서 아직 없는 회차를 10개씩 묶어 `{작품} - (1000|1001|…)`로 3초 간격을 두고 순서대로 검색하고, 같은 키의 결과를 합쳐요. 회차 표기(자릿수)는 첫 결과의 실제 제목에서 읽어요.
- 간격(`src/store/search_pace/`, 마이그레이션 26): 호스트마다 다음에 보내도 되는 시각을 `search_pace` 행으로 두고, 요청마다 한 쓰기 트랜잭션에서 자리를 받아 그 시각까지 기다린 뒤 보내요. 웹의 검색 여럿과 worker가 같은 행을 써서 요청 간격을 한 곳에서 지켜요. `429`는 `Retry-After`만큼 그 호스트의 자리를 막고 검색을 실패로 끝내요(`FetchError::Busy`).
- 웹(`src/past_search/service.rs`, `src/web/past_search_api.rs`): 검색은 웹 프로세스의 메모리에서 돌고(규칙마다 하나, 새 검색은 이전 검색을 끝내요. 최대 6개, 30분 보관), 폴링으로 진행(`sent`/`needed`)과 결과를 돌려줘요. 결과의 링크는 서버가 쥐고 있고 브라우저는 `{rule_id, search_id, key}`만 보내요. 결과에 없는 키는 거절해요.
- worker(`src/worker/commands/receive_past.rs`): `receive_past` 명령이 고른 항목을 `receive_once`와 같은 실행으로 추가하고 이력에 남겨요. 대체 후보(수정본)는 추가 직후 이름 변경을 건너뛰고 0025의 `video_revisions`로 이어지고, 고른 `버전 미상`은 `다시 받기`의 확인과 같게 다뤄요.
- 화면(`web/src/screens/collect/rules/past-search/`, `RuleDetail.tsx`, `ChannelEditor.tsx`): 규칙 상세의 `지난 회차 검색` 세 단계, 이력의 지난 항목과 같이 쓰는 `SelectionBar`·`PickRow`·`ProgressRows`(`PastItems.tsx`), 명령 행을 따라가는 `useCommandRows`(`useReceive.ts`). 채널 편집의 형식 안내에 `{match}`를 설명해요.
- 운영 안내: `readme.md`의 Past episode search.

### 유한한 값

| 값 | 크기 | 까닭 |
| --- | --- | --- |
| 한 쪽의 결과 | 75 (`PAGE_LIMIT`) | nyaa 검색 RSS의 한 쪽 크기. 이 수로 차면 잘렸을 수 있다고 봐요. |
| 한 묶음의 회차 수 | 10 (`BATCH_SIZE`) | 묶음 검색어의 길이와 결과가 75개 안에 들 크기. |
| 추가 검색 | 20번까지 (`MAX_EXTRA_SEARCHES`) | 200회차까지 따라가요. 3초 간격이라 한 검색이 1분 안에 끝나요. |
| 요청 간격 | 3초 (`REQUEST_SPACING`) | nyaa에 대한 요청 사이의 최소 시간. 한 호스트에 대해 웹과 worker가 함께 지켜요. |
| 범위의 폭 | 2000회차 (`MAX_SPAN`), 결과 2000개 (`MAX_RESULTS`) | 메모리와 미리보기의 크기를 묶어 둬요. |
| CRC 읽기 | 검색마다 영상 20개 (`MAX_CRC_READS`) | `버전 미상`을 가리려고 기존 영상을 읽는 일이 한 검색에서 이어지지 않게 해요. |
| 작품 폴더 읽기 | 파일 5000개 (`MAX_FILES`) | 폴더 하나에서 앞의 5000개만 보고, 그 뒤의 파일은 없는 것으로 판단해요. |

### 결정

- 대체 후보(이미 있는 회차의 더 높은 수정본)와 `버전 미상`은 보여 주되 기본 선택하지 않아요. 명세가 "폴더·Transmission에 없는 개별 회차만 기본 선택"이라고 하므로 가장 보수적으로 읽었어요.
- 일치 문구를 쓸 수 없는 규칙(정규식, 제목을 기다리는 규칙)은 검색어를 빈 칸으로 시작하고 까닭을 문장으로 알려요. 형식이 없는 채널은 일치 문구로 채워요.
- Transmission에는 묻지 않고(웹은 Transmission에 닿지 않아요) 다른 릴리스로 이미 받았는지를 규칙·채널 이력과 작품 폴더의 실제 파일로 판단해요. 받는 중이라 파일이 아직 없는 회차는 이력이 알려 줘요.
- 간격은 검색 요청에만 적용해요. worker 주기의 피드 읽기는 바꾸지 않았어요.
- 받기에 실패한 검색 항목은 `추가 실패` 이력으로 남아 `다시 받기`로 다시 받을 수 있어요. 받기를 고른 항목만 이력에 남기므로 검색을 끝내거나 떠나도 이력에는 받은 항목만 있어요.
- `receive_past`는 규칙의 회차 변환을 새로 정하지 않아요. 사람이 범위를 현재 변환으로 확인했기 때문이에요.
- 검색으로 받은 토렌트도 피드에 없는 토렌트로 보아 기존의 사라진 토렌트 정리 대상이에요(피드에 없는 다른 항목과 같아요).
- 4자리 회차의 파일 이름은 `trname`의 기존 한계를 그대로 따라요.
- 마이그레이션은 26번이에요(`search_pace`).

### 검증한 것

`tests/past_search.rs`(웹 API를 실제로 띄우고, 가짜 nyaa·가짜 Transmission·임시 폴더·실제 worker 명령 실행을 써요) 16개와 `src/past_search/` 단위 시험, `src/store/search_pace/` 시험이에요. 가짜 nyaa는 검색어를 단어로 맞추고 75개에서 자르고 요청 순서·시각을 기록해요.

| 기준 행 | 시험 |
| --- | --- |
| 1 한 번에 다 오는 검색 | `row_1_a_search_that_returns_all_selects_the_missing_episodes_and_adds_only_the_confirmed` |
| 2 1기·2기·`14v2`·배치 | `row_2_other_seasons_are_folded_batches_are_not_selected_and_only_the_revision_is` |
| 3 폴더에 있는 회차·다른 릴리스 | `row_3_episodes_in_the_folder_or_received_from_another_release_are_shown_and_not_selected` |
| 4 `버전 미상` | `row_4_a_video_of_unknown_version_with_another_crc_makes_the_result_unknown_and_unselected` |
| 5 75개로 찬 첫 결과 | `row_5_a_full_first_page_is_followed_by_spaced_searches_merged_without_loss_or_repeat`(1001–1100 장편 표본, 요청 4번, 간격 검사) |
| 6 형식 없는 채널 | `row_6_without_a_format_the_query_starts_from_the_match_phrase_and_is_edited_in_place` |
| 7 상시 채널 없음 | `row_7_a_search_leaves_no_channel_and_history_holds_only_the_received_items` |
| 8 휴대폰 너비 | 브라우저(아래) |

- 그 밖: 거절되는 요청(`a_request_that_cannot_be_searched_is_refused_before_any_request_is_sent`), `429`(`a_tracker_that_asks_to_wait_fails_the_search_and_says_for_how_long`), 새 검색이 이전 검색을 끝냄, 규칙이 고르지 않은 결과의 거절, 수정본을 골라 받아 확인한 뒤 대체(`a_revision_chosen_in_the_preview_replaces_the_video_after_it_is_received_and_checked`).
- 0025 수정본 대체의 보완과 합친 뒤 다시 본 것: 검색으로 받은 수정본도 피드의 수정본과 같은 대체 상태 기계를 지나므로, 한 회차의 대체는 한 번에 하나이고 낮은 수정본이 높은 수정본을 대체하지 않아요. 같은 회차의 `v2`·`v3`를 검색으로 함께 받고 `v2`가 먼저 끝난 경우(`two_searched_revisions_of_an_episode_leave_the_higher_one`)와 피드의 `v2`가 받는 중일 때 검색으로 `v3`를 받은 경우(`a_searched_revision_higher_than_the_feeds_open_one_is_the_one_that_replaces`)는 그 보완 전 코드에서 `v2`가 회차 이름을 차지해 실패했고, 보완 뒤 `v3`만 대체해요. 이미 더 높은 수정본이 대체한 회차의 낮은 수정본은 검색으로 골라도 추가하지 않고 `중복`으로 끝나요(`a_searched_revision_lower_than_the_one_that_replaced_the_video_is_not_added`, 피드 주기와 같은 판단을 써요).
- 5행의 간격 검사는 간격을 0으로 바꿨을 때 실패하는 것을 봤어요. 간격 시험은 처음에 60 ms 간격에 여유 5 ms로 두어, 병렬 실행에서 한 번 54.9 ms로 실패했어요(도착 시각을 재므로 스케줄링 흔들림이 들어가요). 150 ms에 여유 60 ms로 고쳐 다섯 번 연속 통과했어요.
- 브라우저(로컬 `trss-web`, 가짜 nyaa, 임시 DB·폴더): 범위 입력 → 검색 → 미리보기(1–24, 결과 27개, 22개 기본 선택, 폴더에 있는 3·4화는 `이미 있어요`로 해제) → 2개를 골라 `선택한 2개 받기` → 세 번째 단계에서 `추가하는 중`(worker가 없어 명령이 열려 있는 상태)까지 봤어요. 375 px에서 세 단계와 미리보기 모두 `scrollWidth`가 `clientWidth`와 같았어요(가로 스크롤 없음, 가로로 넘치는 요소 없음).
- `cargo fmt --check` 통과, `cargo clippy --all-targets -- -D warnings` 통과, `cargo test --no-fail-fast`는 1193개 통과·실패 0개·무시 2개(27개 실행 파일), 웹은 `bun install --frozen-lockfile`·`bun run typecheck`·`bun run build` 통과.
- 실제 nyaa는 검색 RSS를 한 번만 읽어 응답 모양을 확인했어요.

### 검증하지 못한 것

- 실제 Transmission으로 추가·이름 변경·완료 확인(가짜 Transmission만 썼어요).
- 실제 nyaa의 `429`와 여러 번의 묶음 검색(가짜 nyaa로만 봤어요).
- 브라우저에서는 worker가 없어 받기가 끝나는 모습(`추가됨`, `받기 실패`와 `다시 받기`)을 보지 못했어요. 그 흐름은 `tests/past_search.rs`가 웹 API와 worker 명령 실행으로 확인해요.
- 데스크톱 너비는 900 px 부근에서만 보았고 태블릿 너비는 보지 않았어요.

### 남은 일

- 마스크된 링크(다른 호스트의 비밀 값)를 가진 검색 결과의 원래 링크 복원은 nyaa 링크로만 확인했어요.
