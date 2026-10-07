# 0097 Transmission 가짜 서버를 trss-collect의 테스트도 쓰게 옮겨요

- 상태: 대기
- 출처: [수집 받기 줄기의 재구성](refactoring.md#수집-받기-줄기의-재구성), [ADR 0015](../adr/0015-test-a-rule-once-in-its-crate.md)
- 막는 티켓: [0092](0092-refactor-baseline.md)(기준값)

## 작업

Transmission 가짜 서버는 trss-worker의 테스트 폴더(`tests/it/common`)에만 있어요. 2026-10-07에 worker 테스트 파일 18개 중 9개, 테스트 399개 중 330개가 썼어요.
그래서 Transmission이 끼는 수집 규칙은 지금 worker 테스트로만 확인할 수 있고, [0101](0101-receive-line-tests.md)에서 그 규칙 테스트를 trss-collect로 내릴 수 없어요.
가짜 서버를 trss-collect의 테스트도 쓸 수 있는 곳으로 먼저 옮겨요. 가짜 AniList와 가짜 Anissia가 각 클라이언트 크레이트에 `fake`로 있는 것이 앞선 예예요.
같은 폴더의 가짜 피드 서버와 가짜 nyaa도 수집 규칙 테스트가 쓰면 함께 옮겨요.

## 완료 기준

- worker 테스트가 옮긴 가짜 서버를 쓰고, 통과·건너뛴 테스트 수가 앞뒤로 같아요.
- trss-collect의 테스트 하나 이상이 옮긴 가짜 서버로 돌아요.
- 가짜 서버가 trss-web과 trss-worker의 제품 빌드에 들어가지 않아요.
