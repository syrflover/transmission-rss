# 0009 웹과 worker를 배포하고 cron을 걷어내요

- 상태: 대기
- 출처: [구현 경계와 실행 순서](../specs/web-app.md#구현-경계와-실행-순서), [접근 경계와 기기](../specs/web-app.md#접근-경계와-기기)
- 막는 티켓: [0004](0004-worker-collection-history.md), [0005](0005-legacy-yaml-import.md)

## 작업

지금 배포는 [cron](../../scripts/cron.sh)이 5분마다 `docker compose run --rm`으로 종료형 컨테이너를 띄우고, [trss Compose](../../docker-compose.trss.yml)는 웹 포트·미디어 볼륨이 없으며 CPU 0.1·메모리 96M로 제한돼 있어요.
이를 같은 이미지·같은 릴리스 버전의 `trss-web`과 `trss-worker` 두 상시 컨테이너로 바꾸고, 기존 cron을 걷어내며, 운영 안내를 고쳐요.
사용자의 실제 서버에서 기존 YAML을 가져와 worker가 cron과 같은 결과를 내는 것을 확인하는 전환 티켓이에요.

- DB는 앱 호스트의 로컬 영속 볼륨에 두고, SMB·NFS 공유 경로에 두지 않아요.
- 웹 포트는 LAN·VPN·앞단 인증 뒤에만 열어요.
- 자원 한도는 상시 실행에 맞게 다시 정하고, 정한 값과 근거를 운영 안내에 적어요.
- 앞 티켓이 정한 실행 설정을 운영 안내와 Compose에 반영해요.
  DB 경로는 `TRSS_DB_PATH`, 웹은 `TRSS_WEB_BIND`·`TRSS_WEB_PORT`·`TRSS_WEB_STATIC_DIR`(이미지 기본값 `0.0.0.0`·`8080`·`/usr/local/share/trss/web`)이에요.
  worker는 `TRSS_DB_PATH`·`TRANSMISSION_URL`(필수)과 기존과 같은 이름의 속도 제한·큐·다운로드 폴더 변수, 주기 `TRSS_WORKER_INTERVAL_SECS`(기본 300)를 읽어요.
  worker 배타성은 DB 파일 옆 `<DB 경로>.worker.lock`의 파일 잠금이라, DB와 같은 로컬 볼륨에 둬야 해요.
  이미지의 ENTRYPOINT는 cron이 쓰는 `transmission-rss`라서, 웹·worker 컨테이너는 `trss-web`·`trss-worker`를 명시해 실행해요. cron을 걷어낼 때 ENTRYPOINT도 다시 정해요.
- 0001–0003을 합친 트리의 `docker build`에서 `rusqlite`의 `bundled` SQLite가 musl로 컴파일되는 것까지 확인했어요. musl 바이너리가 실제로 DB를 열고 쓰는 것은 worker가 생긴 뒤 이 티켓에서 확인해요.
- 전환 중 cron의 `transmission-rss`와 `trss-worker`를 함께 돌리면, 둘 다 자기 피드에 없는 trss 라벨 토렌트를 정리하므로 서로가 받은 토렌트를 지울 수 있어요.
  cron을 먼저 멈춘 뒤 worker를 켜거나, 두 쪽이 같은 채널 설정을 쓰는 동안만 겹치게 해요.
- `Cargo.lock`이 `.gitignore`에 있어 이미지 빌드마다 Rust 의존성 버전이 달라질 수 있어요. 상시 실행 배포 전에 커밋해 고정할지 정해요.
- 태그 기반 [배포 workflow](../../.github/workflows/deploy.yml)의 이미지 게시는 원격 쓰기이므로, 게시와 서버 반영은 사용자가 승인한 때만 해요.

## 완료 기준

- 운영 안내에 기존 cron 해제와 새 실행·중지·업데이트 절차가 있고, 그대로 따라 전환할 수 있어요.
- 새 구성에서 컨테이너를 다시 만들어도 DB의 채널·규칙·수집 이력이 남아요.
- 사용자의 기존 채널 설정을 0005로 가져온 뒤, 전환 전 cron 실행과 같은 RSS 시점에서 같은 항목이 같은 폴더로 들어가는 것을 한 주기 이상 관찰해요.
  관찰한 날짜·주기 수·차이를 이 티켓의 결과에 적어요.
- 웹만 재시작하는 동안 worker의 정기 처리가 계속돼요.
- 보호 경계 밖에서 웹 포트에 접근할 수 없는 것을 실제 배포 경로에서 확인해요.
  원격 인증 경로의 보호 확인은 결과 목표 3에서 해요.
- 실제 휴대폰에서 골격 화면(0003)과 규칙 편집(0006)을 한 번 수행하고 결과를 기록해요.
