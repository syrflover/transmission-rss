# 0105 회차 대응의 계산과 까닭 문구를 trss-library로 옮겨요

- 상태: 완료
- 출처: [자막의 회차 대응](../specs/library.md#자막의-회차-대응), [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0095](0095-episode-text-key-in-core.md)(회차 텍스트의 키)

## 작업

회차 대응(Episode Mapping)의 순수 계산(시즌 회차 구하기, 정한 이동값, 예외)은 trss-jobs의 `mapping`에 있어요. ADR 0015는 회차 대응의 규칙을 trss-library의 것으로 적었어요.
대응한 회차를 배치 대상이나 까닭 문구로 바꾸는 일은 2026-10-07에 trss-jobs의 5곳에 있었어요. 회차 배치(`place/episode`), 묶음 배치의 두 곳(`package`), 재배치(`relocate`), 기록의 범위 검사(`records`)예요.
같은 조건을 다른 말로 적어요. "시즌의 회차로 옮기지 못했어요", "정하지 못했어요", "다른 시즌의 파일로 보여요"가 그 예이고, `1화부터`와 `1–N화`도 4곳에서 따로 만들어요.
trss-collect의 Anissia 자막 목록은 trss-jobs의 대응을 볼 수 없어서 회차 이동을 직접 더해요.

- 대응의 순수 계산과 까닭 문구 만들기를 trss-library로 옮기고, trss-jobs와 trss-collect가 그것을 불러요.
- 화면과 웹 테스트가 문구를 글자 그대로 확인하므로, 문구는 지금 글자를 지켜요. 같은 조건의 다른 문구를 하나로 맞추면 화면이 바뀌므로 따로 커밋하고 결과 절에 적어요.

## 완료 기준

- 회차 대응의 계산과 까닭 문구가 trss-library에 하나씩 있고, trss-jobs의 5곳과 trss-collect가 그것을 불러요.
- 기존 테스트가 고치지 않고 통과해요.

## 결과

### 옮긴 곳

| 부품 | 지금 | 지운 복사본 |
| --- | --- | --- |
| 회차 대응의 계산 | trss-library `mapping`이에요. 시즌 회차 구하기(`Mapping::season_episode`, `whole`, `shifted`), 게시 시각으로 이동값 정하기(`decide`, `offsets`), 충돌(`conflicts`), 사용자 대응 검사(`UserMapping`)와 근거 문구가 있어요. 앱이 스스로 다른 이동값을 받아들이지 않는 규칙은 `store_in`에서 `reconcile`로 떼어 냈어요. | trss-jobs `mapping`의 순수 계산. trss-jobs `mapping`에는 행을 읽고 쓰는 SQL(`store_in`, `set_user_in`, `revert_in`, 충돌 표)과 `Saved`만 남았고, 옮긴 것을 다시 내보내요. |
| 까닭 문구 | trss-library `mapping::reason`이에요. 보이는 곳(`Wording`: 후보, 파일, 보관한 자막)마다 문구를 만들고, 범위 표기(`season_range`)와 시즌 안 검사(`in_season`)도 여기 있어요. | trss-jobs 5곳(`place/episode`의 `candidate_target`·`of_candidate`, `package`의 `other`·`named_target`, `relocate`의 `target`, `records`의 `confirm_placement`)이 손으로 만들던 문구와 범위 표기 |
| 이동값과 회차 범위의 표기 | trss-core `episode`의 `signed`와 `ranges`예요. trss-library는 trss-collect를 부를 수 없고, 회차 표기 `N화`가 이미 trss-core에 있어서 그 옆에 뒀어요([ADR 0016](../adr/0016-shared-parts-in-core.md)). | trss-collect `episode_offset`의 같은 함수 |
| Anissia 자막 목록의 회차 이동 | trss-collect `revision_by_attribution`이 `trss_library::mapping::shifted`를 불러요. | 회차를 직접 읽어 이동값을 더하던 코드 |

- 범위 표기는 티켓이 적은 4곳이 아니라 5곳에서 만들었어요. `records`는 `1–N화`만 만들었어요.
- trss-jobs의 테스트와 웹 테스트가 `trss_jobs::mapping`에서 가져오는 이름은 다시 내보내기로 지켰어요. 그래서 그 테스트를 고치지 않았어요.

### 다른 점마다 정한 것

- **넘침(동작 바뀜, 따로 커밋)**: `Mapping::season_episode`는 회차와 이동값을 `+`로 더했어요. `i64::MAX`에 가까운 회차에 양수 이동값이면 debug 빌드는 패닉하고 release 빌드는 음수 회차로 넘어갔어요. 이제 `checked_add`로 더하고, 넘치면 대응하지 않은 회차(`Unmapped`)로 둬요. 테스트로 패닉을 재현한 뒤 고쳤어요.
- **재배치의 미정**: 재배치에는 미정 갈래가 없어서, 대응이 미정이면 "…회차 대응으로 옮길 수 없는 회차예요"라고 했을 거예요. 이제 다른 곳처럼 미정 문구를 써요. 재배치는 정한 이동값이 없으면 먼저 돌아가므로(`reevaluate_in`) 화면에 닿지 않아요.
- **묶음 배치의 `other`**: 받지 않음, 대응 없음, 번호 체계가 모호함의 갈래를 `place` 뒤의 모호함 검사로 합쳤어요. 검사 순서는 그대로예요.
- **Anissia 자막 목록**: 이동값이 0이면 정수 회차가 아닌 글(`5.5`, `SP`)이나 `i64`보다 큰 번호도 그대로 비교해요. 회차 대응은 이런 글을 대응하지 않은 회차로 두므로, 이 갈래와 `n:0` 제외만 trss-collect에 남기고 나머지는 `shifted`를 거쳐요. 모든 입력에서 결과가 같아요. 그대로 비교하는 경우를 새 테스트가 고정하고, 이 테스트는 예전 코드에서도 통과해요.
- 화면 문구는 글자 그대로예요. `reason`의 테스트가 모든 문구를 보이는 그대로 확인해요.

### 같은 조건의 다른 문구

문구는 지금 글자를 지켰어요. 하나로 맞출지는 사용자 결정을 기다려요. `{l}`은 `13화` 같은 회차 표기, `{n}`은 대응한 시즌 회차, `{범위}`는 `1–12화`나 `1화부터`예요.

| 조건 | 후보의 작업 | 파일(받은 묶음, 사람이 올린 파일) | 보관한 자막(재배치) | 테스트 |
| --- | --- | --- | --- | --- |
| 정수 회차가 아님 | 후보의 회차 {l}는 정수 회차가 아니라 시즌의 회차로 옮기지 못했어요 | {l}는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요 | 없음 | 파일 쪽만 글자 그대로 확인해요 |
| 대응이 받지 않는 회차 | 회차 대응이 후보의 {l}를 받지 않는 회차로 정해 두었어요 | 회차 대응이 {l}를 받지 않는 회차로 정해 두었어요 | 회차 대응이 {l}를 받지 않는 회차로 정했어요 | 없음 |
| 정한 대응으로 옮길 수 없음 | 후보의 회차 {l}는 회차 대응으로 옮길 수 없는 회차예요 | {l}는 회차 대응으로 옮길 수 없는 회차예요 | 파일과 같아요 | 없음 |
| 시즌 밖 | 후보의 {l}를 옮긴 시즌 {n}화가 시즌의 {범위} 밖이에요 | {l}가 시즌의 {범위} 밖이에요 | {l}를 옮긴 시즌 {n}화가 시즌의 {범위} 밖이에요 | 후보와 파일 쪽 |

- 시즌 밖에는 두 문구가 더 있어요. 묶음의 다른 파일은 "{l}가 시즌의 {범위} 밖이라 다른 시즌의 파일로 보여요"라고 해요. 사람이 배치 확인에서 고른 회차를 거절할 때는 "{n}화는 이 시즌의 1–{N}화 밖이에요."라고 하고, 회차 수를 모르면 "{n}화는 회차가 될 수 없어요."라고 해요.
- 회차 수를 모를 때 시즌 밖은 1화보다 앞이라는 뜻인데, 문구는 "시즌의 1화부터 밖이에요"라고 읽혀 어색해요.
- 미정은 어디서나 "이 제작자의 회차 대응이 아직 미정이에요"예요.

### 검증한 것

2026-10-09에 `cargo test --locked --workspace -j 4`로 확인했어요.

| 커밋 | 통과 | 실패 | 무시 |
| --- | ---: | ---: | ---: |
| 시작(`fix(collect): receive the items that may decide a rule's offset one at a time (0130)`) | 2,852 | 0 | 14 |
| `refactor(collect): add the episode offset of an Anissia caption with the library's`(이 결과를 담은 커밋) | 2,870 | 0 | 14 |

- 다섯 커밋은 0130을 좁히기 전의 바탕에서 커밋마다 돌렸어요. 통과가 2,851, 2,856, 2,865, 2,867, 2,868개였고, 실패는 0개, 무시는 14개였어요. 0130을 좁힌 뒤에는 시작과 마지막 커밋만 다시 돌렸어요.
- 새 테스트는 18개예요. trss-core `signed` 1개, trss-library `mapping` 7개(`whole`, `shifted`, 넘침, `reconcile` 4개), `mapping::reason` 9개, trss-collect 자막 목록 1개예요.
- 옮긴 테스트는 이름과 본문이 그대로예요. trss-jobs `mapping::tests`의 22개가 trss-library `mapping::tests`로 같은 하위 모듈 경로째 옮겨 갔고, DB를 쓰는 `stored` 12개는 trss-jobs에 남았어요. `episodes_are_written_as_ranges`는 trss-collect `episode_offset`에서 trss-core `episode`로 옮겼어요. 옮기기 전 34개의 본문을 공백을 빼고 견주어 모두 같음을 확인했어요.
- 검증 표는 옮긴 테스트를 인용하지 않아 고칠 것이 없어요.
- `cargo fmt --all --check`는 깨끗하고, clippy에 경고가 없어요.
- 커밋마다 diff를 예전 코드와 견주어 보았어요. 동작을 바꾼 것은 넘침 하나이고 저장 형식과 API 모양이 그대로라 독립 리뷰는 하지 않았어요.

### 남은 것

- 같은 조건의 다른 문구를 맞출지 사용자 결정을 기다려요. 맞추면 `reason`의 `match` 갈래를 줄이는 커밋 하나예요.
- 파일 이름을 읽는 문구("파일 이름에 회차 번호가 없어요", "파일 이름이 회차 여럿을 가리켜요")는 세 곳에 있어요. 회차 대응의 문구가 아니라 옮기지 않았어요.
- trss-web `seasons_anissia_api.rs`가 "예외 먼저, 그다음 이동값"의 순서를 `revision_by_attribution` 둘레에서 되풀이해요. 정한 대응이면 `Mapping::season_episode`를 쓸 수 있지만, 이동값 0이면 글을 그대로 비교하는 차이가 있어 바꾸지 않았어요.
- trss-collect의 `1–N화` 문구(`past_search/range.rs`, `episode_offset.rs`)는 영상 회차 변환의 것이라 옮기지 않았어요. [라이브러리 명세](../specs/library.md#자막의-회차-대응)는 회차 대응과 회차 변환을 서로 전용하지 않는다고 해요.
