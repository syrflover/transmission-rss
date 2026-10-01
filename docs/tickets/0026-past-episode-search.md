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

- 검색(`src/past_search/`): 웹이 검색어를 만들고(`query.rs`), 채널 주소에서 `q`를 검색어로 바꿔(`p`는 빼고, 나머지 쿼리 값은 그대로) nyaa 검색 RSS를 읽고(`client.rs`), 결과를 규칙 판정과 같은 규칙으로 가려(`judge.rs`, `release.rs`) 미리보기를 만들어요. 판정은 릴리스 회차를 회차 변환(0024의 `episode_offset`)으로 폴더 회차에 옮긴 뒤 작품 폴더의 실제 파일(`Show S02E05.mkv`, `.part`)과 이력(규칙·채널의 받음·중복 기록)으로 해요. 같은 토렌트인지 가리는 데는 토렌트 해시를 쓰지 않고, 이력의 토렌트가 지금 Transmission에 있는지만 아래의 목록으로 봐요.
- 범위 제안(`range.rs`): AniList 회차 수, 회차 변환, 구독 뒤 처음 본 RSS 항목의 회차 순으로 `릴리스 13–24 → S02E01–12`를 제안하고, 근거가 없으면 비워 둬요.
- 추가 검색(`run.rs`): 첫 결과가 75개로 차면 범위에서 아직 없는 회차를 10개씩 묶어 `{작품} - (1000|1001|…)`로 3초 간격을 두고 순서대로 검색하고, 같은 키의 결과를 합쳐요. 회차 표기(자릿수)는 첫 결과의 실제 제목에서 읽어요.
- 간격(`src/store/search_pace/`, 마이그레이션 26): 호스트마다 다음에 보내도 되는 시각을 `search_pace` 행으로 두고, 요청마다 한 쓰기 트랜잭션에서 자리를 받아 그 시각까지 기다린 뒤 보내요. 웹 프로세스가 여럿이어도, 한 프로세스의 검색이 여럿이어도 같은 행을 써서 요청 간격을 지켜요. 검색 요청은 웹만 보내고 worker는 이 행을 쓰지 않아요(worker 주기의 피드 읽기는 이 간격의 대상이 아니에요). 자리가 60초보다 멀면(`429`로 막힌 호스트, 시계가 앞으로 뛴 경우) 자리를 받지 않고 검색을 실패로 끝내며 다시 시도할 때를 알려요. 기다리는 동안 다른 검색이 `429`를 받으면 그 막힘을 다시 읽어 요청을 보내지 않아요. `429`는 `Retry-After`만큼 그 호스트의 자리를 막고 검색을 실패로 끝내요(`FetchError::Busy`).
- 웹(`src/past_search/service.rs`, `src/web/past_search_api.rs`): 검색은 웹 프로세스의 메모리에서 돌고(규칙마다 하나, 새 검색은 이전 검색을 끝내요. 최대 6개, 시작한 지 30분 뒤에는 폴링하든 안 하든 받을 수 없어요), 폴링으로 진행(`sent`/`needed`)과 결과를 돌려줘요. 결과의 링크는 서버가 쥐고 있고 브라우저는 `{rule_id, search_id, key}`만 보내요. 결과에 없는 키는 거절해요.
- worker(`src/worker/commands/receive_past.rs`): `receive_past` 명령이 고른 항목을 `receive_once`와 같은 실행으로 추가하고 이력에 남겨요. 대체 후보(수정본)는 추가 직후 이름 변경을 건너뛰고 0025의 `video_revisions`로 이어지고, 고른 `버전 미상`은 `다시 받기`의 확인과 같게 다뤄요.
- 화면(`web/src/screens/collect/rules/past-search/`, `RuleDetail.tsx`, `ChannelEditor.tsx`): 규칙 상세의 `지난 회차 검색` 세 단계, 이력의 지난 항목과 같이 쓰는 `SelectionBar`·`PickRow`·`ProgressRows`(`PastItems.tsx`), 명령 행을 따라가는 `useCommandRows`(`useReceive.ts`). 채널 편집의 형식 안내에 `{match}`를 설명해요.
- 지워진 항목의 재선택(`src/past_search/world.rs`, `src/store/status/`, 마이그레이션 27): 이력에 받음·중복으로 남은 항목도 영상이 작품 폴더에 없고 토렌트가 Transmission에서 사라졌으면 다시 고를 수 있어요(`departed`). 웹은 Transmission에 닿지 않으므로 worker가 주기마다 Transmission의 모든 토렌트 해시와 그 시각을 `transmission_listing`·`transmission_torrents`에 남기고(`TorrentListing`), 검색이 그 목록과 이력 항목의 토렌트 해시를 견줘요. 항목이 받음이 된 시각보다 목록이 앞서거나 목록이 아직 없거나 항목의 해시를 모르면 토렌트가 있는 것으로 봐요. worker의 `receive_past`는 같은 판단을 Transmission의 지금 목록과 작품 폴더로 다시 해서(`receive_past::departed`) 화면이 내놓은 항목을 `이미 Transmission에 있어요`로 거절하지 않고 받아요. 받은 뒤에도 이력의 결과는 `받음` 그대로예요. 지워진 것으로 보는 항목은 단일 회차뿐이에요(배치와 회차 없는 항목은 토렌트를 다 받고 빼는 것이 보통이라 계속 `이미 받은 항목`이에요). 작품 폴더가 없거나(마운트되지 않은 볼륨 포함) 항목이 5000개를 넘어 끝까지 읽지 못했으면 영상이 없다고 말할 수 없으므로 어떤 항목도 지워진 것으로 보지 않고, 같은 규칙의 다른 토렌트가 같은 회차로 Transmission에 있어도 그래요. worker도 같은 폴더·토렌트 확인을 하고, 낮은 수정본을 더 높은 수정본이 대체한 폴더에는 지워진 항목이어도 다시 받지 않는 기존 판단(`holds_higher`)을 그대로 지켜요. 웹은 검색이 지워졌다고 본 항목(`Stored.departed`)만 `받음` 결과로 접수하고, 접수할 때 규칙이 켜져 있고 제목을 고르는지(`adoption_plan`)를 아직 받지 않은 항목처럼 확인해요.
- 운영 안내: `readme.md`의 Past episode search.

