# 0145 `trss-web`과 `trss-worker` 크레이트를 `trss` 크레이트 하나로 합쳐요

- 상태: 대기
- 출처: [구조 가볍게 하기 명세의 한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름), [한 프로세스 ADR](../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0144](0144-lighter-structure-baseline.md)(기준값)

## 작업

web과 worker를 한 프로세스로 합치는 첫 단계로, 두 크레이트의 코드와 테스트를 `trss` 크레이트 하나로 옮겨요. 이 티켓에서는 바이너리가 아직 `trss-web`과 `trss-worker` 둘이고, 이미지와 Compose의 서비스도 그대로예요. 실행을 하나로 합치는 일은 [0146](0146-one-trss-process-and-container.md)이 해요. 이렇게 나누면 코드를 옮기는 큰 변경이 동작을 바꾸지 않은 채 따로 들어가고, 프로세스를 합치는 변경은 실행 쪽만 다뤄요.

- 크레이트 이름은 `trss`예요(사용자 결정, 2026-10-21). 기능 크레이트는 그대로예요.
- web의 모듈과 worker의 모듈은 이름이 겹치지 않게 한 크레이트 안에 두고, 두 바이너리는 같은 라이브러리를 써요.
- `trss-worker`의 `tests/it`은 `trss`의 `tests/it`이 되고, web의 메모리 최고치 테스트 3개와 `remote_screen_docker`는 자기 바이너리를 그대로 가져요. 테스트를 테스트 나눔 ADR에 맞추는 일은 [0150](0150-trss-crate-tests-by-adr-0015.md)이 해요.
- 지금 문서는 `trss-web`이나 `trss-worker`의 경로로 코드와 테스트를 가리켜요. 2026-10-21에 명세 6개와 readme에서 두 이름이 나온 줄은 228줄이었어요. 완료한 티켓, 보관한 티켓, ADR은 그때의 기록이라 고치지 않아요.

## 완료 기준

- [ ] `trss-web`과 `trss-worker` 크레이트가 없고, 두 크레이트의 코드는 `trss` 크레이트에 있어요. `trss` 크레이트가 `trss-web`과 `trss-worker` 바이너리를 만들고, 이미지와 Compose는 지금처럼 그 둘을 써요.
- [ ] 통과·건너뛴 테스트 수가 앞뒤로 같고, 결과에 그 수를 적어요.
- [ ] `docker compose`로 띄운 개발 환경이 지금처럼 돌아요. 구독 추가의 받기 하나를 실행해 확인하고 결과에 적어요.
- [ ] 같은 커밋에서, 지금 코드를 적는 문서가 옮긴 파일과 테스트를 새 경로로 가리켜요. 명세, 검증 표, readme, 개요, 용어집, 열린 티켓이 대상이에요. 결과에 찾은 방법과 고친 문서를 적어요.
- [ ] `dev/compose.sh`, `dev/measure/`의 스크립트, `deploy/probe.sh`, `Dockerfile`이 크레이트 경로를 쓰면 새 경로로 고쳐요.
- [ ] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.
