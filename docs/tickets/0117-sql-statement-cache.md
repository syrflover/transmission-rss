# 0117 되풀이해 쓰는 SQL 문을 캐시해요

- 상태: 대기
- 출처: [SQL 문 캐시](refactoring.md#sql-문-캐시), [0116](../archive/tickets/5-deployed-verification/0116-allocator-measurement.md)의 측정
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

2026-10-07 [0116](../archive/tickets/5-deployed-verification/0116-allocator-measurement.md)의 측정에서, 운영 한도로 띄운 trss-web의 user CPU 표본 중 8–10%가 SQL을 해석하는 함수(`sqlite3RunParser`, `yy_reduce`, `sqlite3GetToken`, `keywordCode`)였어요.
같은 날 `tests/` 폴더와 `_tests.rs` 파일을 빼고 `prepare`를 부르는 곳이 175곳이었고, `prepare_cached`를 쓰는 곳은 없었어요.
프로세스마다 SQLite 연결이 하나이고 `Mutex` 뒤에 있으므로(trss-core의 `db.rs`), 그 연결의 문 캐시가 모든 요청에 쓰여요.

- 되풀이해 쓰는 SQL 문을 `prepare_cached`로 받아요. 한 번만 쓰는 문(마이그레이션, 가져오기 같은)은 그대로 둬요.
- rusqlite의 문 캐시는 기본 크기가 작으므로, 쓰는 문의 수에 맞춰 연결을 열 때 크기를 정해요.
- 동작을 바꾸지 않는 정리라서, 기존 테스트를 고치지 않고 통과시켜 확인해요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 테스트 | 기존 테스트가 고치지 않고 통과하고, 통과·건너뛴 수가 앞뒤로 같아요. |
| 스키마가 바뀐 뒤 | 마이그레이션이 테이블을 바꾼 뒤에 캐시한 문이 새 스키마로 다시 준비되는 것을 trss-core의 테스트가 확인해요. |
| web CPU | 0116과 같은 web 부하(개발 환경, 운영 한도, 서버 DB 복사본)에서 trss-web의 CPU 시간과 user 표본 중 SQL 해석 함수의 비중을 앞뒤로 견준 표가 결과 절에 있어요. |
| 테스트 시간 | 0092와 같은 명령과 빌드 상태로 측정한 workspace 테스트 시간을 앞뒤로 견줘 결과 절에 적어요. |