### 유한한 값

| 값 | 크기 | 까닭 |
| --- | --- | --- |
| 한 쪽의 결과 | 75 (`PAGE_LIMIT`) | nyaa 검색 RSS의 한 쪽 크기. 이 수로 차면 잘렸을 수 있다고 봐요. |
| 한 묶음의 회차 수 | 10 (`BATCH_SIZE`) | 묶음 검색어의 길이와 결과가 75개 안에 들 크기. |
| 추가 검색 | 20번까지 (`MAX_EXTRA_SEARCHES`) | 200회차까지 따라가요. 3초 간격이라 한 검색이 1분 안에 끝나요. |
| 요청 간격 | 3초 (`REQUEST_SPACING`) | nyaa에 대한 요청 사이의 최소 시간. 한 호스트에 대해 웹 프로세스들과 검색들이 함께 지켜요. |
| 요청 하나의 최대 대기 | 60초 (`MAX_WAIT`) | 검색이 순서를 기다리는 시간의 한계. 같은 프로세스의 검색 6개가 줄을 서는 시간(6×3초)보다 훨씬 길어요. 그보다 멀면 검색이 곧바로 실패하고 다시 시도할 때를 알려요. |
| 검색이 읽는 이력 | 채널의 받음·중복 기록 20000개, 제목 20000개 (`MAX_SETTLED`, `MAX_TITLES`) | 최근 것부터 읽고, 더 길면 결과의 안내에 알려요. 수정본을 CRC32로 가리는 데 쓰는 제목은 CRC32가 적힌 것만 남겨요. |
| 범위의 폭 | 2000회차 (`MAX_SPAN`), 결과 2000개 (`MAX_RESULTS`) | 메모리와 미리보기의 크기를 묶어 둬요. |
| CRC 읽기 | 검색마다 영상 20개 (`MAX_CRC_READS`) | `버전 미상`을 가리려고 기존 영상을 읽는 일이 한 검색에서 이어지지 않게 해요. |
| 작품 폴더 읽기 | 파일 5000개 (`MAX_FILES`) | 폴더 하나에서 앞의 5000개만 보고, 그 뒤의 파일은 없는 것으로 판단해요. |

### 결정

