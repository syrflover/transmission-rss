# 0081 실제 서버를 새 릴리스로 올리고 1000:1000으로 바꿔요

- 상태: 대기
- 출처: [구현 경계와 실행 순서](../specs/web-app.md#구현-경계와-실행-순서), [실제 재생 환경](../specs/subtitles.md#실제-재생-환경), [0062](0062-run-as-media-user.md)의 남은 일
- 막는 티켓: [0079](0079-musl-test-run.md)(musl 테스트), [0080](0080-server-db-migration-check.md)(마이그레이션), [0086](0086-feed-redirect-referer.md)–[0089](0089-package-season-reason.md)와 [0091](0091-trname-long-and-half-episodes.md)(배포 전에 고칠 결함), [0116](0116-allocator-measurement.md)(메모리 할당기), [0060](0060-server-filesystem-probe.md)(서버 미디어 디스크의 파일 쓰기)

## 작업

실제 서버(j4105, Rocky Linux 9)는 0.5.0을 root로 돌리고 있어요. 목표 3·4의 자막 기능, 서버 브라우저 컨테이너, 1000:1000 실행은 아직 올라가지 않았어요.
지금 코드를 새 릴리스로 올리고, readme의 절차대로 실행 사용자를 1000:1000으로 바꿔요.
이 배포에서 확인한 동작이 뒤의 리팩터링이 동작을 바꾸지 않았는지 견줄 기준선이 돼요.

- 0060의 탐침은 이 배포가 미디어 디스크에 쓰기 전에 그 디스크의 파일 동작을 확인하므로 막는 티켓이에요. [0065](0065-archive-extraction.md)의 압축 해제 측정과 [0061](0061-infuse-placement-check.md)의 Infuse 확인은 막지 않아요. 다만 0061에서 Infuse가 `.trss/` 안의 자막을 보여주면 배치를 다시 정해야 하므로, 그 결과가 먼저 나오면 이 배포 전에 반영해요.
- 릴리스 태그를 올리면 [배포 workflow](../../.github/workflows/deploy.yml)가 앱 이미지와 브라우저 이미지를 게시해요. 태그와 게시는 원격 쓰기라서, 버전 번호와 함께 사용자가 승인한 때만 해요.
- 서버 작업은 사용자가 실행하고, 명령과 확인할 출력은 이쪽에서 드려요. 서버 명령은 서버 터미널에서 실행한다고 명령 위에 적어요.
- 서버의 Docker Engine이 28 이상이고 Compose가 2.33 이상인지 먼저 봐요. 웹의 기본 경로를 정하는 `gw_priority`가 그 버전부터 있어요.
- 전환 순서는 readme의 [Running as 1000:1000](../../readme.md#running-as-10001000)이에요. 새로 생기는 설정(`TRSS_BROWSER_TOKEN`, `TRSS_WEB_HOST_IP`)과 `browser-downloads` 폴더를 먼저 준비해요.
- [0009](../archive/tickets/1-app-owned-collection/0009-deploy-web-worker.md)처럼, 전환 전에 Transmission의 토렌트 목록을 저장해 두고 전환 뒤와 견줘요.
- 이 배포에서 알게 된 것은 readme의 운영 안내에 고쳐 넣어요.

## 완료 기준

- 서버에서 `trss-web`, `trss-worker`, `trss-browser`가 새 릴리스로 돌아요. `docker inspect`의 `Config.User`가 web과 worker 모두 `1000:1000`이에요.
- 두 로그에 `cannot write the app data folder`나 브라우저 폴더의 오류가 없어요.
- DB가 최신 마이그레이션까지 갔고, 0080에서 견준 행들이 남아 있어요.
- worker의 첫 주기 뒤 Transmission의 토렌트 목록을 전환 전과 견줘요. 빠진 토렌트, 폴더가 바뀐 토렌트, 의도하지 않은 새 토렌트가 없어요. 견준 결과가 결과 절에 있어요.
- 웹의 라이브러리, 수집, 할 일 화면이 실제 데이터를 보여주고, 수집 상태판에 멈춤 경고가 없어요.
- readme의 운영 안내가 이번 배포의 실제 순서와 맞아요.
