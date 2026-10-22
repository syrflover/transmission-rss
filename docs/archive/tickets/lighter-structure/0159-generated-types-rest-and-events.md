# 0159 나머지 화면과 이벤트의 API 타입을 생성하고 손으로 쓴 타입을 없애요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 화면 타입 생성](lighter-structure.md#화면-타입-생성)
- 막는 티켓: [0158](0158-generated-types-library.md)(라이브러리 화면), [0155](0155-events-for-jobs-todos-candidates.md)와 [0156](0156-events-for-covers-schedule-status.md)(이벤트를 모두 만든 뒤)

## 작업

[0157](0157-generated-types-collect.md)과 [0158](0158-generated-types-library.md)이 수집 화면과 라이브러리 화면의 타입을 생성한 뒤, 이 티켓이 나머지 화면과 SSE 이벤트의 타입을 생성하고 화면 전체에서 손으로 쓴 같은 뜻의 타입을 없애요. 나머지 화면은 할 일, 자막 작업과 작업 상세, 교체와 배치, 원격 화면, 설정, 처음 실행, 자막 올리기예요.

이 티켓이 화면 타입 생성의 마지막 단계라서, 이벤트의 종류가 모두 생긴 뒤에 해요. 0157이 정한 생성 방식과 규칙을 그대로 따라요.

## 완료 기준

- [x] 나머지 화면이 쓰는 API의 응답과 요청 본문, SSE 이벤트는 모두 Rust 구조체나 열거형이고, 그 타입은 생성돼요. `json!`으로 만들던 응답도 구조체가 돼요.
- [x] API가 화면에 보내는 응답과 화면에서 받는 요청 본문 중 생성하지 않는 것이 없어요. 결과에 확인한 방법을 적어요.
- [x] 화면에 손으로 쓴 같은 뜻의 타입이 남지 않고, 화면은 생성한 타입을 써요. 생성한 이름끼리 겹치지 않아요.
- [x] 지금 API 테스트가 고치지 않고 통과해요. 바꾼 `json!` 응답은 결과에 적어요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-22에 [0157](0157-generated-types-collect.md)이 만든 생성 도구로 나머지 화면과 이벤트의 API 타입을 생성하고, 화면이 그 타입을 쓰게 했어요. 대상은 할 일, 자막 작업과 작업 상세, 교체와 배치, 원격 화면의 소켓 메시지, 설정의 정책과 수집, 처음 실행, 자막 올리기, 서버의 상태 확인, 이벤트 스트림의 이벤트예요. 생성한 파일이 115개 늘어서 `web/src/lib/api-types/`에는 타입 파일 385개와 `index.ts`가 있어요. API의 경로와 JSON 모양은 그대로예요. 구현은 `stora:task` 에이전트가 했어요.

### 생성 범위

- `api_types.rs`의 `roots`에 34개를 올렸어요. 그 타입이 담은 작업 단계, 할 일, 교체 비교의 줄 같은 타입은 따라서 생성돼요.
- TypeScript 이름이 겹치지 않게 `ts(rename)`을 78곳 붙였어요. 0158처럼 화면 쪽 뜻을 더한 이름이에요. 예를 들어 `JobRow`, `JobDetail`, `TodoWithBadge`, `ServerMessage`, `ClientMessage`, `Policy`, `CollectionSettings`예요.
- 원격 화면 소켓의 메시지(`ServerMessage`, `ClientMessage`)와 이벤트 스트림의 이벤트(`StreamEvent`)는 `Json`을 거치지 않아서 `roots`에 직접 올렸어요. 교체 비교의 줄 목록은 저장해 둔 JSON 글을 그대로 보내서, 그 모양(`ReplacementLines`)을 직접 올렸어요.
- 자막 올리기의 요청은 `multipart/form-data`라서 생성하지 않고 손 타입 `NewUpload`로 남겼어요. 쿼리 문자열, 줄 목록의 글 응답, 표지 이미지 바이트도 생성 대상이 아니에요.

### 생성하지 않은 것을 찾는 방법

새 테스트 `web::api_types::every_json_type_of_a_route_is_generated`가 `crates/trss/src/web/`의 핸들러가 `Json<…>`에 적은 타입을 모두 읽어요. 각 타입을 그 파일의 모듈이나 `use` 줄로 전체 경로까지 풀고, 생성이 닿은 타입의 전체 경로(`std::any::type_name`)에 없으면 그 파일과 경로를 대며 실패해요. 경로를 풀지 못한 이름도 실패로 쳐요. 라우트 모듈을 `api.rs`의 라우터에 합치지 않은 경우도 잡아요. `roots`에서 `settings_api::WriteBody`를 빼자 `settings_api.rs: Json<WriteBody> is trss::web::settings_api::WriteBody`를 대며 실패하는 것을 에이전트가 봤어요.