- 대체 후보(이미 있는 회차의 더 높은 수정본)와 `버전 미상`은 보여 주되 기본 선택하지 않아요. 명세가 "폴더·Transmission에 없는 개별 회차만 기본 선택"이라고 하므로 가장 보수적으로 읽었어요.
- 일치 문구를 쓸 수 없는 규칙(정규식, 제목을 기다리는 규칙)은 검색어를 빈 칸으로 시작하고 까닭을 문장으로 알려요. 형식이 없는 채널은 일치 문구로 채워요.
- 웹은 Transmission에 닿지 않으므로, 다른 릴리스로 이미 받았는지를 규칙·채널 이력과 작품 폴더의 실제 파일로 판단하고 Transmission에 지금 있는지는 worker가 남긴 토렌트 목록으로 판단해요. 받는 중이라 파일이 아직 없는 회차는 이력과 그 목록이 알려 줘요. 목록은 한 주기까지 낡을 수 있어서, 목록 뒤에 받은 항목과 해시를 모르는 항목은 토렌트가 있는 것으로 보고, worker가 받기 직전에 Transmission에 직접 다시 물어요.
- 간격은 검색 요청에만 적용해요. 검색 요청은 웹만 보내므로 worker는 `search_pace`를 쓰지 않고, worker 주기의 피드 읽기는 바꾸지 않았어요.
- 받기에 실패한 검색 항목은 `추가 실패` 이력으로 남아 `다시 받기`로 다시 받을 수 있어요. 받기를 고른 항목만 이력에 남기므로 검색을 끝내거나 떠나도 이력에는 받은 항목만 있어요.
- `receive_past`는 규칙의 회차 변환을 새로 정하지 않아요. 사람이 범위를 현재 변환으로 확인했기 때문이에요.
- 검색으로 받은 토렌트도 피드에 없는 토렌트로 보아 기존의 사라진 토렌트 정리 대상이에요(피드에 없는 다른 항목과 같아요).
- 4자리 회차의 파일 이름은 `trname`의 기존 한계를 그대로 따라요.
- 마이그레이션은 26번(`search_pace`)과 27번(`transmission_listing`, `transmission_torrents`)이에요.

### 검증한 것

