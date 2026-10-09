# 0106 할 일 집계를 trss-jobs로 옮기고 화면은 서버의 값을 보여줘요

- 상태: 완료
- 출처: [자막 작업 줄기의 재구성](refactoring.md#자막-작업-줄기의-재구성), [모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성), [할 일](../specs/jobs.md#할-일)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

[모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)은 할 일의 집계를 작업 관리에 맡겨요. 그런데 2026-10-07에 묶음, 정렬, 변경 합계(`Changes::add`), 배지 계산이 trss-web의 `todo_api`에 약 630줄 있었어요. 두 묶음 루프는 글자까지 같아요.
화면도 같은 규칙을 다시 계산해요. `todo/changes.ts`의 `planChanges`는 서버의 변경 합계를 따라 하고, `todo/kinds.ts`는 할 일 종류에서 배지를 다시 정해요. 시간 허용치 10 ms도 trss-subtitles에서 손으로 옮겨 왔어요.
변경 합계의 규칙 테스트는 화면에만 있고(`changes.test.ts`, 383줄), Rust에는 단위 테스트가 없어요.

- 묶음, 정렬, 변경 합계, 배지는 trss-jobs가 계산하고, trss-web은 직렬화만 해요.
- 서버가 계획마다의 변경 합계와 작품마다의 배지를 응답에 더해 보내고, 화면은 그 값을 보여줘요. 타입은 손으로 맞춰요([모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)).
- 화면의 규칙 테스트가 확인하던 경우는 지우지 않고 trss-jobs의 테스트로 내려요.

## 완료 기준

- 할 일 집계, 변경 합계, 배지의 구현이 trss-jobs에 하나씩 있고, trss-web과 화면에는 그 계산이 없어요.
- 할 일 응답의 기존 필드는 모양이 같고, 화면에 보이는 할 일, 변경 합계, 배지가 같아요. 화면 테스트가 이를 확인해요.
- `changes.test.ts`가 확인하던 경우가 trss-jobs 테스트에 있어요. 앞뒤 개수를 결과 절에 적어요.

## 결과

### 옮긴 곳

| 부품 | 지금 | 지운 복사본 |
| --- | --- | --- |
| 할 일 집계 | trss-jobs `todo`예요. `todo::list`가 작업, 구독한 제작자의 대응, 라이브러리의 영상, 받기 실패를 읽고, 작품마다 묶고(`group_by_work`), 합치고, 정렬해요. 읽기 실패를 견디는 방식은 예전과 같아요. | trss-web `todo_api`의 약 630줄. 글자까지 같던 두 묶음 루프(`인증 필요`, `교체 승인`)는 `group_by_work` 하나가 됐어요. |
| 변경 합계 | trss-jobs `todo::Changes`(`add`, 계획 하나의 `of`)와 받은 시각 `Received`예요. | 화면 `todo/changes.ts`의 `planChanges` |
| 배지 | trss-jobs `Todo::badge`와 `badges_by_work`예요. | 화면 `todo/kinds.ts`의 `kindOf`, `kindsByWork`. `TodoCards.tsx`가 카드마다 적어 둔 배지 |

- trss-web의 `todo_api`에는 `Sources`를 만들어 목록을 받는 일, 처리기, 오류를 API 오류로 바꾸는 일, 받기 실패 목록의 다시 받기 제안, `자막 구독` 제안이 남았어요.
- 표지 주소는 웹이 정해요. `Sources`가 `image_url` 함수를 넘기므로 trss-jobs는 웹의 경로를 몰라요.
- 영상의 `size:mtime_ns` 표기(`seen`)는 trss-jobs `todo`로 옮겼어요. 그것을 읽는 `parse_seen`은 trss-web `video_check_api`에 남았고, 두 곳의 주석이 서로를 가리켜요.
- 작업의 제목을 정하는 규칙(애니 제목, 없으면 작품 이름, 없으면 `작품을 찾지 못한 작업`)은 `JobRow::title`이 됐어요. trss-web의 `title_of`는 지웠어요.

### 응답에 더한 값

- `GET /api/todo`의 할 일마다 `badge`가 붙고, 응답에 작품 ID마다 배지 종류를 담은 `badges`가 붙어요. 라이브러리 목록도 같은 `badges_by_work`를 써요.
- 작업 상세(`GET /api/subtitle-jobs/{id}`)의 교체 계획마다 그 계획 하나의 변경 합계 `changes`가 붙어요. 모양은 `교체 승인` 할 일의 `changes`와 같아요.
- 계획마다의 합계를 할 일 응답이 아니라 작업 상세에 붙인 까닭은, 화면의 `planChanges`가 작업 상세의 회차 줄에만 쓰였기 때문이에요. 할 일 응답은 이미 합친 `changes`를 담고 있었어요.
- 기존 필드는 모양과 값이 같아요. 첫 커밋에서는 응답이 바뀌지 않았고, trss-web 테스트는 옮긴 하나 말고는 고치지 않고 통과했어요. 둘째 커밋은 새 필드를 확인하도록 웹 테스트 셋의 기대값을 넓혔어요. 지운 기대값은 없어요.

### 서버와 화면의 계산 차이

없었어요. `planChanges`의 6가지 경우와 배지 규칙을 같은 입력으로 Rust 코드에 넣어 같은 결과를 얻었어요.

### 테스트 개수

| 파일 | 앞 | 뒤 | 옮겨 간 곳 |
| --- | ---: | ---: | --- |
| 화면 `todo/changes.test.ts` | 31 | 30 | 계획 하나의 합계를 확인하던 테스트 하나(경우 6가지)가 trss-jobs `todo/tests.rs`의 테스트 6개가 됐어요. ASS 대 ASS, 비교 전 계획, 읽지 못한 비교, ASS 대 SRT, SRT 대 SRT, 짝이 없는 대사 언어예요. |
| 화면 `todo/kinds.test.ts` | 4 | 2 | `kindOf`(단언 3개)와 `kindsByWork`의 테스트가 trss-jobs 테스트 하나씩이 됐어요. `todosOfWork`와 `withKinds`의 테스트는 화면에 남았어요. |
| 화면 `todo/replacementView.test.ts` | 18 | 18 | 회차 줄 태그 테스트에 숨어 있던 ASS 대 SMI 경우가 trss-jobs 테스트가 됐어요. 화면 테스트는 서버의 `changes`를 태그로 보여주는 것만 확인해요. |
| trss-jobs `todo` | 0 | 28 | 첫 커밋에서 16개(옮긴 배지 테스트 1, 정렬 2, 합계 3, 받은 시각 3, 묶음 2, 받기 실패 묶음 4, 배지 없는 경우 1), 둘째 커밋에서 12개(화면에서 내린 경우 6, ASS 대 SMI 1, 배지와 응답 모양 5)를 더했어요. |
| trss-web | | | 배지 테스트 하나가 trss-jobs로 옮겨 갔고(`a_works_badges_are_its_kinds_once_each_in_the_lists_order`, 단언 그대로), 작업 상세의 계획 합계 테스트 하나(`a_plan_says_what_it_changes_as_one_plans_total`)를 더했어요. |

[RSS 수집 명세의 검증 표](../specs/collection.md#검증-표)가 인용하는 테스트는 옮기거나 지우지 않았어요.

### 검증한 것

2026-10-09에 확인했어요.

| 커밋 | cargo 통과 | 실패 | 무시 | `npm test` |
| --- | ---: | ---: | ---: | ---: |
| 시작(`refactor(collect): add the episode offset of an Anissia caption with the library's (0105)`) | 2,870 | 0 | 14 | 226 |
| `refactor(jobs): gather, group, sum and order the to-dos in trss-jobs` | 2,885 | 0 | 14 | 돌리지 않음(웹 바뀌지 않음) |
| `refactor(web): show the server's to-do badges and plan change totals`(이 결과를 담은 커밋) | 2,898 | 0 | 14 | 223 |

- cargo는 `cargo test --locked --workspace -j 4`예요. `npm run typecheck`는 깨끗해요.
- 마지막 행은 커밋 뒤에 cargo, `npm test`, `npm run typecheck`를 다시 돌려 확인했어요. 다시 돌린 cargo의 첫 실행에서 알려진 불안정 테스트 trss-web `the_holder_shown_is_the_first_subscription_by_rule_id_whatever_the_listing_order`가 한 번 실패했고, 다시 돌리면 통과했어요.
- `cargo fmt --all --check`는 깨끗하고, clippy에 새 경고가 없어요.
- 첫 커밋의 diff는 git의 이동 감지로 옮긴 줄(640줄)과 새로 쓴 줄을 나눠, 새로 쓴 줄을 예전 코드와 견주어 보았어요. 새로 쓴 줄은 `Sources`, 오류 변환, 하나로 합친 묶음 함수, 테스트예요.
- 화면은 테스트와 타입 검사로만 확인했고, 브라우저에서 띄워 보지 않았어요.

### 남은 것

- 화면 `todo/changes.ts`의 `timingRows`는 아직 시간 허용치 10 ms를 손으로 가져요. 서버가 고른 타이밍 줄에서 시작과 끝 중 어느 쪽이 움직였는지 보여주는 데만 쓰고, 합계를 다시 계산하지는 않아요. 없애려면 줄 응답(`/replacements/{plan}/lines`)이 움직인 쪽을 알려야 해요.
- 할 일 카드의 색(`tone`)은 여전히 카드 종류마다 화면에 적혀 있어요.
- `seen`과 `parse_seen`은 trss-library의 `SeenFile` 옆에 함께 두면 더 깔끔하지만, 다른 크레이트를 건드려 이 티켓에서 하지 않았어요.
