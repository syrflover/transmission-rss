# 0029 크레이트 지도를 정하고 workspace의 바이너리와 공통 기반을 나눠요

- 상태: 대기
- 출처: [기능별 크레이트 ADR](../adr/0011-feature-crate-workspace.md), [공통 라이브러리의 모듈 구성](../specs/web-app.md#공통-라이브러리의-모듈-구성)
- 막는 티켓: 없음 (0027·0028처럼 진행 중인 변경이 있으면 먼저 끝내요. 파일을 대량으로 옮기는 동안 다른 변경과 충돌하기 때문이에요)

## 작업

동작을 바꾸지 않고 저장소를 Cargo workspace로 바꾸는 첫 단계예요. 넓은 이동이라 확장·축소 순서로 진행해요.

1. 크레이트 지도의 모양은 [ADR 0011](../adr/0011-feature-crate-workspace.md)에 정했어요(사용자 결정, 2026-10-02). 순환하는 모듈을 한 묶음으로 두는 크레이트 구성, 마이그레이션 SQL과 순서는 모두 `trss-core`, 폴더는 `crates/<이름>/`이에요.
   이 티켓에서는 모듈마다 갈 크레이트를 표로 정하고 결과에 남겨요. 그 원칙을 바꿔야 하는 배치(예: 묶음을 더 나누거나 합침)가 나오면 그때만 사용자에게 물어요.
   2026-10-02에 `crate::` 참조로 센 역방향 의존이에요(시험 코드 제외). 이동 때 다시 확인해요.
   - 기능 → worker: `anissia`·`artwork`·`seasons`(`Clock`·`CycleLock`), `archive_suggestions`·`subscriptions`·`store/channels`(`plan::ChannelPlan`), `past_search`(`feed`·`plan`·`revisions`), `episode_offset`(`revisions`·`season_link`), `store/history`·`store/revisions`·`transmission`(`revisions`), `store/channels`(`commands::episode_undo`), `artwork`(규칙 보관의 `rename_noreplace`), `store/library`(문서 링크만: `commands::rule_archive`·`live`)
   - 기능 → 웹: `schedule`(`web::schedule_api`)
   - 묶음 사이의 순환: `import` ↔ `store/channels`(`import_subscriptions`)
2. 지금의 라이브러리를 workspace의 한 크레이트로 두고, `trss-web`·`trss-worker`를 각자의 바이너리 크레이트로 옮겨요. `src/web`·`src/worker`처럼 진입부에만 쓰이는 모듈은 해당 바이너리 크레이트로 가요.
3. 공통 기반 크레이트를 떼어 내고, 지금의 라이브러리가 그것을 쓰게 해요.

Docker 이미지 빌드, 테스트 위치(`tests/`), 배포 스크립트가 새 구성으로 동작해야 해요.

## 완료 기준

| 확인 | 기대 결과 |
| --- | --- |
| 크레이트 지도 | 크레이트마다 책임·의존하는 크레이트·옮길 모듈과 역방향 의존 목록이 이 티켓의 결과에 있고, ADR 0011의 원칙과 어긋나는 배치는 사용자가 정했어요. |
| `cargo build`·`cargo test`·`cargo clippy` (workspace 전체) | 이동 전과 같은 테스트가 모두 통과하고, 빠진 테스트가 없어요(이동 전후 테스트 수를 비교해요). |
| 공통 기반 크레이트 | 웹·worker·기능 코드를 참조하지 않아요(크레이트 의존 목록으로 확인). |
| 앱 이미지 빌드와 로컬 compose 실행 | 두 바이너리가 이전과 같이 시작해 같은 DB를 읽고, worker의 수집 주기와 웹 화면이 동작해요. |
| 동작 | 마이그레이션 순서·DB 스키마·API 응답이 바뀌지 않아요. |