`tests/past_search.rs`(웹 API를 실제로 띄우고, 가짜 nyaa·가짜 Transmission·임시 폴더·실제 worker 명령 실행을 써요) 17개와 `src/past_search/` 단위 시험, `src/store/search_pace/` 시험이에요. 가짜 nyaa는 검색어를 단어로 맞추고 75개에서 자르고 요청 순서·시각을 기록해요.

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
- 추가 요청에 응답이 없었던 `받기`가 다음 시작에서 추가 전에 끝나면(규칙을 멈춤, 수집 폴더를 뺌, 폴더에 이미 그 수정본이 있음 등) `receive_once`와 같게 응답 없는 추가를 명령에 남기고 명령 라벨을 떼요. `a_paused_rule_after_an_unanswered_add_ends_the_command_and_holds_the_next_cleanup`은 고치기 전 라벨이 남고, 고치기 전 다음 주기의 `commands_unconfirmed`가 0이라 피드에 없는 그 토렌트를 지울 수 있었어요(검사를 하나씩 빼서 둘 다 실패하는 것을 봤어요).
- 0025 수정본 대체의 보완과 합친 뒤 다시 본 것: 검색으로 받은 수정본도 피드의 수정본과 같은 대체 상태 기계를 지나므로, 한 회차의 대체는 한 번에 하나이고 낮은 수정본이 높은 수정본을 대체하지 않아요. 같은 회차의 `v2`·`v3`를 검색으로 함께 받고 `v2`가 먼저 끝난 경우(`two_searched_revisions_of_an_episode_leave_the_higher_one`)와 피드의 `v2`가 받는 중일 때 검색으로 `v3`를 받은 경우(`a_searched_revision_higher_than_the_feeds_open_one_is_the_one_that_replaces`)는 그 보완 전 코드에서 `v2`가 회차 이름을 차지해 실패했고, 보완 뒤 `v3`만 대체해요. 이미 더 높은 수정본이 대체한 회차의 낮은 수정본은 검색으로 골라도 추가하지 않고 `중복`으로 끝나요(`a_searched_revision_lower_than_the_one_that_replaced_the_video_is_not_added`, 피드 주기와 같은 판단을 써요).
- 5행의 간격 검사는 간격을 0으로 바꿨을 때 실패하는 것을 봤어요. 간격 시험은 처음에 60 ms 간격에 여유 5 ms로 두어, 병렬 실행에서 한 번 54.9 ms로 실패했어요(도착 시각을 재므로 스케줄링 흔들림이 들어가요). 150 ms에 여유 60 ms로 고쳐 다섯 번 연속 통과했어요.
- 지워진 항목의 재선택(고치기 전에는 실패하는 것을 봤어요. 판정에서 사라진 토렌트를 모두 있는 것으로 바꿨을 때 웹 쪽 시험이, worker의 확인을 건너뛰었을 때 `receive_past` 시험이, 웹 명령 접수의 허용을 막았을 때 접수가 실패했어요): 영상을 지우고 토렌트를 뺀 5화가 선택 가능·기본 선택으로 보이고 다시 받아져요(`an_episode_whose_video_and_torrent_are_gone_is_missing_and_received_again`), 영상이 폴더에 있으면 `있음`이고 접수도 거절돼요(`an_episode_whose_video_is_in_the_folder_is_had_though_its_torrent_is_gone`), 토렌트가 Transmission에 있으면 영상이 놓이기 전이어도 선택할 수 없어요(`an_episode_whose_torrent_is_in_transmission_cannot_be_chosen_before_its_video_is_placed`). 낡은 목록으로 내놓은 항목은 받기 직전에 토렌트나 영상이 돌아와 있으면 추가하지 않아요(`a_result_offered_from_an_older_look_is_not_added_when_its_torrent_is_back`, `..._when_its_video_is_back`). 지워진 것으로 보지 않는 경우: 폴더가 없음(`a_work_folder_that_is_not_there_does_not_make_a_received_episode_gone`), 검색 뒤 폴더가 사라짐(`a_folder_that_vanishes_after_the_search_stops_the_worker_adding_the_result`), 항목이 5001개(`a_folder_with_more_entries_than_are_looked_at_holds_a_received_episode`), 배치(`a_batch_whose_torrent_is_removed_stays_held`), 검색 뒤 규칙을 멈춤(`a_rule_that_is_paused_after_the_search_does_not_receive_a_gone_result`), 같은 회차의 다른 토렌트(`another_torrent_of_the_rule_for_the_episode_stops_the_worker_adding_a_gone_result`), 더 높은 수정본이 대체한 낮은 수정본(`a_lower_revision_that_a_higher_one_replaced_is_not_received_again_when_the_folder_is_empty`). 이 시험들은 해당 판단을 하나씩 빼서 실패하는 것을 봤어요. 판정은 `src/past_search/world.rs`의 단위 시험(폴더를 끝까지 읽지 못함, 배치, 같은 회차의 다른 토렌트 포함), 목록의 저장은 `src/store/status/tests.rs`와 마이그레이션 시험, worker 주기가 목록을 남기는 것은 `tests/status_snapshots_from_cycle.rs`가 봐요. 이 통합 시험은 목록을 `StatusStore`에 직접 써요(주기를 돌리면 피드의 다른 회차도 추가돼서 시험 대상이 흐려져요). 합친 `cargo test --no-fail-fast`는 1280개 통과·실패 0개·무시 2개(27개 실행 파일), `cargo fmt --check`와 `cargo clippy --all-targets -- -D warnings`가 통과했어요. 웹(`web/`)은 바꾸지 않았어요.
- 브라우저(로컬 `trss-web`, 가짜 nyaa, 임시 DB·폴더): 범위 입력 → 검색 → 미리보기(1–24, 결과 27개, 22개 기본 선택, 폴더에 있는 3·4화는 `이미 있어요`로 해제) → 2개를 골라 `선택한 2개 받기` → 세 번째 단계에서 `추가하는 중`(worker가 없어 명령이 열려 있는 상태)까지 봤어요. 375 px에서 세 단계와 미리보기 모두 `scrollWidth`가 `clientWidth`와 같았어요(가로 스크롤 없음, 가로로 넘치는 요소 없음).
- `cargo fmt --check` 통과, `cargo clippy --all-targets -- -D warnings` 통과, `cargo test --no-fail-fast`는 1193개 통과·실패 0개·무시 2개(27개 실행 파일), 웹은 `bun install --frozen-lockfile`·`bun run typecheck`·`bun run build` 통과.
- 실제 nyaa는 검색 RSS를 한 번만 읽어 응답 모양을 확인했어요.
- 검토 뒤 고친 것의 시험(고치기 전에는 실패하는 것을 봤어요): 한 시간 막힌 호스트의 검색이 바로 실패함(`a_host_blocked_for_an_hour_fails_the_search_at_once`), 기다리는 동안 생긴 막힘에는 요청을 보내지 않음(`a_block_that_comes_while_a_request_waits_for_its_slot_stops_the_request`), 한계를 넘는 자리는 받지 않음(`a_slot_further_away_than_the_longest_wait_is_not_taken`), 보관 시간이 지난 검색은 받을 수 없고 사라질 때 중단됨(`a_search_past_its_keep_cannot_be_received_even_before_anything_sweeps`, `a_search_dropped_for_its_age_is_aborted_as_by_every_other_removal`), CRC32가 없는 제목은 수정본 판정에 올리지 않음(`of_the_channels_titles_only_those_a_crc_can_be_matched_to_are_kept`). 합친 뒤 `cargo test --no-fail-fast`는 1228개 통과·실패 0개·무시 2개(27개 실행 파일), `cargo fmt --check`와 `cargo clippy --all-targets -- -D warnings`, 웹의 `bun run typecheck`·`bun run build`가 통과했어요.

