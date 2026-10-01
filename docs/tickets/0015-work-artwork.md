# 0015 작품 표지를 AniList에서 고르거나 올려요

- 상태: 완료 (명세 표의 가져오기 행은 결과 목표 5의 몫이고, 실제 AniList에 묻는 확인은 남았어요)
- 출처: [작품 표지](../specs/library.md#작품-표지), [작품 표지 필드](../specs/settings.md#작품-표지-필드), [표지 ADR](../adr/0007-anilist-work-artwork.md), [앱 데이터 파일 ADR](../adr/0008-trss-folder-and-app-data-files.md)
- 막는 티켓: [0012](0012-watch-folders-discovery.md)

## 작업

작품마다 표지 하나를 AniList 검색 결과에서 고르거나 이미지 파일을 올려 정하고, 명확한 경우에만 앱이 자동으로 정해요.
라이브러리 격자·목록과 작품 상세 머리의 빈 표지 자리를 실제 이미지로 채워요.
명세의 자동 판정·이미지 파일 수명·선택 버전 조건이 모두 이 티켓의 범위예요.

- 선택 상태는 작품마다 `auto`·`manual`·`disabled`이고, 새로 발견한 작품은 `auto`예요. 사용자가 비우면 `disabled`예요.
- 자동 검색은 앱이 새 작품을 등록할 때와 사용자가 검색·복구를 요청할 때만 수신 작업을 만들어요. 감시 폴더 첫 등록처럼 작품이 많이 한꺼번에 생기면 AniList 요청 간격을 지켜 차례로 처리하고, 요청 한도에 걸리면 기다렸다 이어가요.
  제목 관찰값은 작품 폴더 이름이에요. Anissia 연결이 생기면 명세대로 힌트가 늘어요.
- 이미지는 앱 데이터 폴더(DB 옆)의 앱이 만든 경로에 두고, 준비·검증·공개·DB 반영 순서와 선택 버전 비교를 지켜요. 바이트·픽셀·시간 상한은 구현하며 정하고 이 티켓의 결과에 적어요.
- 표지 화면은 작품 상세의 표지를 눌러 크게 보는 자리에서 `AniList에서 고르기`·`파일 올리기`·`표지 비우기`·`자동으로 되돌리기`를 둬요.
- 앱 YAML 내보내기·가져오기의 표지 필드는 결과 목표 5에서 다뤄요.

## 완료 기준

명세 [작품 표지](../specs/library.md#작품-표지)의 입력·상태 표가 이 티켓의 완료 기준이에요. 그 표의 가져오기 행은 결과 목표 5에서 확인하고, 나머지는 격리된 입력(가짜 AniList 응답, 임시 앱 데이터 폴더)으로 확인해요.
그에 더해:

| 입력·상태 | 기대 결과 |
| --- | --- |
| 작품 폴더 `Lycoris Recoil`, AniList 검색 결과에 정규화 제목이 같은 후보 하나만 있고 결과 페이지가 끝남 | 표지가 자동으로 정해지고 이미지 바이트를 검증한 뒤에 라이브러리에 보여요. |
| 같은 제목의 후보가 둘(시즌 2 따로) | 비워 두고 작품 상세에서 고를 수 있어요. 작품·영상 표시는 그대로예요. |
| 감시 폴더 첫 등록으로 작품 약 520개가 생김 | AniList 요청이 간격을 지켜 차례로 나가고, 수집 주기와 웹 응답이 막히지 않아요. |
| 작품을 [0011](0011-archive-folder-move.md)로 보관 폴더에 옮김 | 표지가 그대로 보여요. |
| `.png` 이름의 JPEG, `.jpg` 이름의 텍스트, 상한을 넘는 이미지를 올림 | JPEG은 받아들이고, 나머지는 까닭과 함께 거부하며 기존 표지가 남아요. |

## 결과

### 구현한 것

- 저장(`src/store/artwork/`, 마이그레이션 11): 작품마다 `work_artwork` 한 줄(`mode` `auto`·`manual`·`disabled`, `source` `anilist`·`upload`, AniList ID, 이미지 참조(ID·경로·바이트 수·SHA-256·형식), 선택 버전, 대기 작업, 안내 코드)이에요. 명세의 불변 관계(`disabled`면 선택·이미지 없음, AniList 출처면 ID 있음, 이미지 출처는 선택 출처와 같음 등)는 `CHECK`로 막아요. 작품 줄이 생기면 트리거가 `auto`와 검색 작업을 함께 만들어서 0012의 발견 코드는 고치지 않았어요. 다시 읽기(재검사)는 줄을 새로 만들지 않으므로 검색도 다시 만들지 않아요. 작품 줄이 지워지면 선택도 지워지고(파일 기록은 정리에 맡겨요), 보관 이동(0011, 같은 ID)은 선택을 그대로 가져가요.
- 자동 판정(`src/artwork/title.rs`): 관찰값은 작품 폴더 이름이에요. NFC·대소문자 접기(`caseless`)·양끝 공백 제거·연속 공백 축약만 하고, AniList의 romaji·English·native 제목과 synonyms 가운데 하나가 같으면 후보예요. 검색을 마지막 페이지까지 읽었고 후보가 정확히 하나일 때만 골라요(같은 항목이 두 페이지에 나오면 하나로 셈). 둘 이상이면 `모호함`, 페이지가 남았거나 상한에 걸리면 `끝까지 확인하지 못함`, 없으면 `같은 제목 없음`으로 비워 둬요.
- AniList(`src/artwork/anilist.rs`): `https://graphql.anilist.co`에 묻고(`TRSS_ANILIST_URL`로 바꿀 수 있어요), 이미지는 확인된 AniList 응답의 `coverImage` 주소 가운데 허용한 출처(기본 `https://s4.anilist.co`, `TRSS_ANILIST_IMAGE_ORIGINS`)에서만, 리다이렉트 없이 받아요. 웹과 worker의 모든 요청은 DB의 `anilist_pace` 한 줄로 차례를 나눠 2초 간격을 지키고, `429`는 `Retry-After`만큼(없으면 60초, 최대 1시간) 모든 요청을 멈춰요.
- 대기열(`src/artwork/queue.rs`): worker가 수집 주기와 따로, 주기 잠금 밖에서 돌려요(자기 잠금 `<DB 경로>.artwork.lock`). 작업은 DB 줄이라 재시작하면 남은 것부터 이어요. 받기 작업이 검색보다 먼저이고, 그 안에서는 요청 순서예요. AniList에 닿지 못하면 1분·10분·60분 뒤 다시 하고 그다음은 `실패`로 두어 사용자를 기다려요. 10분마다 중단된 공개를 복구하고 정리해요.
- 이미지(`src/artwork/image.rs`, `files.rs`): 파일 이름·요청 MIME이 아니라 바이트의 시작(magic)으로 JPEG·PNG·WebP를 가리고 끝까지 디코딩해 확인해요(동시 디코딩은 프로세스마다 하나). 앱 데이터 폴더(DB가 있는 폴더)의 `artwork/.staging/<UUID>.tmp`에 새로 만들어(`create_new`, fsync) 검증하고, 파일 객체(장치·inode)를 기록한 뒤 `RENAME_NOREPLACE`로 `artwork/<UUID>.<확장자>`에 공개해요. 마지막으로 선택 버전을 확인하는 트랜잭션에서 선택을 바꿔요. 공개 전에 `artwork_files`에 앱이 만든 파일로 먼저 적어 두므로, 중단되면 다음 정리가 기록한 파일 객체가 같은 것만 마저 공개하거나 지워요.
- 제공(`GET /api/library/works/{id}/artwork/image`): 매번 앱 데이터 폴더에서 한 마디씩 `O_NOFOLLOW`로 열어 링크·폴더 밖 경로를 거부하고, 일반 파일·바이트 수·SHA-256·형식이 기록과 같을 때만 보내요(`ETag`=SHA-256, `Cache-Control: no-cache`, `nosniff`). 아니면 `404`이고 화면은 자리표시자예요. 상태 API는 `available`·`missing`·`mismatch`·`unverified`를 구분해요.
- 정리: `artwork_files`에서 앱이 공개한 파일 가운데, 모든 선택의 경로와 파일 객체(lstat·stat의 장치·inode) 어느 것에도 닿지 않는 것만 지워요. 참조 경로를 확인하지 못하거나(권한, 폴더 링크 등) 파일 객체가 기록과 다르면 아무것도 지우지 않아요. 정리와 선택 반영은 같은 `IMMEDIATE` 트랜잭션 잠금으로 겹치지 않아요.
- 사용자 동작(`src/web/artwork_api.rs`): `GET …/artwork`, `POST …/artwork/search`(`{q, page}`), `…/pick`(`{version, anilist_media_id}`), `…/upload?version=N`(본문이 이미지 바이트), `…/clear`·`…/auto`·`…/repair`(`{version}`). 모든 변경은 보던 선택 버전을 보내고, 버전이 바뀌었으면 `409`와 지금 상태를 돌려줘요. 목록·상세 API는 `cover_url`을 함께 줘요.
- 화면: 라이브러리 격자·목록과 작품 상세 머리가 표지를 보여주고, 이미지가 실패하면 머리글자 자리표시자가 남아요. 상세의 표지를 누르면 크게 보는 대화상자에 `AniList에서 고르기`(검색어는 폴더 이름, 후보마다 작은 표지·다른 제목·형식·연도·ID, `결과 더 보기`), `파일 올리기`, `표지 비우기`, `자동으로 되돌리기`가 있어요. 대기 중이면 3초마다 다시 읽어요.
- 운영 안내: `readme.md`에 `artwork/`·잠금 파일, 백업 대상, 바깥 연결(`graphql.anilist.co`, `s4.anilist.co`), 500개 기준 약 20분의 첫 검색 시간, 상한, 시험용 환경 변수를 적었어요.

상한(`src/artwork/mod.rs`, `anilist.rs`, `queue.rs`의 이름 있는 상수):

| 상수 | 값 | 까닭 |
| --- | --- | --- |
| `MAX_IMAGE_BYTES` | 10 MiB | 업로드와 원격 이미지 모두. AniList 표지는 수백 KB예요. |
| `MAX_IMAGE_SIDE` | 8192px | 한 변 |
| `MAX_IMAGE_PIXELS` | 1,200만 화소 | 128M 컨테이너에서 RGBA 디코딩이 약 48MB에 머물도록 |
| `DECODE_MAX_ALLOC` | 64 MiB | 디코더 할당 상한 |
| `FETCH_TIMEOUT` / `API_TIMEOUT` | 30초 / 20초 | 이미지 받기 / GraphQL 한 번 |
| `USER_MAX_WAIT` | 10초 | 화면 요청이 AniList 차례를 기다리는 최대 시간. 넘으면 `잠시 뒤 다시 해 주세요` |
| `REQUEST_SPACING` | 2초 | 분당 30회 |
| `SEARCH_PAGE_SIZE` × `MAX_SEARCH_PAGES` | 50 × 4 | 자동 검색이 읽는 최대 후보 수. 넘으면 `끝까지 확인하지 못함` |
| `STALE_STAGING` | 15분 | 이보다 오래된 준비 중 파일만 복구·정리 |
| `RETRY_DELAYS` | 1분·10분·60분 | 그 뒤 `실패` |

### 결정

명세가 정하지 않은 곳은 보수적으로 정했어요.

- 이 기능 전에 등록된 작품도 마이그레이션 때 `auto`와 검색 한 번을 받아요. 0012로 이미 등록한 작품이 표지를 하나씩 눌러야만 얻는 것을 피하려는 것으로, 명세의 "새 작품 등록"을 "앱이 처음 이 작품의 표지 상태를 기록할 때"로 읽었어요. 그 뒤로는 재검사·화면 열기·재시작이 검색을 만들지 않아요.
- 사용자 동작은 web이 직접 해요. 사용자가 결과(거부 까닭, 충돌)를 기다리고, 업로드 바이트가 web에 도착하며, worker 명령은 수집 주기 뒤에서 기다리기 때문이에요. 안전은 `RENAME_NOREPLACE`와 버전 확인 트랜잭션이 지키고, AniList 간격은 DB 한 줄로 두 프로세스가 나눠요.
- 선택 버전은 선택이 바뀔 때(사용자 동작, 자동 검색이 고름) 올라가고, 이미 고른 항목의 이미지가 도착하거나 작업을 바꿀 때는 올라가지 않아요. 그래서 이미지가 먼저 도착해도 그 전 화면에서 한 선택이 그대로 적용되고, 늦은 자동 결과는 버전·작업 확인에서 버려져요.
- 자동인데 표지가 없고 대기 작업도 없으면 대화상자에 `자동으로 다시 찾기`를 둬요(명세의 "사용자가 검색을 요청"). AniList 이미지가 없거나 바뀌었으면 `다시 받기`가 같은 ID만 다시 받아요. 직접 올린 파일이 없어지면 자동 후보로 바꾸지 않고 `파일 올리기`로 복구하라고 안내해요.
- AniList 결과의 작은 표지는 브라우저가 허용한 이미지 출처에서 바로 읽어요(앱이 받아 두지 않아요).
- 보관 이동에서 목적지에 같은 이름의 작품이 있어 합쳐지면(`Followed::Merged`) 옮긴 작품의 줄과 함께 그 표지 선택도 사라지고 남는 작품의 표지가 그대로예요.

### 검증한 것

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`(625개 통과)와 웹 `npm run typecheck`, `npm run build`가 통과했어요. 표지 시험은 열 번 거듭 돌려도 같았어요.
- 시험은 가짜 AniList(`src/artwork/fake.rs`, `127.0.0.1`의 GraphQL·이미지·`429`·실패·응답 붙잡기)와 임시 앱 데이터 폴더로만 돌고 실제 네트워크에 나가지 않아요. 명세 표의 행과 이 티켓의 행:
  - 같은 제목의 다른 시즌·동명 후보, 공백·대소문자, 구별 표기, 다음 페이지·중단된 조회: `title.rs` 세 시험, `two_entries_with_the_same_title_leave_the_cover_empty_and_the_work_as_it_is`(작품·회차는 그대로), `a_search_counts_only_when_read_to_its_last_page`.
  - 유일한 제목 `Lycoris Recoil`: `one_exact_title_is_selected_and_shown_only_after_its_bytes_are_verified`(이미지가 검증되기 전에는 표지가 없음), `an_entry_whose_image_is_not_an_image_selects_the_id_but_shows_nothing`.
  - 작품 약 520개: `hundreds_of_new_works_are_searched_one_at_a_time_at_the_pace`(두 프로세스가 한 간격을 나누고 그동안 라이브러리 목록이 2초 안에 답함), `a_failure_waits_and_a_429_holds_every_request_without_counting`, `the_queue_stops_on_shutdown_and_a_restart_resumes_it`, `requests_take_turns_and_a_block_holds_them_all`, `a_newly_recorded_work_is_auto_with_a_search_and_a_rescan_asks_for_none`.
  - 늦은 자동 조회·수신과 재선택·업로드·비우기: `a_late_automatic_result_never_undoes_a_newer_user_choice`, `a_late_automatic_image_never_undoes_an_upload_a_pick_or_a_clear`, `an_image_that_arrives_first_does_not_stop_a_choice_made_before_it`.
  - 두 화면의 수동 선택: `of_two_screens_changing_from_the_same_version_the_first_stays`, `a_change_from_an_old_version_changes_nothing`, `a_change_from_an_old_version_is_a_conflict_with_the_current_state`(API `409`).
  - 위장 파일과 상한: `the_format_comes_from_the_bytes`, `text_damage_and_other_formats_are_refused`(빈 파일, 텍스트, GIF, 잘린 JPEG·PNG), `the_limits_are_kept_at_their_bounds`(바이트·변·화소의 경계와 초과), `an_upload_is_judged_by_its_bytes_and_a_refusal_keeps_the_cover`, `an_upload_is_judged_by_its_bytes_not_its_name_or_type`(API로 `.png` 이름의 JPEG은 받고, `.jpg` 이름의 텍스트와 상한을 넘는 파일은 까닭과 함께 거부하며 표지가 남음).
  - 준비 실패·목적지 점유·공개 후 중단: `an_interrupted_publish_is_finished_by_the_files_identity`, `a_taken_place_is_never_overwritten`.
  - 작품 폴더 이동: `a_selection_follows_its_work_through_an_archive_move`, `removing_a_work_removes_its_selection_but_not_its_file_record`.
  - 누락·불일치·폴더 밖·링크 경로 제공과 복구: `an_image_is_served_only_while_its_file_is_the_recorded_one`(같은 크기의 다른 바이트, 위장한 텍스트, 잘린 파일, 올바른 바이트로 가는 기호 링크, 링크 폴더를 지나는 경로, `../`와 절대 경로), `an_image_whose_file_changed_is_not_served_and_its_state_says_why`, `a_missing_image_is_not_replaced_and_a_repair_fetches_the_same_entry`(직접 올린 파일 누락은 자동으로 대체하지 않음, 복구는 같은 ID만).
  - 같은 물리 파일을 가리키는 참조와 소유 불명확: `the_cleanup_keeps_files_other_references_lead_to`(같은 파일을 다른 이미지 ID로 가리키는 복제, 다른 이름의 경로 별칭, 하드 링크, 앱이 만들지 않은 파일, 파일 객체를 읽을 수 없는 참조가 있으면 아무것도 지우지 않음).
  - 그 밖에 `the_table_refuses_states_the_spec_does_not_allow`, `the_migration_gives_works_recorded_before_it_one_search`, `back_to_auto_searches_again_and_repair_fetches_the_same_id`, `search_and_pick_go_through_anilist_and_its_image_host_only`, `the_config_reads_overrides_and_allows_only_its_origins`.
- 브라우저(로컬 `trss-web`·`trss-worker`, 스크래치 DB·앱 데이터, 가짜 AniList, 작품 8개, 1280×900과 390×844, 2026-10-01):
  - worker가 작품 8개를 차례로 검색해 `Bocchi the Rock!`·`Kusuriya no Hitorigoto`·`Lycoris Recoil`·`Oshi no Ko`는 표지가 정해지고, `Clevatess`(같은 제목 둘)는 `모호함`, `Dandadan`(이미지가 텍스트)은 이미지 거부, `Frieren`·`Spy x Family`(폴더 이름과 같은 제목 없음)는 비어 있었어요. 격자·목록 모두 표지와 자리표시자가 맞게 보였어요.
  - `Clevatess` 상세에서 대화상자를 열어 `AniList에서 고르기` → 후보 두 개(작은 표지 포함) → `Clevatess #170001`을 고르니 머리와 대화상자가 바로 바뀌었어요. PNG 올리기는 받아들여 `직접 올린 파일이에요.`, `.jpg` 이름의 텍스트는 `JPEG·PNG·WebP 이미지가 아니에요. …`로 거부되고 앞의 표지가 남았어요. `자동으로 되돌리기`는 검색 중 안내 뒤 `모호함`으로 비었고, `표지 비우기`는 `표지를 비워 뒀어요. 자동으로 찾지 않아요.`예요. 더 쓰지 않게 된 이미지 파일(고른 표지, 올린 PNG)은 정리돼 `artwork/`에 남지 않았어요.
  - 버전이 하나 낮은 `auto` 요청은 `409`와 지금 상태, `다른 곳에서 먼저 표지를 바꿨어요. …`를 돌려줬어요(curl).
  - 앱 데이터의 `Bocchi` 파일 한 바이트를 바꾸고 `Oshi no Ko` 파일을 옮기니 격자에서 둘 다 자리표시자로 바뀌었어요. `Bocchi` 대화상자는 `표지 파일의 내용이 기록과 달라요. …`와 `다시 받기`를 보였고, 누르니 worker가 같은 ID(#130003)를 다시 받아 표지가 돌아왔어요.
  - 390px: 격자 두 열, 대화상자는 양옆 16px이고 안에서만 세로로 스크롤하며 검색 결과 두 열이 들어가요. `scrollWidth == innerWidth`예요.

### 검증하지 못한 것

- 실제 AniList(`graphql.anilist.co`, `s4.anilist.co`)에 묻지 않았어요. 실제 응답 모양·제목 표기·`429` 동작·이미지 출처는 공개 문서와 가짜 서버로만 맞췄어요.
- 운영 컨테이너(메모리 128M)에서 큰 이미지를 디코딩할 때의 메모리 사용은 재지 않았어요(상한은 계산으로 정했어요).
- 실제 수집 주기와 함께 520개 대기열을 돌리지 않았어요. 주기가 막히지 않는 것은 대기열이 주기 잠금 밖의 따로 된 작업이라는 구조와 라이브러리 응답 시험으로만 봤어요.
- 브라우저 확인은 창이 가려진 채라 `:focus` 표시와 대화상자 움직임을 눈으로 보지 못했고, 파일 선택은 DevTools로 파일 입력에 넣어 했어요(실제 파일 선택 창은 아님). 다크 모드, 실제 휴대폰·스크린 리더, 721–1099px 너비는 보지 않았어요.
- 자동 화면 시험은 없어요(이 저장소에 웹 시험 도구가 없어요).

### 남은 일

- 명세 표의 YAML 가져오기 행과 [작품 표지 필드](../specs/settings.md#작품-표지-필드)의 내보내기·가져오기는 결과 목표 5에서 해요. 저장 모양은 그 필드에 맞췄어요.
- Anissia 연결(제목·시작일 힌트)과 시즌별 AniList 연결은 아직 없어서 관찰값은 폴더 이름뿐이에요. 연결이 생기면 `title.rs`의 판정에 힌트를 더해요.
- 첫 배포 뒤 실제 AniList로 작품 몇 개를 확인하고, 운영 컨테이너에서 큰 이미지 올리기의 메모리를 봐요.
- 위 `결정`의 첫 항목(기존 작품의 검색 한 번)은 사용자 확인이 필요해요. 바꾸면 마이그레이션의 채우기 문장만 빼면 돼요.