- 처음 만든 테스트는 짧은 이름으로 맞춰서, 같은 이름의 `policy_api::WriteBody`가 생성되면 `settings_api::WriteBody`를 빼도 통과했어요. 리뷰에서 나와서 전체 경로로 바꿨어요.
- 전체 경로로 바꾸자 명령 접수 본문 `commands_api::CreateBody`가 드러났어요. 이 본문은 [0157](0157-generated-types-collect.md#바꾼-응답과-요청-본문)이 `CommandRequest`로 생성하고 서버는 `Request::read`로 종류마다 골라 읽어서, 테스트의 예외 목록에 하나만 두었어요. 화면이 쓰는 본문과 서버가 읽는 본문이 맞는지는 `command_requests_are_the_ones_the_web_reads`가 확인해요.
- `Json`을 거치지 않는 소켓 메시지, 이벤트, 저장해 둔 JSON 글은 이 테스트가 보지 못해서 `roots`에 직접 올렸어요.

### 바꾼 응답과 요청 본문

`json!`으로 만들던 응답은 모두 구조체가 됐어요.

| 곳 | 구조체 |
| --- | --- |
| 자막 작업 만들기와 찾기 작업 만들기, 이미 있는 작업을 알리는 409 세 곳의 `current` | `JobCreated` |
| 작업 마치기, 다시 시작 | `FinishView`, `RestartView` |
| 배치 확인 | `ConfirmedView` |
| 교체 결정, 결과 | `DecideView`, `ResultsView` |
| 자막 올리기 | `UploadAnswer`(`UploadMade`와 `KeptCounts`, 또는 `JobCreated`) |
| 상태 확인 | `Health` |
| 이벤트 스트림 | `StreamEvent` |
| 원격 화면 소켓 | `Outgoing`(`screen_api.rs`, `hub.rs`, `nav.rs`), 화면 준비 요청의 `Asked` |
| 교체 비교의 줄 목록 | `ComparisonLines`(예전 `format!`과 같은 JSON 글) |

정해진 글자를 보내던 필드는 같은 글자로 직렬화되는 열거형이 됐어요. 새 열거형은 `StepShown`, `UnpackShown`, `Finishing`, `Restart`, `Scope`, `Warning`, `Limit`, `FormatLabel`, `Settled`, `Answered`, `ScreenShown`, `EndedReason`, `DialogKind`이고, `Assignment`, `FontReceipt`, `FileShown`, `FailureKind`, `Archive`, `Kind`, `Format`, `Encoding`, 교체 비교의 `Change`에는 직렬화와 타입 생성을 더했어요. 새 테스트 `job_and_screen_enums_serialize_as_the_strings_they_replaced`가 열거형마다 예전 글자와 같은지 확인해요.

요청 본문의 몇 필드는 Rust에서 지금처럼 글자로 읽고 TypeScript에서만 좁혔어요. 열거형으로 읽으면 모르는 값을 본문을 읽을 때 거절해서, 지금의 거절 문장과 그 순서가 바뀌기 때문이에요.

- 정책 저장 본문의 `format_order`(`ts(as)`, `SubtitleFormat` 목록)
- 교체 결정 본문의 `decision`(`ts(type)`, `"replace" | "keep"`)
- 원격 화면 입력의 마우스·터치·키 `event`와 마우스 `button`(`ts(type)`). 서버는 모르는 값을 조용히 버려요.

할 일의 `context`·`origin`, 할 일 목록의 `needs`·`badges`도 TypeScript에서 좁혔어요. `serde(default)`인 요청의 선택 필드는 `field?: T | null`이고, 값이 없으면 키를 빼는 응답 필드(`DialogueLine`, `TimingLine`)는 `field?: T`예요.

### 화면

- 손으로 쓴 타입 선언 83개를 지웠어요. `todo/api.ts`, `remote/protocol.ts`, `unpack.ts`, `settings/policy/api.ts`, `settings/collection/api.ts`, `useSuggestionLookup.ts`, `episodeLine.ts`, `fontReceipt.ts`, `library/detail/upload.ts`에서예요. `placementTypes.ts`와 `replacementTypes.ts`는 파일째 없어졌어요.
- 생성한 타입에서 만든 별칭으로 남긴 것이에요.
  - 할 일 종류별 타입 여섯(`AuthTodo` 등): Rust의 `Todo`가 `kind`로 나뉘는 열거형 하나라서 `Extract`로 꺼내요.
  - 원격 화면 입력의 이름과 버튼(`MouseEventName` 등): `ClientMessage`를 좁힌 것이에요.
  - `PolicyValues`: 정책 본문에서 `version`을 뺀 폼의 값이에요.
  - `ScheduledAnime`: 가져오기 검토가 편성표 항목에서 남기는 부분이에요.
  - `Drawn`(`JobSteps.tsx`): 작업 단계에서 화면에만 쓰는 부분이에요.
  - `EventName`(`events.ts`): 이벤트 이름에 브라우저의 `error`를 더한 것이에요.
- 자막 올리기의 응답이 두 모양의 합이 되어서, `UploadSection.tsx`는 `"kept" in result`와 `"dropped" in result`로 나눠 읽어요. 서버의 200·202 응답 모양과 같아요.
- 웹 테스트 6개(`changes`, `episodeLine`, `placementTable`, `relocation`, `replacementView`, `unpack`)는 타입을 가져오는 경로만 바뀌었어요.

원격 화면의 프레임 메시지 `Outgoing`은 base64 글을 복사하지 않고 빌려 써요. 예전 `json!`은 프레임마다 그 글을 한 번 복사했어요.

### 검토

`stora:reviewer`가 바꾼 응답, 409의 `current`, 원격 화면 메시지 여덟 가지, 이벤트를 예전 `json!`·`format!` 코드와 견주었어요. 리뷰어는 키, 값, `null`, 상태 코드, 거절 문장과 그 순서에서 다른 곳을 찾지 못했어요. 원격 화면 메시지는 예전 모양과 새 모양을 직렬화해 비교하는 임시 테스트를 돌렸어요. 지적 다섯 가지는 모두 고쳤어요.

- `trss-jobs`의 `ts` 기능이 `trss-core`와 `trss-subtitles`의 `ts`를 켜지 않아서, 그 크레이트만 `--features ts`로 빌드하면 실패했어요. 워크스페이스에서는 `trss`가 켜 주어서 드러나지 않았어요.
- 생성 범위 테스트가 짧은 이름으로 맞췄어요. 위에 적은 대로 고쳤어요.
- 이벤트 이름을 serde의 이름과 따로 손으로 적었어요. 이제 직렬화한 값에서 이름을 읽고, 테스트가 네 이벤트의 이름을 모두 확인해요.
- `Health.status`는 늘 `"ok"`라서 그 글자로 생성해요.
- 프레임 메시지가 base64 글을 프레임마다 복사했어요. 리뷰어가 release 빌드에서 240 KB 글로 프레임 하나를 만드는 시간을 측정했더니, 예전 `json!`이 약 271 µs, 복사하는 구조체가 약 317 µs, 빌리는 구조체가 약 112 µs였어요. 빌려 쓰게 고쳤어요.


### 확인

- 2026-10-22에 `cargo test --locked --workspace`가 통과 3,201개, 실패 0개, 건너뛴 것 12개였고, 빌드 뒤 다시 실행한 테스트 바이너리의 시간 합은 43.66초였어요. 0156 뒤의 3,199개에 생성 범위 테스트와 열거형 글자 테스트가 더해졌어요. 원본 로그는 이 PC의 `dev/local/measure/0159/`에 있어요.
- 같은 날 `cargo clippy --locked --workspace --all-targets --all-features`에 경고가 없고, `cargo fmt --all --check`, 웹의 `npm test`(306개)와 `npm run typecheck`가 통과했어요.
- `ts` 기능이 있는 일곱 크레이트(`trss-core`, `trss-subtitles`, `trss-anilist`, `trss-collect`, `trss-library`, `trss-jobs`, `trss-import`)가 저마다 `cargo check --locked -p <크레이트> --features ts`로 빌드돼요.

- 기존 Rust 테스트의 단언은 고치지 않았어요. `screen_api/nav.rs`의 테스트 모듈에는, 모듈이 더는 가져오지 않는 `json`을 쓰려고 `use serde_json::json;`만 더했어요.
- `TodoCount`의 Rust 문서에 한 줄을 더하고 생성하지 않은 채 테스트를 돌리자 `committed_files_are_current`가 `changed: TodoCount.ts`를 대며 실패했어요. 되돌리자 통과했어요.
- `cargo tree -p trss -e normal,build`에 ts-rs가 없어요.

### 동작 보존

경로, JSON, 상태 코드, 거절 문장과 그 순서는 그대로예요. 화면의 코드는 타입이 요구한 `UploadSection.tsx`의 나눠 읽기 말고는 타입만 바뀌었어요.
