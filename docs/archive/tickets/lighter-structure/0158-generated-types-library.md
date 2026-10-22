# 0158 라이브러리 화면이 쓰는 API의 타입을 생성해요

- 상태: 완료
- 출처: [구조 가볍게 하기 명세의 화면 타입 생성](lighter-structure.md#화면-타입-생성)
- 막는 티켓: [0157](0157-generated-types-collect.md)(생성 도구)

## 작업

[0157](0157-generated-types-collect.md)이 만든 생성 도구로, 라이브러리 화면이 쓰는 API의 타입을 생성해요. 라이브러리 화면은 작품 목록과 작품 상세, 시즌과 시즌 정보, 자막 후보, 표지, 회차 대응, 자막 제작자, 감시 폴더, 가져오기, 보관 파일 정리예요. 2026-10-21에 이 API들은 web 크레이트의 `Serialize` 타입 중 가장 많은 몫(작품 상세 하나에 31개)을 가졌어요.

0157이 정한 생성 방식, 정수와 문자열 리터럴의 규칙, `json!`을 구조체로 바꿀 때의 조건을 그대로 따라요.

## 완료 기준

- [x] 라이브러리 화면이 쓰는 API의 응답과 요청 본문은 모두 Rust 구조체나 열거형이고, 그 타입은 생성돼요. `json!`으로 만들던 응답도 구조체가 돼요.
- [x] 라이브러리 화면에서 손으로 쓴 같은 뜻의 타입은 지우고, 화면은 생성한 타입을 써요. 생성한 이름끼리 겹치지 않아요.
- [x] 지금 API 테스트가 고치지 않고 통과해요. 바꾼 `json!` 응답은 결과에 적어요.
- [x] 화면 테스트와 `npm run typecheck`가 통과해요.
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.

## 결과

2026-10-21에 [0157](0157-generated-types-collect.md)이 만든 생성 도구로 라이브러리 화면이 쓰는 API의 타입을 생성하고, 화면이 그 타입을 쓰게 했어요. 대상은 작품 목록, 작품 상세(저장 공간, 자막 순서, 보관본 정리 포함), 시즌과 시즌 정보, Anissia 연결과 자막 후보, 표지, 회차 대응, 자막 제작자, 감시 폴더, 가져오기, 영상 확인 요청의 API예요. 생성한 파일이 145개 늘어서 `web/src/lib/api-types/`에는 타입 파일 270개와 `index.ts`가 있어요. API의 경로와 JSON 모양은 그대로예요.

### 생성 범위

- `api_types.rs`의 `roots`에 이 API들의 응답과 요청 본문 45개를 올렸어요. 그 타입이 담은 규칙, 채널, 할 일의 받기 실패 같은 타입은 따라서 생성돼요.
- `trss-anilist`, `trss-jobs`, `trss-import`에 `ts` 기능을 더했어요. 0157의 세 크레이트처럼 `trss`의 dev-dependencies만 켜고, `cargo tree -p trss -e normal,build`에 ts-rs가 없어요.
- TypeScript 이름이 겹치는 타입에는 `ts(rename)`을 119곳 붙였어요. `WorkView`가 `LibraryWork`, `EntryView`가 `SeasonEntry`, 가져오기의 `PreviewRequest`가 `ImportPreviewBody`가 되는 식으로, 다른 API와 겹치는 Rust 이름에 화면 쪽 뜻을 더했어요.
- 같은 모양을 두 번 선언했던 표지·시즌 검색의 `SearchBody`(TypeScript에서 `AnilistSearchBody`), 표지·시즌의 `VersionBody`, `WatchFolderRef`, `OrderView`는 하나로 합쳤어요. 회차 대응 미리보기의 `ChoiceBody`는 라이브러리의 `Choice`로 바꿨어요. 같은 글자를 받고 모르는 필드를 거절하는 것도 같아요.

### 바꾼 응답과 요청 본문

이 API들에는 테스트 밖의 `json!` 응답이 없어서, 구조체로 바꾼 `json!` 응답은 없어요.

정해진 글자를 `&'static str`나 `String`으로 보내던 필드는 같은 글자로 직렬화되는 열거형이 됐어요. `enums_serialize_as_their_codes`가 열거형마다 예전 글자와 같은지 확인해요.

- 라이브러리와 공통: `SubtitleCoverage`, `SubtitleFormat`, `SortKey`, `FilterKey`, 표지의 `ArtworkMode`·`ArtworkSource`·`ImageFormat`·`ArtworkJobKind`·`ArtworkNoteCode`, 시즌의 `SeasonOrigin`·`SeasonNoteCode`, `UnrecognizedReason`, `MappingKind`, `MappingChoice`
- 자막 작업: `ItemState`, `ItemWait`, `StoredFormat`, `ApplyMode`, `AssetKind`, `CleanKind`, `CopyChoice`, `TodoKind`
- 가져오기: `SuggestionKind`
- `trss` 크레이트: `ImageStatus`, `EpisodeForm`, `ReferenceTitleKind`, `StorageKind`, `SeasonPending`, `FailureRole`, `FailureState`

`TodoKind`는 할 일의 표시를 새 `Todo::badge_kind()`로 만들고, 예전 `Todo::badge()`는 글자를 그대로 돌려줘서 그 테스트를 고치지 않았어요. 정렬과 거르기의 `SortKey`·`FilterKey`는 `Serialize`만 더했고, 질의 문자열은 예전처럼 `from_code`로 읽어요.

요청 본문의 몇 필드는 Rust에서 지금처럼 글자로 읽고 TypeScript에서만 좁혔어요(`ts(as)`). 자막 적용 본문의 `mode`와 자막 순서 본문의 `format_order`예요. 열거형으로 읽으면 모르는 값을 본문을 읽을 때 거절해서, 지금의 `BAD_MODE` 같은 거절과 그 순서가 바뀌기 때문이에요. 보관본 정리 상태의 `state`는 DB에서 오는 글자라서 `String`으로 두고, SQL이 거르는 `"asked" | "held"`로 좁혔어요(`ts(type)`).

### 빠진 필드와 `null`

회차의 `movie`는 `skip_serializing_if`인 `Option<Option<MovieRef>>`라서, 극장판 회차가 아니면 키가 없고, 연결하지 않은 극장판 회차면 `null`이에요. 그래서 `movie?: MovieRef | null`로 생성해요. 서버가 늘 보내는 `compare`, `subtitles`, `previous_episodes`, `season_episodes`는 손 타입에서 선택이었지만 생성한 타입에서는 필수예요. 요청 본문에서 `serde(default)`인 선택 필드는 0157처럼 `field?: T | null`이에요.

### 화면

- 라이브러리, 설정의 감시 폴더·가져오기·저장 공간, 할 일의 영상 확인 요청에서 손으로 쓴 타입 선언 84개를 지웠어요. 그중 `settings/import/types.ts`는 파일째 없어졌어요.
- 셋은 생성한 타입에서 만든 별칭으로 남겼어요. `AnissiaLink`는 극장판 연결을 시즌 0 연결로 들고 있어서 `movie`를 더했어요. `CandidateList`는 극장판 후보를 시즌 후보 모양으로 다시 만들어서 `movie`를 더했어요. `Ranked`는 후보의 두 필드만 골라요.
- `library/api.ts`, `storage.ts`, `subtitles.ts`, `upload.ts`는 생성한 이름을 다시 내보내서, 가져다 쓰는 파일의 경로를 바꾸지 않았어요. `Candidate`, `EpisodeFailure`, `MovieLinkAnswer`를 쓰던 곳은 생성한 이름인 `SubtitleCandidate`, `RevisionFailure`, `MovieLink`를 써요.
- 보관본 정리의 파일 종류 라벨에 `other: "기타"`를 더했어요. 생성한 `AssetKind`에 `other`가 있어서 `Record`가 요구했어요. `other` 파일은 계획에서 버려져 보관되지 않아서 정리 확인에 나오지 않고, 보이는 내용은 그대로예요.
- `subtitles.test.ts`의 픽스처 하나가 서버가 보낼 수 없는 형식 `"vtt"`를 썼어서, 서버가 보내는 `"other"`로 바꿨어요. 단언은 그대로예요.
- 0159의 화면에 속한 손 타입은 남겼어요. 정책 화면의 `SubtitleFormat`, 배치와 교체의 `format`, 할 일의 `FileFormat`, 가져오기의 일정 조회가 쓰는 편성표 응답이에요.

### 확인

- 2026-10-21에 `cargo test --locked --workspace -j 4`가 통과 3,144개, 실패 0개, 건너뛴 것 11개였고 테스트 바이너리의 시간 합은 44.15초였어요. 0163 뒤와 개수가 같은 것은 기존 테스트 `enums_serialize_as_their_codes`에 단언만 더했기 때문이에요. 원본 로그는 이 PC의 `dev/local/measure/0158/`에 있어요.
- 기존 Rust 테스트 파일은 고치지 않았어요. 바꾼 테스트는 위의 열거형 단언과 화면 픽스처 하나예요.
- 같은 날 `cargo clippy --workspace --all-targets --all-features`에 경고가 없고, `cargo fmt --all --check`, 웹의 `npm test`(285개)와 `npm run typecheck`가 통과했어요.

### 검토

독립 리뷰가 범위 안의 응답 글자, 키가 있고 없음과 `null`, 요청 본문이 받는 JSON과 거절 순서, 질의 문자열을 예전 코드와 맞춰 봤고 막는 지적은 없었어요. 작은 지적 넷은 이렇게 했어요.

- `기타` 라벨은 위에 적은 까닭으로 남겼어요.
- 극장판 후보 목록이 `previous_episodes`와 `season_episodes`를 키 없이가 아니라 `null`로 들고 있어요. 이 값을 읽는 회차 대응 조작은 극장판에서 그리지 않아서 보이는 차이가 없어요.
- `StorageKind::of`는 모르는 종류를 첨부로 읽어요. 지금은 `trss-jobs`의 저장 공간 SQL이 자막·폰트·첨부 셋만 내서 정확하지만, 종류가 늘면 컴파일러가 알려 주지 않아요. 지금 동작을 바꾸지 않아서 남겼어요.
- 위 픽스처를 바꾸면서, 타입 밖의 형식 글자를 받을 때의 대체 표시는 테스트가 거치지 않게 됐어요. 생성한 타입이 그런 글자를 막아요.
