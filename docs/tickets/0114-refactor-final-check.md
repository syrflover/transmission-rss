# 0114 리팩터링을 마치며 다시 재고 실제 서버에서 확인해요

- 상태: 진행 중. 측정, 예전 복사본 찾기, 배포 전 확인을 마쳤고, 실제 서버에 올리는 일은 사용자 승인을 기다려요.
- 출처: [잣대와 기록](refactoring.md#잣대와-기록), [완료 판정](refactoring.md#완료-판정)
- 막는 티켓: [0093](0093-migrated-test-db-once.md)–[0113](0113-remaining-area-tests.md), [0115](0115-erai-magnet-revisions.md), [0117](0117-sql-statement-cache.md), [0128](0128-keep-unnamed-cycle-torrent.md), [0130](0130-receive-a-rule-in-turn.md)

## 작업

리팩터링의 다른 티켓을 모두 마친 뒤, [0092](0092-refactor-baseline.md)와 같은 명령과 빌드 상태로 다시 재요.
그다음 정리한 릴리스를 실제 서버에 올리고, 목표 5에서 기준선으로 삼은 흐름을 한 번 더 이어 봐요.

- 배포하기 전에 workspace 테스트를 musl 대상으로도 돌려요([0079](../archive/tickets/5-deployed-verification/0079-musl-test-run.md)의 절차).
- 마이그레이션이 더해졌으면 실제 서버 DB의 새 복사본으로 먼저 확인해요([0080](../archive/tickets/5-deployed-verification/0080-server-db-migration-check.md)의 절차).
- 태그, 이미지 게시, 실제 서버에 올리는 일은 사용자 승인을 받아요.

## 완료 기준

- [리팩터링 명세의 기록](refactoring.md#기준값과-마지막-측정)에 마지막 측정이 0092와 같은 항목으로 있어요. 테스트 시간이 줄지 않았으면 까닭이 있어요.
- 변경 범위와 테스트 개수의 앞뒤가 기록에 있어요.
- [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기)의 행마다 예전 복사본이 없는지 이름으로 찾아본 결과가 결과 절에 있어요.
- 실제 서버에서 [0083](../archive/tickets/5-deployed-verification/0083-deployed-end-to-end.md)의 흐름을 다시 이어 본 결과가 있어요. 0083과 다른 점이 있으면 원인을 가려요.
- 리팩터링 명세에 완료 날짜, 결과, 남은 일, 리팩터링 커밋들(제목과 hash)이 있고, 명세와 리팩터링 티켓을 `docs/archive/tickets/` 아래의 한 폴더로 옮겼어요.

## 결과

### 마지막 측정

2026-10-10에 `cc91ce5`에서 [0092](0092-refactor-baseline.md)와 같은 명령과 빌드 상태로 측정했어요. 값과 바이너리별 표, musl 실행, 변경 범위는 [리팩터링 명세의 마지막 측정](refactoring.md#마지막-측정)에 있어요.

- 테스트 바이너리의 `finished in` 합은 기준값 두 번의 평균 56.45초에서 43.0초로 23.8% 줄었어요. 통과한 테스트는 2,818개에서 3,026개로 208개 늘었어요.
- 변경 범위를 같은 방법으로 세면, 리팩터링 기간의 `feat`·`fix` 커밋 30개는 평균 크레이트 1.90개·파일 5.7개에 닿았어요. 기준값의 44개는 2.70개·13.2개였어요. 리팩터링 기간의 `feat`은 [0111](0111-web-and-screen-rules.md)의 것이라, 정리한 코드에서 기능 하나가 닿는 범위는 목표 6의 기능 작업에서 다시 세어 봐야 해요.
- 측정한 뒤 아래 "예전 복사본 찾기"에서 고친 커밋 세 개가 코드를 바꿨어요. 그 마지막 커밋에서 테스트를 다시 돌린 결과는 "배포 전 확인"에 있고, 그 실행의 시간은 측정으로 쓰지 않았어요.

### 예전 복사본 찾기

2026-10-10에 `cc91ce5`에서 [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기)의 행마다 예전 복사본을 이름으로 찾았어요.
지금 둔 곳의 이름과 예전 복사본이 남기는 흔적(`st_dev`·`.dev()`, `Retry-After` 읽기, `sync_all`, `renameat2`, `/proc/net/route`, 손으로 쓴 `n:` 키, `format!("{b:02x}")`, 화면의 `^0+`·`padStart`·`Number(` 같은 것)을 grep과 codegraph로 찾았어요.
찾은 것은 모은 곳이나 그것을 부르는 곳, 티켓이 일부러 남긴 것으로 적은 복사본, 어느 문서도 설명하지 않는 복사본으로 나눴어요. 마지막 것은 리드가 코드를 읽어 확인하고 처리했어요.

| 행 | 부르는 곳(대략) | 남은 복사본과 처리 |
| --- | ---: | --- |
| 같은 파일 알아보기 | 45곳 | trss-collect 규칙 보관의 `RealDisk::device`가 `st_dev`를 직접 읽었어요. 이제 `folder_check::device_of`를 불러요. trss-probe의 `identity`(`dev`·`ino` 쌍)는 남겨요. trss-probe는 trss-core에 의존하지 않고, [0104](0104-durable-file-writes-in-core.md)도 같은 까닭으로 그 `renameat2`와 fsync를 남겼어요. |
| 회차 텍스트의 키, 정렬, `N화` | 75곳 | Anissia 자막 목록의 수정본 표시가 저장 키 `n:…`를 세 곳에서 손으로 썼어요. 이제 `episode::stored_key`를 불러요. 작품 상세의 후보 목록은 회차 키, 숫자 비교, 정렬을 화면에서 다시 구현했어요(`episodeKey.ts`의 `numericKey`·`episodeKey`, `candidates.ts`의 `compareEpisodes` 등). 할 일의 배치 표는 이름의 회차와 놓을 회차를 `Number(p.named) === p.episode`로 견줬어요. 둘 다 이제 서버가 trss-core `episode`로 계산해 보내요. 응답에 더한 필드는 `episode_key`(작품 상세의 회차와 후보), `episode_rank`·`episode_form`(후보), `named_is_episode`(배치 표의 줄)예요. |
| 요청 간격, `429` 대기, `Retry-After`, 응답 크기 제한 | 30곳 | 없어요. 표지 올리기의 `read_body`(`trss-web` `artwork_api.rs`)는 받는 요청의 본문을 읽는 것이라 응답 크기 제한과 다른 개념이에요. |
| worker 백그라운드 큐의 루프 | 50곳 | 없어요. 0103이 지우라고 한 `lock_path_for` 감싸개도 남지 않았어요. |
| 파일을 안전하게 쓰기 | 40곳 | 없어요. |
| 릴리스 이름과 회차 읽기 | 95곳 | 없어요. 정규식은 모두 trss-collect `release_name`과 trss-core `trname_names`에 있어요. |
| 회차 대응의 계산과 까닭 문구 | 45곳 | 없어요. |
| 웹이 다시 구현한 규칙 | 60곳 | 없어요. 다만 하루의 밀리초가 trss-collect `store::status`와 trss-jobs `recheck`에 상수로 따로 있었어요. 표가 말한 웹의 복사본은 아니지만 같은 값이라 `calendar::DAY_MS`를 쓰게 했어요. |
| 화면이 다시 계산하는 규칙 | 25곳 | 회차 키의 행에 적은 후보 목록과 배치 표가 이 행에도 걸려요. 표기만 하는 것 중 분기 이름(`2026년 4분기`)이 편성, 구독 탭, 규칙 카드에 세 벌 있었어요. 요일 이름처럼 `web/src/lib/quarter.ts` 한 곳으로 모았어요. 후보 목록의 갱신 시각(`updatedText`)은 연도를 붙이는 날짜 표기를 따로 가져서, 같은 규칙을 다루는 [0133](0133-web-notation-rules.md)에 적었어요. 지난 회차 검색의 여섯 자리 입력 검사는 서버 한도를 옮겨 적은 것이 아니라 화면의 입력 검사예요. |
| 작은 도우미 | 45곳 | 수정본 재확인이 8바이트 요약을 손으로 16진수로 적었어요. 이제 `files::hex`를 불러서, [0096](0096-file-and-path-helpers.md)이 "지금처럼 8바이트만 넘겨요"라고 적은 것과 맞아요. Anissia의 바쁨 문구는 웹에 두 문장으로 있어요. 구독 API의 오류는 `…다시 시도해 주세요.`이고, 가져오기 제안의 까닭은 `…기다려야 해요.`예요. 화면 문구는 글자 그대로 두므로 남겨요. |

- 부르는 곳의 수는 찾은 줄을 센 대략의 값이에요.
- 표 밖에서 비슷한 코드를 두 가지 봤어요.
  - 나뉜 압축 파일의 조각 이름은 trss-jobs `place/unpack.rs`의 `volume`과 trss-archive `source.rs`의 `is_volume_name`이 따로 읽어요. 앞의 것은 조각을 묶음과 번호로 나누고, 뒤의 것은 조각인지만 보며 `.01`처럼 두 자리 확장자도 조각으로 봐서, 같은 규칙이 아니에요.
  - 규칙의 저장 폴더에서 작품 폴더를 정하는 규칙(수집 폴더 아래 첫 부분)은 trss-collect `plan::work_folder_of`와 `rule_archive::work_folder`가 각자 계산해요. 돌려주는 꼴과 `..`·UTF-8이 아닌 이름의 처리가 달라서, 합치려면 그 차이를 정해야 해요. 이번에는 두고 리팩터링 명세의 남은 일에 적어요.

고친 커밋이에요.

| 커밋 | 바꾼 것 |
| --- | --- |
| `refactor: use trss-core's episode key, hex, day length and device number where copies were left (0114)` (`5eb6d98`) | Anissia 수정본 표시의 `n:` 키, 수정본 재확인의 16진수, 하루 길이 상수 두 개, 규칙 보관의 장치 번호 |
| `refactor(web): name a quarter in one screen module (0114)` (`653f03b`) | 분기 이름 세 벌 |
| `feat(web): send the episode keys, order and forms the screens compared by hand (0114)` (`6caf49e`) | 후보 목록의 회차 키·순서·꼴과 배치 표의 회차 비교 |

- `6caf49e`의 규칙은 주인 크레이트에서 테스트해요. trss-core에 `zero_and_the_episode_a_number_is`와 `ranks_follow_the_order_of_the_keys_and_share_one_per_episode`를 더했어요. trss-web에는 응답 모양을 보는 `a_candidate_carries_the_key_rank_and_form_of_its_episode`를 더하고, 작품 상세와 배치 표의 기존 테스트에 새 필드를 더했어요.
- 화면이 순위로 정렬하며 `최신 화부터`에서 숫자만 뒤집는 부분은 테스트가 없어요. `candidates.ts`가 React를 불러서 `node --test`로 돌릴 수 없어요.
- 화면에 보이는 것은 같아요. 다만 숫자가 아닌 회차 텍스트끼리의 순서가 화면의 UTF-16 순서에서 Rust의 바이트 순서로 바뀌었어요. 둘은 U+FFFF보다 큰 글자를 U+E000–U+FFFF의 글자와 견줄 때만 달라요.

### 배포 전 확인

- 마지막 코드 커밋 `6caf49e`에서 `cargo test --locked --workspace -j 4`를 돌렸어요. 3,029개가 통과했고, 실패는 0개, 무시는 14개였어요. 측정 때보다 늘어난 3개는 `6caf49e`가 더한 테스트예요.
- 같은 커밋에서 musl 대상([0079](../archive/tickets/5-deployed-verification/0079-musl-test-run.md)의 `dev/musl-test.sh`)으로도 돌렸어요. 3,029개가 통과했고, 실패는 0개, 무시는 14개였어요.
- 0.6.2(`bab88cf`) 뒤로 마이그레이션, compose 파일, Dockerfile이 바뀌지 않았어요. 그래서 [0080](../archive/tickets/5-deployed-verification/0080-server-db-migration-check.md)의 서버 DB 복사본 확인은 하지 않고, 서버에서는 `.env`의 `TRSS_VERSION`만 바꿔요.
- `6caf49e`에서 앱 이미지와 브라우저 이미지를 개발 PC에서 빌드했어요(`dev/compose.sh up`, 73초). 개발 환경을 그 이미지로 다시 띄우자 세 컨테이너가 떴어요. worker는 `cc91ce5` 이미지로 먼저 띄웠을 때 첫 주기를 오류 없이 돌았고(항목 250개를 봤고 Transmission에 더한 것은 0개), 다시 띄운 뒤에는 직전 주기가 가까워 첫 주기를 건너뛰었어요. 할 일, 라이브러리, 편성, 수집의 구독 탭이 콘솔 오류 없이 열렸어요. `FX Senshi Kurumi-chan`의 작품 상세는 바꾸기 전과 같은 글자였고, 응답에 새 필드(`episode_key` `n:1`·`n:2`, 후보의 `episode_rank` 0·1, `episode_form` `number`)가 있었어요. 개발 데이터에는 후보가 한 제작자의 두 회차뿐이라, 여러 제작자와 숫자가 아닌 회차의 순서는 화면에서 보지 못했어요.

### 남은 것

- 사용자 승인을 받아 태그를 달고 이미지를 게시해 실제 서버에 올려요. 그 뒤 실제 서버에서 [0083](../archive/tickets/5-deployed-verification/0083-deployed-end-to-end.md)의 흐름을 다시 이어 봐요.
- 리팩터링 명세에 완료 날짜, 결과, 남은 일, 커밋을 적고, 명세와 리팩터링 티켓을 `docs/archive/tickets/` 아래로 옮겨요.
