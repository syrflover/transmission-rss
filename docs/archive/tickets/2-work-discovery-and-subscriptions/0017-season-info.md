# 0017 시즌마다 AniList 항목을 이어 시즌 정보를 보여줘요

- 상태: 완료 (실제 AniList에 묻는 확인은 남았어요)
- 출처: [시즌 정보](../../../specs/library.md#시즌-정보), [머리와 시즌](../../../specs/library.md#머리와-시즌), [오른쪽 카드 열](../../../specs/library.md#오른쪽-카드-열), [라이브러리 화면](../../../specs/library.md#라이브러리-화면)
- 막는 티켓: [0014](0014-work-detail-video.md), [0015](0015-work-artwork.md)(AniList 조회와 요청 간격을 함께 써요)

## 작업

로컬 시즌마다 AniList 항목을 하나 이상 이어, 작품 상세의 시즌 정보 칸·AniList 링크·`작품 정보`의 줄거리와 라이브러리의 방영연도순·방영 중 필터·원제 검색을 채워요.
[0013](0013-library-screen.md)·[0014](0014-work-detail-video.md)가 시즌 정보가 붙을 때까지 비워 둔 자리들이에요.

- 연결 규칙(가장 앞 시즌의 자동 연결, 다음 시즌의 속편 제안과 확인, 분할 방영의 여러 항목, 줄거리, 갱신)은 명세 [시즌 정보](../../../specs/library.md#시즌-정보) 그대로예요.
- AniList 조회·요청 간격·`429` 대기는 [0015](0015-work-artwork.md)의 클라이언트와 `anilist_pace`를 함께 써요. 자동 연결과 정해진 갱신은 worker의 대기열이 하고, 사용자 동작(검색·고르기·제안 확인·연결 풀기·다시 받기)은 웹이 바로 해요. 표지처럼 연결마다 버전을 두어 늦게 끝난 자동 결과가 사용자의 선택을 되돌리지 않게 해요.
- 작품 상세: 시즌 정보 칸(방영·분량·제작사·장르, 넓으면 네 칸, 좁으면 두 칸, 아이콘에는 글자)과 제목 줄의 `AniList` 링크(선택 시즌의 첫 항목, 눌러도 시즌이 바뀌지 않음), 연결 바꾸기, 속편 제안(`이 항목이 맞아요`·`다른 항목 고르기`), 항목 더하기·빼기·순서. 오른쪽 카드 열에 `작품 정보`(선택 시즌의 줄거리)를 더해요. 회차 줄의 방영일은 AniList의 회차별 방영 일정이 있는 방영 중 항목에서만 채우고, 없으면 비워 둬요.
- 머리의 원제는 가장 앞 시즌 첫 항목의 원제(native)예요. 한국어 작품명은 Anissia 연결 전까지 폴더 이름이에요.
- 라이브러리: 서버 정렬·필터·검색([0013](0013-library-screen.md))에 방영연도·방영 중·원제·영문명·로마자 제목을 더해요.
- 시즌 정보 칸의 미상 처리와 표지와의 분리는 명세대로예요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 작품 폴더 `Lycoris Recoil`(시즌 1), AniList 검색에 정확히 같은 제목 하나, 결과 끝까지 읽음 | 시즌 1에 그 항목이 `자동`으로 이어지고 방영·분량·제작사·장르가 채워져요. |
| 같은 제목 후보가 둘 | 잇지 않고 시즌 정보는 `미상`이며, 사용자가 검색해 고를 수 있어요. |
| 시즌 1이 이어진 작품의 시즌 2 | 시즌 1 마지막 항목의 속편이 제안으로 보이고, 확인하기 전에는 시즌 정보가 `미상`이에요. 확인하면 이어져요. 할 일 수는 늘지 않아요. |
| 시즌 1에 Part 1(12화, 4월–6월)과 Part 2(13화, 7월–9월)를 차례로 이음 | 분량 `25화`, 방영 4월 시작–9월 끝, 제작사·장르는 겹치지 않게 합쳐져요. 한 항목의 회차 수가 미상이면 분량이 `미상`이에요. |
| 시즌 2를 고른 뒤 `AniList` 링크를 누름 | 시즌 2 첫 항목의 AniList 페이지가 열리고 선택 시즌은 그대로예요. 시즌 1·작품 표지의 AniList ID가 달라도 각각 맞는 곳으로 가요. |
| 시즌을 바꿈 | `작품 줄거리`가 그 시즌 첫 항목의 설명으로 바뀌어요. 여러 문단·`<br>`·`<i>`·문자 참조가 섞인 설명의 문단과 글자가 남고, `<script>`나 SVG가 실행되지 않아요. |
| 사용자가 고른 직후 늦게 끝난 자동 연결 | 사용자의 선택이 남아요. 오래된 버전으로 보낸 변경은 `409`와 지금 상태를 돌려줘요. |
| 방영 중 항목과 끝난 항목이 섞인 라이브러리 | `방영 중` 필터에 최신 시즌이 방영 중인 작품만 나오고, `방영연도순`은 최신 시즌 첫 항목의 시작 연도 내림차순(미상은 뒤)이에요. 원제·영문명으로 검색돼요. |
| 하루가 지남(시계 경계) | 방영 전·방영 중 항목만 다시 받고 끝난 항목은 받지 않아요. 요청은 표지와 같은 간격을 지켜요. |
| 연결이 없는 시즌 | 시즌 정보 칸이 `미상`이고, 다른 시즌의 값이나 파일 상태로 채우지 않아요. |
| 휴대폰 너비 | 시즌 정보가 두 칸으로 보이고 가로 스크롤이 없어요. |

## 결과

### 구현한 것

- 저장(`crates/trss-library/src/store/seasons/`, 마이그레이션 13): AniList 항목(`anilist_entries`: 제목 셋, 형식, 상태, 회차 수, 시작·종료일, 제작사·장르·방영 일정·속편, 설명, 받은 때), 시즌마다의 연결 상태(`season_info`: 버전, `auto`/`user`, 검색 작업, 메모), 순서 있는 연결(`season_entries`)이에요. 연결은 작품 ID에 묶여서 시즌 폴더가 사라졌다 돌아와도 남아요. 감시 폴더를 등록 해제해도 작품을 떼어 둘 뿐이라([0012](0012-watch-folders-discovery.md)) 연결이 남고, 떼어 둔 작품의 시즌은 화면·검색·하루 갱신에서 빠졌다가 같은 경로를 다시 등록하면 그대로 돌아와요.
- 서비스와 대기열(`crates/trss-library/src/seasons/`, `trss-worker`): 첫 시즌의 검색 작업은 표지와 같은 `title.rs` 판정(정규화한 정확히 같은 제목 하나, 검색을 끝까지 읽음)으로 항목을 `자동`으로 이어요. 방영 전·방영 중 항목은 하루가 지나면 다시 받고, 끝난 항목은 `정보 다시 받기`로만 받아요. 요청은 표지와 같은 `anilist_pace`(2초 간격, `429` 대기)를 쓰고, 대기열은 따로 잠금(`<DB>.seasons.lock`)을 잡아요. 연결마다 버전이 있어 오래된 버전은 `409`와 지금 상태를 받고, 늦게 끝난 자동 결과는 버려져요. 검색 결과(항목 저장, 자동 연결, 메모)를 DB에 적지 못하면 AniList 실패와 같이 다음 재시도 간격만큼 미루고, 갱신한 항목을 적지 못하면 1시간 뒤로 미뤄요. 미루는 기록마저 실패하면 대기열이 한 번 쉬어요. 같은 일을 곧바로 다시 잡아 AniList에 거듭 묻지 않아요.
- 웹 API(`crates/trss-web/src/seasons_api.rs`): `info`·`search`·`links`·`auto`·`refresh`와 작품 상세의 시즌별 `info`·작품의 `native_title`·회차의 `air_at`. 줄거리는 서버가 글자만 남겨 문단 배열로 보내요(`crates/trss-library/src/seasons/describe.rs`: `<br>`는 줄바꿈, `<p>`는 문단, 나머지 태그는 지우고 안의 글자는 남기며, 문자 참조는 한 번만 풀어요).
- 합쳐 보이는 값: 방영은 첫 항목의 시작부터 마지막 항목의 끝까지(방영 중 항목이 있으면 `방영 중`), 분량은 합(미상이 하나면 미상), 제작사는 애니메이션 제작사로 표시된 주 제작사, 제작사·장르는 순서를 지킨 합집합이에요. 머리의 원제는 가장 앞 시즌 첫 항목의 원제예요.
- 라이브러리: `방영연도순`(최신 로컬 시즌의 첫 항목 시작 연도 내림차순, 미상은 뒤), `방영 중`(최신 로컬 시즌에 `RELEASING` 항목), 검색(연결한 항목의 원제·영문명·로마자 제목도 찾아요).
- 화면: 시즌 정보 칸(방영·분량·제작사·장르, 컨테이너 너비에서 네 칸/두 칸), 제목 줄의 `자동` 표시·`AniList` 링크·`연결 바꾸기`, 속편 제안(`이 항목이 맞아요`·`다른 항목 고르기`), 연결 대화상자(검색, 더하기, 빼기, 순서, 연결 끊기, `자동으로 다시 찾기`, `정보 다시 받기`), `작품 정보` 카드의 줄거리(글자 노드로만), 머리의 원제, 회차 줄의 방영일.
- 회차 줄의 방영일은 구현했어요. 방영 중 항목의 `airingSchedule`에서만 채우고, 앞 항목의 알려진 회차 수만큼 밀어서 로컬 회차에 맞춰요(그 앞 항목의 회차 수가 미상이면 비워 둬요).

### 결정

- 마이그레이션은 이미 있는 작품·시즌에 검색을 만들지 않아요(조정자 결정). 검색 작업은 새로 기록되는 첫 시즌 줄에 대한 트리거가 만들어서, 이 버전 전에 등록한 작품은 `연결 바꾸기`로 직접 이어야 해요. 표지도 이제 같은 규칙이에요([0015](0015-work-artwork.md)).
- 가장 앞 시즌은 번호가 1 이상인 시즌 가운데 가장 작은 것이고, 시즌 0(스페셜)은 아니에요. 더 작은 시즌이 나중에 생기면 새 첫 시즌이 검색을 받고 이전 첫 시즌의 아직 안 한 검색은 버려요(이미 이어진 연결은 그대로).
- 속편 제안은 바로 앞 번호(N-1) 시즌이 로컬에 있을 때만, 그 시즌의 마지막 항목의 `SEQUEL`(형식·날짜 포함 전부)을 보여요. 시즌 2가 없고 3만 있는 작품은 제안이 없어요.
- 작품이 합쳐지면(보관 이동에서 같은 이름이 있는 경우) 남는 작품의 같은 시즌에 연결이 없을 때 옮긴 작품의 연결(항목과 그 순서, `auto`/`user`)을 가져와요. 그 시즌의 메모와 아직 안 한 검색은 비우고, 버전은 두 작품의 것보다 올라가요. 남는 작품에 이미 연결이 있으면 그것이 남고, 옮긴 작품의 나머지는 작품 줄과 함께 사라져요. 표지와 같은 규칙이에요.
- 한 시즌에 이을 수 있는 항목은 8개까지예요. 작품 하나를 연결하는 데 첫 시즌 자동 연결은 검색 요청 1–4개와 항목 요청 1개, 사용자가 이은 항목은 항목마다 요청 1개(이미 저장돼 있으면 없어요), 하루 갱신은 방영 전·방영 중 항목마다 요청 1개예요.
- 설명의 HTML 문자 참조는 자주 쓰는 이름 표(손으로 쓴 것)와 숫자 참조만 풀어요. 표에 없는 이름은 쓰인 그대로 남겨요.
- 설명의 "태그는 글자로 다룬다"를 "태그 표기는 지우고 안의 글자는 남긴다"로 읽었어요. `<i>`·`<b>` 같은 흔한 태그가 `<i>`라는 글자로 보이면 줄거리가 읽기 어렵기 때문이에요. 태그 모양을 그대로 보여주는 쪽을 원하면 `describe.rs`의 한 곳만 바꾸면 돼요.
- `방영 중` 상태는 항목 가운데 하나라도 `RELEASING`이면, 아니면 마지막 항목의 상태예요. `방영연도순`과 `방영 중` 필터는 최신 로컬 시즌만 봐서, 최신 시즌을 아직 잇지 않은 작품은 앞 시즌이 이어져 있어도 연도는 미상이고 방영 중이 아니에요(명세대로예요).

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(702개 통과)와 웹 `npm run typecheck`, `npm run build`가 통과했어요.
- 시험은 가짜 AniList(`crates/trss-anilist/src/fake.rs`)와 손으로 옮기는 시계(`AtomicI64`)로만 돌아 실제 네트워크에 나가지 않아요. 완료 기준의 행과 시험:
  - 유일한 제목 `Lycoris Recoil`: `one_exact_title_links_the_first_season_as_auto_with_its_values`, `the_title_is_compared_the_way_the_cover_does`(표지와 같은 판정).
  - 같은 제목 후보가 둘 / 끝까지 읽지 못함: `a_second_candidate_with_the_same_title_links_nothing_and_a_note_says_why`, `a_search_that_was_not_read_to_its_end_links_nothing`.
  - 다음 시즌의 속편 제안과 확인: `the_next_season_is_offered_the_sequels_and_linked_only_when_the_user_confirms`, `the_next_season_shows_the_sequels_and_links_nothing_until_one_is_confirmed`(API; 제안은 N-1 시즌이 있을 때만, 확인 전에는 `미상`), `only_a_season_without_entries_gets_the_previous_seasons_sequels`.
  - 분할 방영의 합: `two_parts_in_order_are_one_season`, `two_parts_are_one_season`(분량 합, 시작–끝, 제작사·장르 합집합, 회차 수 미상이면 미상).
  - 시즌마다 다른 AniList 링크: `the_ani_list_link_of_each_season_is_its_own_first_entrys_page`.
  - 줄거리: `the_synopsis_is_the_first_entrys_plain_text_in_paragraphs`, `a_long_mixed_description_keeps_its_paragraphs_and_words`, `character_references_are_decoded_once`, `something_that_only_looks_like_a_tag_stays_as_written`, `other_tags_go_and_their_text_stays`, `controls_are_dropped_and_text_in_any_script_stays`(`<script>`·`<svg onload>`·`<img onerror>`·닫히지 않은 태그·`&amp;lt;`를 넣은 입력).
  - 늦은 자동 결과와 버전: `an_automatic_result_that_finishes_after_the_user_chose_changes_nothing`, `a_late_automatic_result_is_dropped_after_a_user_change`, `a_stale_version_changes_nothing_and_a_missing_entry_links_nothing`, `changing_the_links_answers_the_new_info_and_an_old_version_is_a_409_with_the_current_one`(API `409`과 `current`).
  - 라이브러리: `the_library_sorts_by_airing_year_filters_what_is_airing_and_searches_linked_titles`, `the_latest_local_season_decides_the_year_and_airing_not_an_earlier_one`.
  - 하루 갱신과 사용자 갱신: `entries_that_are_not_finished_are_received_again_a_day_later_and_finished_ones_are_not`, `only_linked_entries_that_are_not_finished_are_due_a_day_after_they_were_received`, `the_user_can_receive_a_finished_entry_again`, `refresh_requests_keep_the_pace_the_covers_use`, `a_429_holds_the_refresh_for_as_long_as_it_says_and_a_failure_waits_an_hour`, `an_entry_that_is_gone_keeps_what_was_stored_and_is_not_asked_for_every_cycle`.
  - 연결 없음: `a_season_without_a_link_is_unknown_and_never_filled_from_another_season`.
  - 마이그레이션과 트리거: `the_migration_creates_no_search_for_what_exists_already`, `a_newly_recorded_first_season_gets_one_search_and_nothing_else_does`, `a_lower_season_that_appears_later_takes_the_search_and_the_higher_ones_is_dropped`, `a_season_folder_that_goes_and_comes_back_keeps_its_link_and_gets_no_new_search`.
  - 방영일: `episode_air_dates_come_only_from_a_releasing_entry_with_a_schedule`, `air_times_come_only_from_a_releasing_entry_after_known_counts`.
  - 리뷰 뒤 더한 것(고치기 전 코드에서 실패를 확인했어요): `a_search_whose_outcome_cannot_be_written_waits_like_a_failure_instead_of_asking_again_at_once`, `a_refresh_that_cannot_be_written_waits_an_hour`(트리거로 기록을 실패시킴), `a_merge_carries_the_moved_works_choices_where_the_kept_work_has_none`(`crates/trss-library/src/store/library/tests.rs`). 등록 해제 뒤 연결이 남는 것은 `an_entry_round_trips_and_an_unregistered_work_keeps_its_links`와 `unregistering_takes_the_works_out_and_the_same_path_brings_them_back_as_they_were`(`crates/trss-worker/tests/library_watch.rs`)가 봐요.
- 브라우저(로컬 `trss-web`·`trss-worker`, 스크래치 DB, 가짜 AniList(`TRSS_ANILIST_URL`·`TRSS_ANILIST_IMAGE_ORIGINS`), 작품 8개, 1280px와 390px, 2026-10-01):
  - worker가 시즌 1을 검색해 `Lycoris Recoil`·`Sousou no Frieren`·`Oshi no Ko`·`Bocchi the Rock!`·`Dandadan`·`Kusuriya no Hitorigoto`는 `자동`으로 이었고, `Clevatess`(같은 제목 둘)는 `모호함`, `Spy x Family`(같은 제목 없음)는 `일치 없음` 메모로 남았어요.
  - `Dandadan`(방영 중): 방영 `2024년 10월 4일 ~`, 분량 12화, 제작사 `Science SARU`, 장르 셋, `방영 중` 표시, 회차 줄에 `9월 30일 (수)` 같은 방영일이 AniList 일정에서만 채워졌어요.
  - `Bocchi the Rock!`: 줄거리가 세 문단으로 나뉘고 `&amp;`·`&#8212;`·`&#x266a;`가 풀렸으며, `<script>`·`<svg onload>`·`<img onerror>`가 DOM에 생기지 않았어요(`document.querySelectorAll`로 확인). 닫힌 태그의 글자(`alert('x')`)는 글자로만 보여요.
  - `Sousou no Frieren` 시즌 2: 시즌 1 마지막 항목의 속편 둘(TV, OVA)이 제안으로 보이고 시즌 정보는 `미상`이었어요. `이 항목이 맞아요`로 이으니 값이 채워지고 `자동` 표시는 없고 줄거리와 회차 줄의 방영일이 바뀌었어요(처음에는 방영일이 안 바뀌어서 이을 때 작품을 다시 읽도록 고쳤어요).
  - 연결 대화상자: 다른 곳(curl)에서 먼저 바꾼 뒤 저장하면 `다른 곳에서 먼저 시즌 정보를 바꿨어요. …`가 보이고 목록이 지금 상태로 바뀌었어요. OVA를 더해 위로 올려 저장하니 분량 `11화`, 방영 시작이 OVA의 날짜로, 제작사는 합집합, 회차 줄의 방영일은 앞 항목의 회차 수만큼 밀렸어요.
  - `Oshi no Ko` 시즌 2에 Part 1·Part 2(회차 수 미상)를 이으니 분량이 미상이고, 시작–끝은 7월–12월, 제작사에는 애니메이션 제작사가 아닌 항목이 빠졌어요.
  - `Lycoris Recoil`의 `자동으로 다시 찾기`: 연결이 끊기고 검색 중 표시 뒤에 worker가 같은 항목을 다시 이었어요.
  - 라이브러리: 방영연도순, `방영 중` 필터, 원제(`リコリス`)·영문명 검색이 API와 화면에서 맞았어요.
  - 390px: 시즌 정보가 두 칸이고, 연결 대화상자는 양옆 16px이며 안에서만 세로로 스크롤하고, `scrollWidth == innerWidth`였어요. `작품 정보`는 접힌 카드로 목록 아래에 있어요.

### 검증하지 못한 것

- 실제 AniList에 묻지 않았어요. 실제 응답 모양(`relations`·`airingSchedule`·`studios(isMain)`·`description(asHtml:false)`의 실제 내용), 제목 표기, `429` 동작은 공개 문서와 가짜 서버로만 맞췄어요.
- 하루 갱신은 손으로 옮기는 시계 시험으로만 봤어요. 브라우저에서는 하루를 기다리지 않았어요.
- 브라우저 확인은 창이 가려진 채라 `:focus` 표시와 대화상자 움직임을 눈으로 보지 못했어요. 다크 모드, 실제 휴대폰·스크린 리더, 721–1099px 너비는 보지 않았어요.
- 자동 화면 시험은 없어요(이 저장소에 웹 시험 도구가 없어요).
- 이 버전 전에 등록한 작품에 대한 이어짐은 없어요(위 `결정`).

### 남은 일

- 첫 배포 뒤 실제 AniList로 작품 몇 개의 자동 연결·속편 제안·줄거리를 확인해요.
- Anissia 연결이 생기면 첫 시즌 자동 연결의 판정에 제목·시작일 힌트를 더할 수 있어요(명세의 자동 판정). 지금은 폴더 이름뿐이에요.
- 이 버전 전에 등록한 작품의 시즌 정보를 한 번에 채우는 일(조정자 결정으로 마이그레이션은 하지 않아요)이 필요하면 사용자가 요청할 때 시작하는 동작을 따로 정해요.
- 시즌 정보의 YAML 내보내기 필드는 [내보내기](../../../specs/settings.md#내보내기)를 구현할 때 정해요(명세).

