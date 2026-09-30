# 0009 웹과 worker를 배포하고 cron을 걷어내요

- 상태: 진행 중 (릴리스 끝, 서버 전환 대기)
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
- 0001–0007을 합친 트리의 `docker build`로 만든 이미지에서 `trss-worker`를 띄워, musl 바이너리가 빈 DB를 만들고 마이그레이션을 적용하고 잠금 파일을 만든 뒤 한 주기를 돌고 SIGTERM에 0으로 끝나는 것을 확인했어요(연결할 수 없는 Transmission 주소로). 실제 Transmission과 실제 피드는 이 티켓에서 확인해요.
- worker의 Transmission 요청 제한은 연결 5초·전체 30초예요. 실제 `.torrent` 주소 추가가 30초 안에 답하는지 관찰해요.
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

## 결과

### 정한 것 (사용자 결정)

- 웹 포트는 서버 LAN IP에만 바인드해요(`TRSS_WEB_HOST_IP`, 비어 있으면 Compose가 실행을 거부). 밖에서는 VPN으로 들어와요.
- `Cargo.lock`을 커밋하고 이미지를 `cargo build --locked`로 빌드해요.
- 릴리스는 `0.4.0`이에요.
- 서버 작업은 사용자가 실행하고, 명령과 확인할 출력은 이쪽에서 드려요.

### 로컬 준비

- `docker-compose.trss.yml`에 상시 서비스 `trss-worker`·`trss-web`을 두었어요. 둘 다 `ghcr.io/syrflover/transmission-rss:${TRSS_VERSION}`(같은 릴리스), `restart: unless-stopped`, 로그 회전(10m×3)이고, 실행 파일은 `entrypoint`로 명시해요.
  DB는 `TRSS_DATA_DIR`(기본 `./data`)의 `/data/trss.db`예요. 미디어는 Transmission과 같은 `/downloads`에 읽기 전용으로 붙여, 폴더 링크 검사와 Transmission `downloadDir` 비교가 같은 경로 표기를 봐요.
- 옛 cron 서비스 `trss`는 되돌리기용으로 `legacy` 프로필에 남겼고, `scripts/cron.sh`가 `--profile legacy`로 불러요. `CHANNELS_CONFIG_URL`은 전환 뒤 `.env`에서 빠져도 파일 전체가 풀리도록 필수에서 뺐어요. ENTRYPOINT(`transmission-rss`)는 되돌리기가 필요 없어질 때 다시 정해요.
- 자원 한도는 컨테이너마다 0.25 CPU·128M으로 시작해요. 근거와 재검토 계획은 [readme](../../readme.md#resource-limits)에 있어요. 로컬 유휴 상태는 worker 3.4MiB·web 1.2MiB였어요(피드와 Transmission 없이, 실제 부하가 아님).
- 운영 안내([readme](../../readme.md))에 설정, 실행·중지·업데이트·로그·백업, cron에서 옮기는 순서(토렌트 목록 저장 → cron 해제 → web → 가져오기 → worker), 되돌리기를 적었어요.

### 로컬 확인 (2026-09-30, `--locked`로 빌드한 이미지, 연결할 수 없는 Transmission 주소)

- 두 컨테이너가 뜨고, 데이터 폴더에 `trss.db`와 `trss.db.worker.lock`이 생기고, worker가 빈 주기를 한 번 돌았어요.
- 웹은 `127.0.0.1:18080`에서 200을 돌려주고, 같은 호스트의 LAN IP로는 연결되지 않았어요(로컬에서 바인드 주소만 확인한 것이고, 실제 배포 경로의 보호 경계 확인은 아래 남은 일이에요).
- `restart trss-web` 동안 worker 컨테이너의 시작 시각이 그대로였어요.
- `down` 뒤 `up`으로 다시 만들어도 API로 만든 채널이 남았고, worker는 DB의 주기 기록을 보고 "최근에 돈 주기"로 건너뛰었어요.
- `stop`에 두 컨테이너 모두 종료 코드 0으로 곧바로 끝났어요.

### 릴리스 (2026-09-30, 사용자 승인)

- `feat/app-owned-collection`을 master에 병합 커밋으로 합쳐 push했고, `0.4.0` 태그로 Deploy workflow가 `ghcr.io/syrflover/transmission-rss:0.4.0`과 `:latest`를 게시했어요(amd64).
  서버의 cron은 이미지를 새로 받지 않고, 받더라도 ENTRYPOINT가 옛 바이너리라 전환 전까지 동작이 같아요.

### 남은 일

- 서버 전환과 완료 기준의 실제 확인: cron과 같은 결과 관찰(주기 수·차이 기록), 실제 배포 경로의 보호 경계, 웹만 재시작, 휴대폰에서 0003·0006 수행, `.torrent` 추가 응답 시간, 한 번 받기의 명령 라벨이 실제 Transmission에서 붙었다 떨어지는지(0008의 전제), 자원 한도 재검토.
- 되돌리기가 필요 없어지면 ENTRYPOINT를 정하고 `cron.sh`와 `legacy` 서비스를 걷어내요.
