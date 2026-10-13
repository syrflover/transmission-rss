# 0138 Anissia에 이은 작품을 한글 작품명으로 보여줘요

- 상태: 완료 (2026-10-13, 개발 환경에서 확인. 서버 확인은 [0143](0143-deploy-feedback-fixes.md)에서 해요. 아래 "결과")
- 출처: [라이브러리 화면](../specs/library.md#라이브러리-화면)의 작품명(2026-10-11에 더한 요구), [용어집의 작품명](../glossary.md), 0.7.0을 써 본 사용자 피드백(2026-10-11)
- 막는 티켓: 없음

## 작업

0.7.0은 한국어 제목을 작품 상세 머리에만 써요. 그 제목도 구독이 이어진 시즌의 Anissia 제목에서만 가져와요(`trss-web` `library_work_api.rs`의 `korean_title`). 그래서 작품 상세에서 손으로 Anissia에 이은 `All Works Maid`는 폴더 이름으로 보이고, 라이브러리 격자와 목록은 모든 작품이 폴더 이름이에요.

[라이브러리 화면](../specs/library.md#라이브러리-화면)의 작품명 요구대로 만들어요. 사용자가 정한 것은 다음과 같아요(2026-10-11).

- 작품명은 구독으로 이은 시즌과 손으로 이은 시즌에서 모두 가져와요.
- 시즌마다 다른 Anissia 작품에 이었으면 가장 앞 시즌(시즌 1 이상)의 제목을 써요. `Season 00`의 극장판 연결은 쓰지 않아요.

명세에 리드가 적은 것도 있어요. 사용자가 다르게 정하면 명세와 함께 고쳐요.

- 작품명은 라이브러리 격자와 목록, 작품 상세 머리, 할 일처럼 라이브러리 작품을 부르는 곳에 같게 써요.
- `제목순` 정렬은 작품명을 따르고, 검색은 작품명과 작품 폴더 이름으로 모두 찾아요.

다음은 이 티켓에서 정해요.

- **작품명을 정하는 곳.** 목록 한 페이지가 작품명을 함께 담아 와야 해요. trss-library가 작품명을 읽는 쿼리를 하나로 두고 목록과 상세가 같이 쓰게 해요. Anissia 제목은 `anissia_anime.subject`에 있어요.
- **작품명을 쓰는 곳의 목록.** 이번 주 편성 카드와 수집 화면의 카드는 이미 구독한 Anissia 작품의 제목을 써요. 이 카드들을 바꿀지, 바꾼다면 무엇을 쓸지 코드를 보고 정해요. 할 일 카드도 봐요.

## 완료 기준

- 라이브러리 격자와 목록, 작품 상세 머리가 작품명을 보여줘요. 손으로 이은 작품도 한글로 보여요.
- 작품명을 정하는 규칙을 trss-library 테스트가 확인해요. 다음 경우를 겨눠요.
  - 구독으로 이은 시즌과 손으로 이은 시즌이 모두 작품명의 근거예요.
  - 시즌 1과 시즌 2가 다른 Anissia 작품에 이어지면 시즌 1의 제목이에요.
  - `Season 00`만 이었으면 폴더 이름이에요.
  - 이은 것이 없으면 폴더 이름이에요.
- `제목순` 정렬과 검색을 trss-library나 trss-web 테스트가 확인해요. 작품 폴더 이름으로도 찾아져요.
- [라이브러리 명세의 검증 표](../specs/library.md#검증-표)에서 작품명 행을 고쳐요.

## 결과

### 정한 것

- **작품명을 정하는 곳.** trss-library의 `WORK_TITLE`(`crates/trss-library/src/store/seasons/anissia.rs`)이 작품명을 읽는 SQL 식 하나예요. 기록된 시즌 가운데 시즌 1 이상에서 Anissia에 이은 가장 앞 시즌의 제목을 고르고, 없으면 폴더 이름이에요. 자동 표지처럼 기록된 시즌만 세므로, 잠시 옮겨진 시즌 폴더의 연결은 그동안 작품명에 쓰지 않아요. 구독의 연결(`trss-collect`의 `link_season`)과 작품 상세의 연결이 같은 `season_anissia` 행을 쓰므로 누가 이었는지는 묻지 않아요.
- 목록(`WorkOverview::title`), 작품 상세(`WorkDetail::title`), 규칙 상세의 시즌(`SeasonHoldings::title`), `영상 확인` 할 일, 자막 작업의 행(`trss-jobs` `JOB_COLUMNS`), `자막 구독` 제안과 회차 확인(`trss-jobs` `follow.rs`), `받기 실패`(`trss-collect` `work_at`)가 모두 이 식을 써요. 설정의 자막 형식 순서 목록은 `trss-core`가 읽으므로 `trss-web`이 `LibraryStore::titles`로 이름을 바꿔 넣어요. 기능 규칙을 `trss-core`에 두지 않는다는 [ADR 0016](../adr/0016-shared-parts-in-core.md) 때문이에요.
- API는 목록과 작품 상세에 작품명 `title`을 더하고 `name`을 폴더 이름으로 남겨요. 작품 상세의 `korean_title`은 없앴어요. 작품명만 보내는 다른 API(할 일과 작업의 `work.name`, 규칙의 `work_name`, `파일 용량·정리`와 자막 형식 순서의 `name`)는 필드 이름을 그대로 두고 값을 작품명으로 바꿨어요.
- **작품명을 쓰는 곳의 목록.** 이번 주 편성 카드, 구독 카드, 자막 작업과 그 할 일, `자막 구독` 제안은 한 시즌의 Anissia 작품을 다루므로 그 작품의 Anissia 제목을 그대로 쓰고, Anissia 작품이 없을 때만 작품명을 써요. 이 결정은 [라이브러리 화면](../specs/library.md#라이브러리-화면)에 리드 결정으로 적었어요.
- 작품 상세에서 Anissia 연결을 바꾸면 머리의 작품명이 따라오도록 페이지와 라이브러리 목록을 다시 읽어요. 표지 창의 AniList 검색은 한글 작품명이 아니라 폴더 이름으로 시작해요.

### 개발 환경 확인 (2026-10-13)

f75ec6d 위의 0142·0141 커밋에 이 변경을 얹은 작업 트리를 서버 DB 사본(작품 88편)으로 띄워 내장 브라우저(Chromium)로 봤어요. 사본에서 Anissia에 이은 작품은 구독으로 이은 `FX Senshi Kurumi-chan` 하나였어요.

| 확인 | 결과 |
| --- | --- |
| 손으로 잇기 | `All Works Maid`의 시즌 1을 작품 상세의 `Anissia 연결`로 잇자 머리와 탭 제목이 바로 `히로인? 성녀? 아니요, 올 워크스 메이드 입니다(자랑)!`가 됐어요. |
| 라이브러리 목록 | 두 작품만 작품명이 폴더 이름과 달랐어요(`FX 전사 쿠루미`, 위 제목). `제목순` 88편이 작품명 순서였고, 격자 카드가 한글 작품명을 두 줄로 줄여 보여 줬어요. |
| 검색 | `올 워크스`와 `all works`가 모두 `All Works Maid` 한 편을 찾았고, `쿠루미`가 `FX Senshi Kurumi-chan`을 찾았어요. |
| 표지 창 | 제목이 `표지 · 히로인? …`이었고, `AniList에서 고르기`의 검색어는 `All Works Maid`였어요. |
| 파일 용량·정리 | `FX Senshi Kurumi-chan`이 `FX 전사 쿠루미`로 보였어요. |
| 가로 넘침 | 다섯 폭(1440·1024·768·390·320px)에서 `/library`, 긴 제목의 작품 상세, `/settings/storage`, `/todo`의 20개 조합 모두 넘침이 없었어요. |

`cargo test --workspace -j 4`의 3,034개, `npm test`의 226개와 `npm run typecheck`가 통과했어요. 새 테스트와 바꾼 테스트는 다음과 같아요.

- `trss-library` `store/seasons/anissia_tests.rs` `the_work_title_is_the_anissia_title_of_the_lowest_linked_season_from_one_on`: 이은 것이 없을 때, `Season 00`만 이었을 때, 시즌 2만 이었을 때, 시즌 1과 2가 다른 작품에 이어졌을 때, 시즌 1의 연결을 끊었을 때, 시즌 폴더가 잠시 없을 때의 작품명이에요. 목록과 상세가 같은 값인지도 봐요.
- `trss-library` `store/library/page.rs` `the_title_sort_follows_the_work_title_and_a_search_finds_the_title_or_the_folder_name`: 폴더 이름 순서와 다른 `제목순`, 작품명과 폴더 이름 양쪽으로 찾는 검색이에요.
- `trss-web` `library_work_api/tests.rs` `the_head_of_a_work_names_the_subscription_of_its_season_and_the_work_title`: 구독의 연결이 작품 상세와 목록의 `title`이 되는 것이에요.
- `trss-web` `policy_api/tests.rs` `the_works_with_their_own_order_are_listed_with_their_work_titles`: 자막 형식 순서 목록의 이름이에요.
- 작품명이 바뀐 것을 기대값에 반영한 테스트 일곱 개(`trss-jobs` `tests/it/follow.rs` 하나, `trss-web`의 `todo_api` 둘, `jobs_api`, `subtitle_upload_api`, `subscriptions_api` 둘)예요. 시험 작품의 시즌 1이 Anissia 작품 `작품`에 이어져 있어서 폴더 이름 대신 `작품`이 됐어요.

### 확인하지 않은 것

- 할 일, 자막 작업, 규칙 상세, 자막 형식 순서 목록의 작품명은 API 테스트로만 확인했어요. 서버 DB 사본의 할 일은 `FX 전사 쿠루미`의 `교체 승인` 하나였는데, 그 작업의 Anissia 제목이 작품명과 같아서 바뀐 것을 화면에서 가려 볼 수 없었어요. 나머지 화면은 보지 않았어요.
- `Season 00`만 이은 작품과 2기만 이은 작품은 테스트로만 확인했어요.
- 극장판만 있는 작품의 작품명은 [0139](0139-movie-only-works.md)에서 만들어요.
- 서버 확인은 [0143](0143-deploy-feedback-fixes.md)에서 해요.
