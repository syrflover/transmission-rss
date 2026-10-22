# 0157 화면 타입 생성 도구를 들이고 수집 화면이 쓰는 API의 타입을 생성해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 화면 타입 생성](lighter-structure.md#화면-타입-생성), [웹 앱 공통 명세의 공통 라이브러리의 모듈 구성](../../../requirements/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: [0145](0145-merge-web-and-worker-crates.md)(크레이트 합치기)

## 작업

화면의 TypeScript 타입은 Rust 응답과 손으로 맞춰요. 2026-10-21에 `trss-web`에는 `Serialize`를 붙인 구조체·열거형이 206개, 테스트 밖의 `json!` 호출이 75곳 있었어요. 이 티켓은 Rust 타입에서 화면 타입을 생성하는 길을 만들고, 수집 화면이 쓰는 API부터 옮겨요. 수집 화면은 구독, 규칙, 수집 기록, 지난 회차 검색, 수집 상태판, 이번 주 편성이에요. 라이브러리 화면은 [0158](0158-generated-types-library.md)이, 나머지 화면과 이벤트는 [0159](0159-generated-types-rest-and-events.md)가 옮겨요.

이 티켓 뒤에 새로 만드는 API 타입은 처음부터 생성해요.

명세를 쓰며 정한 것은 다음과 같아요.

- 생성 도구는 serde 속성을 읽는 ts-rs예요.
- 정수는 TypeScript에서 `number`로 생성해요. 지금 화면이 정수를 `number`로 받고 있어서예요.
- 생성한 파일은 저장소에 커밋해요. 화면을 빌드하는 Docker 단계에는 Rust가 없어서예요.

`json!`으로 만들던 응답을 구조체로 바꿀 때, 빠진 필드와 `null`인 필드의 차이까지 지금 JSON과 같게 해야 해요. API의 JSON 모양은 그대로라는 조건 때문이에요.

## 완료 기준

