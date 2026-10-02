# 0027 자동 표지가 시즌의 AniList 연결을 따라가요

- 상태: 완료 (실제 AniList와 실제 서버 데이터로는 확인하지 않았어요. 아래 "검증하지 못한 것")
- 출처: [시즌 정보](../specs/library.md#시즌-정보)의 자동 표지 따라가기, [이미지 파일의 수명](../specs/library.md#이미지-파일의-수명)
- 막는 티켓: 없음

## 작업

작품 상세에서 시즌의 AniList 항목을 연결했는데 표지가 그대로라는 사용자 보고(2026-10-02)에서 시작했어요.
명세가 시즌 연결과 표지를 분리해 두었기 때문이며 버그는 아니었고, 표지가 자동 상태이면 연결을 따라가도록 명세를 바꿨어요(사용자 결정, 2026-10-02).

- 시즌 연결을 저장할 때(사용자의 연결·바꾸기·끊기와 앱의 첫 시즌 자동 연결 모두), 표지가 자동 상태이면 가장 앞 시즌에 연결한 첫 항목의 표지를 받아 검증한 뒤 표지로 삼아요.
- 직접 고르거나 올린 표지와 `disabled`는 바꾸지 않아요.
- 받는 동안이나 받지 못했을 때 기존 표지를 그대로 두며, 연결 저장은 표지 수신의 성공과 상관없이 끝나요.
- 표지 대화상자를 닫은 뒤에도 작품 상세 머리의 표지가 바뀐 표지로 갱신돼야 해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 자동 판정이 비워 둔 작품에서 시즌 1을 AniList 항목에 연결 | 그 항목의 표지가 검증된 뒤 머리와 라이브러리 목록의 표지로 보여요. 새로 고치지 않아도 머리가 바뀌어요. |
| 자동으로 고른 표지가 있는 작품에서 가장 앞 시즌의 첫 항목을 다른 항목으로 바꿈 | 새 첫 항목의 표지로 바뀌어요. |
| 시즌 2만 새로 연결 | 가장 앞 시즌의 첫 항목이 그대로면 표지도 그대로예요. |
| 직접 고른 표지·올린 표지·비운 표지가 있는 작품에서 시즌을 연결 | 표지가 바뀌지 않아요. |
| 표지 이미지 수신 실패(가짜 AniList 이미지 호스트) | 연결은 저장되고 기존 표지가 남으며, 표지 대화상자에서 실패를 확인할 수 있어요. |
| 늦게 끝난 자동 표지 수신과 사용자의 표지 직접 선택이 겹침 | 사용자의 선택이 남아요(기존 선택 버전 비교). |

## 결과

### 구현한 것

- 따라갈 항목(`src/store/seasons/repo.rs`의 `cover_target`): 작품 폴더에 기록된 시즌 가운데 항목을 연결한 가장 앞 시즌(시즌 0 특별편은 빼요)의 첫 항목이에요.
- 표지 쪽(`src/store/artwork/repo.rs`의 `follow_season_link`, `ArtworkStore::follow_season_link`): 한 트랜잭션에서 따라갈 항목을 읽고, 표지가 `auto`이며 고른 AniList 항목이 그 항목이 아니면 그 항목을 고른 것으로 바꿔요. 선택 버전을 올리고 `fetch` 작업을 걸며, 이전 판정의 메모는 지워요. 가진 이미지 참조는 그대로 두므로 새 이미지를 받아 검증해 `fetched`가 반영하기 전까지 기존 표지가 보여요. 그래서 새 이미지는 기존 수신 파이프라인(worker의 `fetch` 작업, `Artwork::store_image`의 형식·크기 검사, 선택 버전 비교)을 그대로 거쳐요. `manual`·`disabled`와, 이미 그 항목을 고른 `auto`는 건드리지 않아요. 마이그레이션은 없어요.
- 연결 저장 쪽(`src/seasons/mod.rs`의 `Seasons::follow_cover`): 사용자의 연결·바꾸기·끊기(`set_links`), `자동으로 다시 찾기`(`restart_auto`), worker의 첫 시즌 자동 연결(`src/seasons/queue.rs`의 `run_search`)이 연결을 저장한 직후에 불러요. 연결은 이미 저장된 뒤이므로 표지 쪽 오류는 기록만 하고 연결 저장은 그대로 성공해요.
- 작품 상세 API(`src/web/library_work_api.rs`): `cover_pending`(수신할 `fetch` 작업이 남았는지)을 더했어요.
- 화면(`WorkDetailScreen.tsx`, `api.ts`, `CoverDialog.tsx`): `cover_pending`인 동안 작품 머리가 3초마다 표지를 다시 읽고, 표지가 바뀌면 라이브러리 목록 캐시도 비워요. 표지 대화상자나 시즌 대화상자가 닫힌 뒤에도 새로 고치지 않고 머리가 바뀌어요. 표지 대화상자는 AniList 항목을 고른 표지에 메모(수신 실패)가 있으면, 기존 이미지가 보여도 `다시 받기`를 보여줘요.
- 문서 주석: `src/store/artwork/mod.rs`·`schema.sql`의 작업 설명에 이 요청이 `fetch` 작업을 만든다고 더했어요.

### 결정

아래는 구현하면서 해석한 것이에요. 사용자가 확인한 결정은 아니에요.

- 시즌 0(특별편)은 따라가지 않아요. 첫 시즌 자동 연결(`first_season`)이 시즌 0을 빼는 것과 같아요.
- 연결이 모두 끊겨 따라갈 항목이 없으면 표지를 그대로 둬요.
- 고른 항목이 같은데 이미지를 받지 못한 `auto` 표지에 같은 연결을 다시 저장해도 수신을 다시 요청하지 않아요. 표지 대화상자의 `다시 받기`(`repair`)를 써요.
- 따라가기로 바뀐 표지의 `anilist_media_id`는 새 항목이고, 이미지는 받기 전까지 이전 항목의 것이에요. 표지 대화상자의 "AniList에서 자동으로 고른 표지예요 (AniList #…)"는 그동안 고른 항목을 말해요.
- 아직 정하지 못한 표지(`auto`, 선택 없음)에 걸려 있던 자동 검색 작업은 시즌 연결을 저장하면 `fetch` 작업으로 바뀌어요. 진행 중이던 검색 결과는 선택 버전이 달라 버려져요.
- 보관 이동으로 작품이 합쳐질 때 옮겨지는 시즌 연결은 따라가지 않아요(`merge_links`).

### 검증한 것

- `cargo test`(전체): 1454개 통과, 0개 실패(라이브러리 1078개 포함). 이 티켓의 새 시험은 13개예요. `cargo clippy --all-targets`는 경고 없이 끝났어요.
- `src/seasons/cover_tests.rs`(가짜 AniList, 임시 앱 데이터 폴더, 시험이 움직이는 시계), 완료 기준의 각 줄에 대응해요.
  - 자동 판정이 비워 둔 작품에서 시즌 1을 연결: `linking_the_first_season_of_an_undecided_cover_receives_that_entrys_cover`, 메모가 있는 경우 `an_undecided_cover_with_a_note_is_taken_over_by_the_link`. 연결 직후에는 이미지 없이 항목만 고르고, 수신·검증 뒤에 이미지가 보여요.
  - 자동으로 고른 표지에서 첫 항목을 바꿈: `changing_the_first_entry_of_the_earliest_season_changes_the_cover_after_it_is_received`. 받기 전에는 기존 이미지가 그대로 서빙되고, 받은 뒤에도 `auto`로 남아 다음 변경도 따라가요.
  - 시즌 2만 연결, 첫 항목 뒤에 항목을 더함, 시즌 2를 끊음: `a_link_that_leaves_the_earliest_seasons_first_entry_keeps_the_cover_as_it_is`(선택이 변하지 않고 이미지 요청도 없어요).
  - 직접 고른·올린·비운 표지: `a_picked_an_uploaded_or_a_cleared_cover_is_not_changed_by_a_link`.
  - 수신 실패: `an_image_that_cannot_be_received_leaves_the_cover_and_the_saved_link`(이미지 호스트가 `404`, 재시도 간격으로 세 번 미룬 뒤 `failed`로 포기, 연결과 기존 표지 유지), `an_image_that_is_not_an_image_is_refused_and_the_cover_stays`(`rejected`).
  - 늦은 자동 결과와 사용자의 선택: `a_late_automatic_image_never_undoes_the_users_choice`(이미지 응답을 붙잡아 둔 사이 업로드하면 늦은 결과는 `Dropped`, 이전 버전 화면의 업로드는 충돌, 이후 연결도 사용자의 표지를 되돌리지 않아요).
  - 그 밖에 연결 끊기로 다음 시즌 따라가기, 시즌 0 제외, worker의 자동 연결(표지 자체 검색은 돌지 않아요), `자동으로 다시 찾기`.
  - 따라가기 호출을 빼고 시험을 돌려 보아, 따라가는 쪽 9개가 실패하고 건드리지 않는 쪽(`manual`·시즌 2·시즌 0)만 통과하는 것을 확인했어요.
- `src/web/seasons_api/tests.rs`의 `saving_a_link_lets_an_automatic_cover_follow_and_shows_a_failure_where_the_cover_view_reads_it`: `POST links`가 곧바로 답하고, 표지 대화상자가 읽는 `GET artwork`가 `pending: fetch`를, 작품 상세가 `cover_pending: true`를 보여요. worker가 받은 뒤에는 `cover_url`이 새 이미지예요. 이미지가 거부되면 `note.code = rejected`, `pending` 없음, 이전 이미지가 `available`로 남아요.
- `web/`: `bun run typecheck`(`tsc -b`)와 `bun run build`가 통과했어요. `web/package.json`에는 lint·test 스크립트가 없어요.
- 브라우저(로컬 `trss-web`과 `trss-worker`, 임시 DB, 직접 만든 가짜 AniList): 자동 검색이 항목을 찾지 못한 작품 상세에서 시즌 1에 `Alpha Show`(#10)를 연결해 저장하자 대화상자가 닫히고, 머리가 글자 표지에서 #10의 표지로 바뀌었어요(새로 고침 없음: 페이지에 심은 표식이 그대로예요). 이어서 연결을 `Beta Show`(#20)로 바꾸자 저장 뒤 약 5초 동안 이전 표지가 보이다가 #20의 표지로 바뀌었어요. 표지 대화상자를 연 적이 없어요.

### 검증하지 못한 것

- 실제 AniList(응답 모양, 속도 제한)와 실제 서버의 데이터로는 확인하지 않았어요. 가짜 AniList와 시험용 이미지만 썼어요.
- 라이브러리 목록 화면의 갱신은 브라우저로 보지 않았어요. 코드는 표지가 바뀌면 목록 캐시를 비우지만(`forgetPrefix(LIST_PREFIX)`), 목록의 표지가 바뀌는지는 시험하지 않았어요.
- 실패 시 `다시 받기` 단추와 그 문구는 브라우저로 누르지 않았어요(수신 실패는 API 시험까지만 봤어요).
- 휴대폰 화면과 두 화면의 동시 조작은 브라우저로 보지 않았어요.

### 남은 일

- 위 "결정"의 해석(시즌 0 제외, 연결이 모두 끊기면 표지 유지, 같은 연결의 재저장은 수신을 다시 요청하지 않음)은 사용자가 그대로 두기로 했고(2026-10-02), 앞의 둘은 명세의 시즌 정보에 옮겼어요.
- 명세의 [자동 판정](../specs/library.md#자동-판정) 끝에서 수신 작업을 만드는 동작에 시즌 연결 저장이 빠져 있던 것은 이 티켓과 함께 고쳤어요.