### 검증하지 못한 것

- 실제 Transmission으로 추가·이름 변경·완료 확인(가짜 Transmission만 썼어요).
- 실제 nyaa의 `429`와 여러 번의 묶음 검색(가짜 nyaa로만 봤어요).
- 브라우저에서는 worker가 없어 받기가 끝나는 모습(`추가됨`, `받기 실패`와 `다시 받기`)을 보지 못했어요. 그 흐름은 `tests/past_search.rs`가 웹 API와 worker 명령 실행으로 확인해요.
- 데스크톱 너비는 900 px 부근에서만 보았고 태블릿 너비는 보지 않았어요.
- 이력이 20000개를 넘을 때 안내가 붙는 것, 판정을 `spawn_blocking`으로 옮겨 런타임 스레드를 막지 않는 것은 시험으로 확인하지 못했어요(긴 이력을 만들어 보지 않았어요).
- 웹의 두 고침(화면을 떠나도 고른 항목을 끝까지 보냄, 받기로 넘긴 검색만 남기고 나머지는 떠날 때 끝냄)은 웹에 시험 틀이 없어 타입 검사와 빌드로만 확인했고 브라우저에서 눌러 보지는 않았어요.

### 남은 일

- 마스크된 링크(다른 호스트의 비밀 값)를 가진 검색 결과의 원래 링크 복원은 nyaa 링크로만 확인했어요.
- 알려진 한계(코드는 그대로 두었어요):
  - 링크에 가려진 값이 있는 검색 결과는 그 값을 채널에 저장된 비밀 값으로 채울 수 있을 때만 받을 수 있어요(`fill_masked_values`, `src/worker/commands/link.rs`). 채울 수 없으면 받기가 실패해요.
  - 작품 폴더는 앞의 5000개 항목만 읽어서(`MAX_FILES`), 그 뒤에 있는 회차는 없는 것으로 판단하고 기본 선택될 수 있어요.
  - 받음으로 남은 항목은 영상이 폴더에 없고 토렌트도 Transmission에서 사라졌을 때만 다시 고를 수 있어요. 토렌트가 아직 Transmission에 있으면(영상이 놓이기 전이거나 파일만 지운 경우) 선택할 수 없고, 영상이 폴더에 있으면 토렌트를 지웠더라도 `있음`이에요. 목록이 아직 없거나 항목의 해시를 모르는 옛 기록은 토렌트가 있는 것으로 봐요.
  - 작업 폴더 전체를 지웠거나 폴더가 없으면 받은 회차를 다시 받을 수 없어요. 영상이 없는 것인지 볼륨이 빠진 것인지 알 수 없어서 `이미 받은 항목`으로 둬요. 빈 폴더를 만들어 두면 지워진 회차를 다시 고를 수 있어요. 항목이 5000개를 넘는 폴더도 같아요.
  - 다시 받은 토렌트의 해시가 이력에 적힌 해시와 다르면(같은 링크는 같은 해시예요) 이력은 옛 해시를 그대로 가져요. 영상이 놓이기 전까지만 다시 고를 수 있는 것으로 보일 수 있어요.
  - 채널의 이력이 20000개를 넘으면 오래된 기록은 판정에 쓰이지 않아요(결과의 안내에 알려요).