- [x] 명령 하나로 Rust 타입에서 화면 타입을 생성하고, 그 명령이 [웹 앱 공통 명세의 공통 라이브러리의 모듈 구성](../../../requirements/web-app.md#공통-라이브러리의-모듈-구성)이나 readme의 개발 환경에 있어요.
- [x] 생성한 파일은 저장소에 있고, 생성 결과가 커밋한 파일과 다르면 Rust 테스트가 실패해요. 타입 하나를 바꾸고 생성하지 않은 채 테스트를 돌려 실패하는 것을 확인해요.
- [x] 정해진 문자열만 오는 필드는 TypeScript에서 문자열 리터럴 유니언이고, `string`으로 넓히지 않아요. 정수 필드는 `number`예요.
- [x] 수집 화면이 쓰는 API의 응답과 요청 본문은 모두 Rust 구조체나 열거형이고, 그 타입은 생성돼요. `json!`으로 만들던 응답과, 모든 화면이 함께 쓰는 오류 본문도 구조체가 돼요.
- [x] 수집 화면에서 손으로 쓴 같은 뜻의 타입은 지우고, 화면은 생성한 타입을 써요. 생성한 이름끼리 겹치지 않아요.
- [x] 지금 API 테스트가 고치지 않고 통과해요. 바꾼 `json!` 응답은 결과에 적고, 빠진 필드와 `null`의 차이를 확인한 방법을 적어요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-21에 ts-rs 12.0.1로 Rust 타입에서 화면 타입을 생성하는 길을 만들었어요. 수집 화면이 쓰는 API와 모든 화면이 함께 쓰는 오류 본문의 타입을 생성하고, 화면이 그 타입을 쓰게 했어요. 생성한 파일은 `web/src/lib/api-types/`의 타입 파일 118개와 그것을 모아 내보내는 `index.ts`이고, 화면은 `@/lib/api-types`에서 가져와요. API의 경로와 JSON 모양은 그대로예요.

### 생성하는 길

- 저장소 루트에서 `cargo types`를 실행하면 파일을 다시 만들어요. 이 별칭은 `trss`의 무시된 테스트 `web::api_types::write_committed_files`를 실행하고, 명령은 [웹 앱 공통 명세의 공통 라이브러리의 모듈 구성](../../../requirements/web-app.md#공통-라이브러리의-모듈-구성)에 적었어요.
- `web::api_types::committed_files_are_current`는 임시 디렉터리에 생성한 결과가 커밋한 파일과 다르면, 다른 파일 이름을 대며 실패해요.
- 생성은 `api_types.rs`의 `roots`에 올린 타입에서 시작해 그 타입이 담은 타입까지 따라가요. 두 Rust 타입이 TypeScript에서 같은 이름이 되면 생성이 멈추고 `ts(rename)`으로 하나를 바꾸라고 알려요.
- ts-rs는 테스트 빌드에만 들어가요. 기능 크레이트(`trss-core`, `trss-collect`, `trss-library`)는 `ts` 기능 뒤에서 `TS`를 붙이고 `trss`는 `cfg_attr(test, …)`로 붙이며, `ts` 기능은 `trss`의 dev-dependencies만 켜요. `cargo tree -p trss -e normal,build`에 ts-rs가 없어요.
- 정수는 `with_large_int("number")`로 `number`가 돼요.
- Rust의 doc comment가 TypeScript 파일에 주석으로 들어가요. 그래서 doc comment만 고쳐도 다시 생성해야 하고, 잊으면 위 테스트가 잡아요.

### 바꾼 응답과 요청 본문

- `json!`으로 만들던 응답은 모두 오류 본문이었어요. `error.rs`의 `ApiError` 응답 7곳, 없는 API 경로의 404(`api.rs`), 출처 확인의 거절(`origin_guard.rs`), 서버 브라우저 화면의 거절(`screen_api.rs`)이 `ErrorBody`와 `ErrorCode`가 됐어요. 채널에 이미 그 애니를 따르는 규칙이 있을 때의 409가 담는 `current`는 `AlreadySubscribed`예요. 이제 수집 화면이 쓰는 API 파일 12개에는 테스트 밖의 `json!`이 없어요.
- `&'static str`로 보내던 코드 필드는 같은 글자로 직렬화되는 열거형이 됐어요. `HistoryResult`, 처음 설정의 `Step`, 지난 회차 검색 결과의 `State`, `RuleState`, `SubtitleMode`, 보관 명령의 `Direction`, `CommandState`, `PastCause`, `UndoFileState`, `GroundKind`, `HeldBy`, `PollState`예요. `enums_serialize_as_their_codes`가 열거형마다 예전 글자와 같은지 확인해요.
- 명령 요청은 `CommandRequest`, `CommandKind`, `CommandPayload`로 생성해요. 이 셋은 테스트 빌드에만 있고, 서버는 지금처럼 `kind` 글자로 `Request::read`에서 본문을 골라 읽어요. `command_requests_are_the_ones_the_web_reads`는 종류마다 화면이 쓰는 본문을 서버가 읽는지 확인하고, `Request`에 종류를 더하면 그 `CommandKind` 없이는 이 테스트가 컴파일되지 않아요.
- 규칙 저장 본문의 `state`는 Rust에서 글자로 읽고, TypeScript에서만 `RuleState`로 생성해요(`ts(as)`). 열거형으로 읽으면 모르는 상태를 본문을 읽을 때 400으로 거절해서, 지금처럼 규칙과 버전을 확인한 뒤(404, 409) 거절하는 순서가 바뀌어요. 리뷰에서 나온 것이고, `an_unknown_state_is_refused_only_after_the_version_is_checked`가 그 순서를 지켜요. 열거형으로 읽게 되돌린 코드에서 이 테스트가 409 대신 400을 받아 실패하는 것을 봤어요.
- `ArchiveSuggestion.state`와 `SubscriptionItem.state`는 서버가 보관한 규칙을 빼고 보내서, TypeScript에서 예전처럼 `"active" | "paused"`로 좁혔어요(`ts(type)`). 이것도 리뷰에서 나온 것이에요.
- 정해진 글자만 오는 필드가 아닌 것은 `string`으로 남겼어요. Anissia가 준 값을 그대로 보내는 `Anime.status`와 `ScheduleEntry.status`, 명령 종류마다 다른 `CommandOutcome.result`, 문장인 `reason`이에요. 예전 손 타입도 `string`이었어요.
- 오류 본문의 키 순서가 `current`, `error`, `message`에서 `error`, `message`, `current`로 바뀌었어요. JSON 객체로는 같고, 본문을 글자로 비교하는 테스트나 화면은 없어요.

### 빠진 필드와 `null`

`json!` 오류 본문은 `current`가 없으면 키를 빼고, 서버 값이 `null`일 때만 `null`을 넣었어요. `ErrorBody.current`는 `skip_serializing_if = "Option::is_none"`이라서 같게 써요. 이것은 두 가지로 확인했어요.

- `error_body_keeps_current_absent_or_null_as_before`가 `current`가 없으면 키가 없고, `Some(Value::Null)`이면 `null`이 있는 것을 확인해요.
- 지금 API 테스트는 고치지 않고 통과했어요. 바뀐 테스트 파일은 순서 테스트를 더한 `web/rules_api/tests.rs` 하나이고, 지운 단언은 없어요.

요청 본문에서 `serde(default)`인 선택 필드는 서버가 빠진 것과 `null`을 똑같이 받아서, `ts(optional = nullable)`로 `field?: T | null`로 생성해요.

### 화면

- 손으로 쓴 타입 선언 71개를 지웠어요. 그중 셋은 생성한 타입에서 만든 별칭으로 남겼어요. `ApiErrorCode`는 `ErrorCode`에 화면만 쓰는 `"network"`를 더하고, `AnimeOfFolder`는 `Anime`의 두 필드이고, `ScheduleLink`는 `LinkRuleBody`에서 `version`을 뺀 것이에요.
- `sendCommand`는 종류(`CommandKind`)에 맞는 본문만 받아요.
- `history/api.ts`의 `HISTORY_RESULTS`는 `satisfies readonly HistoryResult[]`로 생성한 유니언 밖의 글자를 막아요. 지난 회차 검색을 시작하는 본문은 손 타입 대신 `StartSearchBody`를 써요. 둘 다 리뷰에서 나온 것이에요.

### 확인

- 2026-10-21에 `cargo test --locked --workspace -j 4`가 통과 3,074개, 실패 0개, 건너뛴 것 11개였어요. 0151 뒤 3,069개에 `api_types` 테스트 4개와 순서 테스트 1개가 더해졌고, 건너뛴 것은 `write_committed_files` 하나가 늘었어요. 원본 로그는 이 PC의 `dev/local/measure/0157/`에 있어요.
- 같은 날 `cargo clippy --workspace --all-targets --all-features`에 경고가 없고, `cargo fmt --all --check`, 웹의 `npm test`(242개)와 `npm run typecheck`가 통과했어요.
- 생성하지 않은 변경은 이렇게 확인했어요. `SubscriptionItem.state`의 `ts(type)`을 지우고 생성하지 않은 채 테스트를 돌리자 `committed_files_are_current`가 `changed: SubscriptionItem.ts`를 대며 실패했어요.
- 독립 리뷰가 응답과 요청의 JSON 모양을 예전 코드와 맞춰 봤어요. 작은 지적 다섯 가지 중 넷은 위에 적은 대로 고쳤고, 키 순서는 위에 적었어요.
