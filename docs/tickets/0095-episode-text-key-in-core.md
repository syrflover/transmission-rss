# 0095 회차 텍스트의 키, 정렬, 표기를 trss-core 하나로 모아요

- 상태: 완료 (2026-10-08)
- 출처: [흩어진 개념 모으기](refactoring.md#흩어진-개념-모으기), [ADR 0016](../adr/0016-shared-parts-in-core.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

회차 텍스트(`13`, `13.5`, `SP1`)를 숫자 키로 읽고, 정렬하고, `13화`로 적는 일이 2026-10-07에 8곳에 있었어요.
trss-subtitles의 `episode`, trss-collect의 Anissia 자막 목록, trss-library의 작품 상세와 목록, trss-jobs의 대응과 배치 문구와 작업 실행기, trss-web의 작품 API예요.
trss-subtitles는 trss-core와 trss-browser에만 의존하므로, 모두가 볼 수 있는 가장 아래인 trss-core에 둬요.

곳마다 다른 점이 있어요.

- 작품 목록은 `13.0`과 `13.5`를 "기타"로 정렬하고, 작품 상세는 `13.0`을 13으로 봐요.
- `f64`로 읽는 곳은 `1e3`, `inf`, `+5`를 숫자로 받아요. trss-subtitles의 키는 받지 않고, 그 테스트도 있어요.
- 자막 회차 예외에 저장하는 키는 `n:13` 꼴이에요. trss-subtitles의 키는 `13`이에요.

작품 목록도 `13.0`을 13으로 봐요(사용자 결정, 2026-10-08). `13.5` 같은 소수 회차는 목록에서 지금처럼 범위 뒤에 따로 보여요.

## 완료 기준

- 회차 텍스트의 키, 정렬, `N화` 표기가 trss-core에 하나씩 있고, 위의 곳들이 그것을 불러요.
- 저장된 `n:13` 꼴 키를 지금처럼 읽고 써요. 기존 테스트가 고치지 않고 통과해요.
- `13.0`을 13으로 볼지 사용자 결정을 받아 결과 절에 적어요. 결정 때문에 화면의 정렬이나 표기가 바뀌면 그 변경은 정리 커밋과 따로 커밋해요.
- `1e3`, `inf`, `+5`를 회차로 받던 곳이 받지 않게 되면, 그 차이를 결과 절에 적어요.

## 결과

### 결정

작품 목록도 `13.0`을 13으로 봐요(사용자 결정, 2026-10-08). `12`, `13.0`, `14`가 있으면 목록은 `12–14화`로 보여줘요.
화면이 바뀌는 수정이라서 정리 커밋보다 먼저 따로 커밋했어요(`e03223e`, `fix(library): list a 13.0 episode as episode 13 like the work detail`).
`13.5` 같은 소수 회차는 지금처럼 범위 뒤에 따로 보여요. 목록은 소수 회차를 글자 순서로 두므로 `10.5`가 `2.5`보다 앞이고, `13.5`와 `13.50`은 두 항목이에요. 이 순서를 지키려고 목록은 자기 키(`ListKey`)를 조금 남겨요.

### 바꾼 것

`ea12f18`(`refactor(core): read, order and label an episode text in one place`)에서 trss-core에 `episode` 모듈을 더했어요.

- `EpisodeNumber`는 회차 텍스트가 십진수일 때의 값이에요. `f64`를 만들지 않고 숫자를 글자로 지녀서 `013`, `13`, `13.0`이 같은 수예요. 숫자는 ASCII 숫자와, 있으면 `.` 뒤의 ASCII 숫자뿐이에요.
- `EpisodeKey`는 수이거나 그 밖의 글자예요. 수가 먼저 값 순서로 오고, 글자는 그 뒤예요.
- `stored_key`는 자막 회차 예외에 저장하는 `n:13`·`t:SP` 꼴을 써요. 저장 형식은 그대로라 마이그레이션이 없어요.
- 표기는 `episode_label`(쓴 그대로 뒤에 `화`, `013` → `013화`)과 `EpisodeKey::label`(키 꼴, `013` → `13화`) 두 가지예요. 아래 "남은 차이"를 봐요.

부르는 곳이에요.

- trss-subtitles: `episode`의 `numeric_key`와 `compare`를 지웠어요. 고르기와 `erulabo`의 카드 고르기가 `EpisodeKey`를 써요. trss-core가 dev-dependency에서 dependency로 옮겼어요.
- trss-collect: Anissia 자막 목록의 `numeric_episode`를 지웠어요. `episode_key`는 `stored_key`를 부르는 한 줄로 남아요.
- trss-library: 작품 상세의 `EpisodeKey`와 `key_of`를 지웠어요. 작품 목록의 `key_of`는 `EpisodeNumber`를 써요.
- trss-jobs: 대응(`mapping`), 따라가기(`follow`), 배치의 키 비교가 `EpisodeNumber`를 써요. 배치, 재배치, 작업 실행기의 `N화` 복사본 네 벌을 지우고 `episode_label`을 불러요.
- trss-web: 작품 API의 `attach_revisions`가 `EpisodeNumber`로 정렬 값을 만들어요.

### 받는 글자가 달라진 곳

- trss-jobs 작업 실행기(`runner.rs`)의 `N화` 표기만 실제로 달라졌어요. 이 곳만 `f64`로 읽었기 때문에 `1e3`, `inf`, `nan`, `+5`, `-1`, `.5`, `1.` 뒤에 `화`를 붙였어요. 이제는 붙이지 않아요(`1e3화` → `1e3`). 작업 사건과 로그 줄의 글자만 달라져요.
- 작품 상세 JSON의 `number`는 그대로예요. 예전에도 숫자 글자만 받는 키를 거쳐서, 위의 글자는 `null`이었어요.
- trss-web의 `attach_revisions`도 그대로예요. 회차를 정규식(`\d{2,}(\.\d)?`)으로 먼저 걸러요.
- `u128`을 넘는 정수(39자리 이상)는 작품 상세가 예전에는 글자로, 이제는 수로 정렬해요. 목록은 예전처럼 글자로 봐요.

### 남은 차이

- `N화` 표기가 두 꼴로 남았어요. trss-jobs의 문구는 쓴 그대로(`013화`), trss-subtitles의 두 문구는 키 꼴(`13화`)로 적어요. 하나로 맞추면 앞자리 0이나 `13.50` 같은 글자에서 보이는 문구가 바뀌어서 그대로 뒀어요. 그런 글자를 쓰는 테스트는 없어요.
- trss-web은 정렬에 `f64`인 `sort` 칸을 그대로 견줘요. 2^53을 넘는 회차에서만 키 순서와 달라지고, JSON의 칸 타입은 그대로예요.

### 검증한 것

2026-10-08에 `cargo test --locked --workspace -j 4`로 확인했어요.

- 앞은 통과 2,825개, 실패 0개, 무시 14개였어요.
- `e03223e` 뒤에는 2,826·0·14예요. 목록이 `12`, `13.0`, `14`, `13.5`를 `12–14화`와 그 뒤의 `13.5`로 보이는지, `13.0`만 있으면 쓴 그대로 보이는지, `13.0` 자막이 `13` 영상을 덮는지 확인하는 테스트를 더했어요. 기존 테스트는 고치지 않았어요.
- `ea12f18` 뒤에는 2,832·0·14예요. 저장 꼴 `n:13`, `n:13.5`, `n:15`를 확인하는 trss-collect와 trss-jobs의 기존 테스트가 고치지 않고 통과했어요.
- [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)대로 trss-subtitles의 `the_key_is_the_apps`와 trss-library의 `episodes_order_as_numbers_and_other_text_comes_last`를 지우고, 그 단언을 trss-core로 옮겼어요. trss-core의 테스트 8개는 키, 받지 않는 글자, 한 수의 여러 표기, 순서, `whole`과 `is_key`, `to_f64`, 저장 꼴, 두 표기를 확인해요.
