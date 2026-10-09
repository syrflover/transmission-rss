# 0113 라이브러리, 설정, 웹 앱 공통 영역의 테스트를 나누고 검증 표를 둬요

- 상태: 대기
- 출처: [그 밖의 테스트 정리](refactoring.md#그-밖의-테스트-정리), [영역별 검증 표](refactoring.md#영역별-검증-표), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0094](0094-file-identity-in-core.md), [0095](0095-episode-text-key-in-core.md), [0096](0096-file-and-path-helpers.md), [0102](0102-request-pace-in-core.md), [0103](0103-queue-loop-in-core.md), [0104](0104-durable-file-writes-in-core.md), [0111](0111-web-and-screen-rules.md), [0112](0112-small-helpers-and-test-helpers.md)

## 작업

수집 받기 줄기([0101](0101-receive-line-tests.md))와 자막 작업 줄기([0110](0110-job-line-tests.md)) 밖의 영역도 ADR 0015대로 테스트를 나눠요.

- trss-web의 API 테스트 461개 중 70–110개가 다른 크레이트의 규칙을 다시 확인한다고 2026-10-07에 추정했어요. 0101과 0110에서 나누지 않은 것을 여기서 나눠요. 규칙의 주인 크레이트에 같은 경우가 있으면 지우고, 없으면 내려요.
- trss-web의 `folders_on_different_filesystems_are_refused`는 `/proc`를 훑어 혼자 4.9초가 걸리고, trss-collect의 같은 이름 테스트와 같은 규칙을 확인해요. 웹 쪽을 정리해요.
- 0101에서 trss-web의 보관 제안, 수집 이력, 규칙, 제목 후보, `in_place` 테스트를 나눴어요. 규칙 미리보기의 분류 테스트는 [0111](0111-web-and-screen-rules.md)에서 `build_preview`와 함께 내려요.
- worker의 수집 영역 테스트 중 0101의 다섯 파일 밖에 있는 것도 같은 방식으로 나눠요. 2026-10-09 `a4bb97c`에서 `collect_folder.rs` 3개, `first_read.rs` 8개, `title_waiting.rs` 3개, `past_search.rs` 32개였어요.
- trss-anilist와 trss-anissia의 `Retry-After` 클라이언트 테스트는 [0102](0102-request-pace-in-core.md) 뒤로 trss-core `response::retry_after`의 테스트와 같은 값을 확인해요. 클라이언트에는 헤더를 읽어 넘기는 연결만 남겨요.
- worker의 `app_data_folders_cover_the_receive_area_the_artwork_and_the_subtitle_files`는 [0096](0096-file-and-path-helpers.md)에서 두 목록이 trss-core의 같은 상수를 쓰게 되어 실패할 수 없어요. 지워요.

[라이브러리와 작품](../specs/library.md), [설정과 이전](../specs/settings.md), [웹 앱 공통](../specs/web-app.md) 명세에 요구별 검증 표를 둬요.

## 완료 기준

- 결과 절에 파일마다 앞뒤 테스트 수와, 지운 것·내린 것·남긴 것의 개수가 있어요. 지운 테스트마다 같은 경우를 확인하는 남은 테스트를 찾을 수 있어요.
- workspace 테스트가 통과해요.
- 세 명세에 검증 표가 있어요. 여섯 명세 모두에 표가 있는지 확인해요.
