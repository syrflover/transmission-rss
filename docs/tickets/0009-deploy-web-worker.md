# 0009 웹과 worker를 배포하고 cron을 걷어내요

- 상태: 진행 중 (서버 전환 끝, Transmission OOM은 `memory.high`로 피함, 완료 기준의 실제 확인 남음)
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
- 자원 한도는 컨테이너마다 0.25 CPU·128M으로 시작했어요. 근거와 재검토 계획은 [readme](../../readme.md#resource-limits)에 있어요. 작품 표지(0015)의 이미지 디코딩이 한 번에 최대 64 MiB를 쓰게 되어, 사용자가 2026-10-01에 `trss-web`과 `trss-worker`의 메모리 한도를 둘 다 256M으로 올리기로 결정했어요(Transmission 한도는 그대로예요). 합은 [0015 결과](0015-work-artwork.md)와 `src/artwork/mod.rs`에 있어요. 로컬 유휴 상태는 worker 3.4MiB·web 1.2MiB였어요(피드와 Transmission 없이, 실제 부하가 아님).
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

### 서버 전환 (2026-10-01, j4105, Transmission 4.1.1)

- cron의 마지막 실행은 00:05였어요. `cron.sh uninstall` 뒤 Transmission 토렌트 목록을 기준선으로 저장했어요(18개, 봇 라벨 16개). 저장소 폴더와 미디어 디스크는 모두 로컬 xfs예요.
- web을 켜고 기존 YAML을 가져왔어요: 채널 2개(erai-raws, SubsPlease), 규칙 26개, 저장 폴더 `/downloads/Shows (current)`.
- worker 첫 주기: `250 item(s) seen (250 new), 0 added, 17 already in Transmission, 0 failed, 217 without a rule, 16 excluded, 0 removed`.
  기준선과 견주면 빠진 토렌트·새 토렌트·폴더가 바뀐 토렌트가 없어, cron이 넣은 항목을 같은 폴더에서 모두 알아봤어요. 봇 토렌트 16개에 항목 라벨이 하나씩 붙었어요.
- **cron과 다른 점**: 릴리스 이름 그대로이던 봇 토렌트 11개(모두 받는 중·대기·멈춤, 메타데이터는 받음)의 이름을 worker가 규칙의 회차 보정대로 바꿨어요(예: `[SubsPlease] Re Zero ... - 84` → `Re Zero kara Hajimeru Isekai Seikatsu S04E18.mkv`, `episode: -66`). `Existing`이 앞선 실행이 끝내지 못한 이름 바꾸기를 마무리하는 명세대로의 동작이에요.
  cron이 남긴 릴리스 이름의 원인은 옛 바이너리가 아니라 아래의 Transmission 강제 종료로 보여요(바꾼 이름이 저장되기 전에 사라짐). trname은 이 이름들에 정상 이름을 만들고, 같은 Transmission에 직접 보낸 `torrent-rename-path`도 `success`였어요.
- Compose 프로젝트 이름을 `trss`로 둔 탓에 같은 폴더의 Transmission Compose(폴더 이름으로 `trss`)와 한 프로젝트가 되어, Transmission 컨테이너가 고아로 표시됐어요. `trss-app`으로 바꿨어요(`--remove-orphans`를 쓰면 Transmission이 지워질 수 있었어요).
- 웹만 재시작: `restart trss-web` 동안 worker의 시작 시각(15:23:10 UTC)이 그대로였고, 웹은 다시 200을 돌려줬어요.
- 한 번 받기: 명령 두 개(Re Zero S04E18, Link Click S3-08)가 `received`로 끝났고, Link Click은 규칙 폴더 `Link Click/Season 03`에 들어갔어요. 끝난 뒤 두 토렌트에 명령 라벨(`trss-cmd:`)이 남아 있지 않았어요. 추가할 때 라벨이 실제로 붙었는지는 보지 못했어요. Link Click은 확인용으로 받은 것이라 사용자가 뒤에 지웠어요.
- 자원: 전환 직후 worker 3.1MiB, web 1.1MiB였어요. 며칠 뒤 다시 봐요.

#### 이름과 라벨이 되돌아간 일: 호스트 커널의 OOM 강제 종료

- 첫 주기 직후에는 봇 토렌트 16개에 항목 라벨이 있고 10개가 새 이름이었는데, 15:40 UTC에는 항목 라벨이 2개(뒤에 새로 추가된 토렌트)만 남고 10개가 릴리스 이름으로 돌아가 있었어요. 뒤 주기들의 `Already` 줄도 릴리스 이름이었어요.
- 토렌트는 다시 추가되지 않았어요(`addedDate`가 cron 시각 그대로). 대신 Transmission id가 바뀌어 있었어요(`Mebius Dust` 17 → 18). 데몬이 다시 시작됐다는 뜻이에요.
- 원인: 커널이 Transmission 컨테이너의 메모리 cgroup에서 `transmission-daemon`을 OOM으로 죽이고 있었어요(`memory.events`의 `oom_kill` 463). 컨테이너 안의 s6가 데몬을 다시 띄워서 컨테이너는 계속 떠 있고, Docker도 OOM을 보고하지 않아요. 로그의 `Killed`는 2026-08-26부터 1204번이고, 전환 무렵에는 거의 1분에 한 번이었어요.
  죽을 때마다 resume 파일에 아직 쓰지 않은 이름·라벨·받기 진행률이 사라져요. 그래서 받는 중인 토렌트들이 몇 %에 머물러 있었어요. worker 코드와는 상관없고, 전환 전부터 있던 문제예요.
- 한도가 원인이 아니었어요. 한도를 1G로 올려도 1분마다 죽었어요. 죽는 순간 cgroup에는 데몬 메모리(`anon`)가 50MB 남짓, 파일 캐시 `inactive_file`이 980MB, 그중 쓰지 않은 것(`file_dirty`)이 143MB였어요. 비울 수 있는 캐시가 800MB 넘게 남은 채 OOM이 난 거예요. 회수 자체는 돌고 있었어요(`pgscan` ≈ `pgsteal`).
- 시점: 호스트(Rocky Linux 9)가 2026-08-26 21:27 KST에 커널 `5.14.0-687.41.1`로 재부팅했고, 약 2시간 뒤 처음 죽었어요. 그 전 `687.17.1`로 돈 기간(8월 11–26일)에는 기록이 없어요.
- `687.53.1`로 올린 직후 죽지 않은 것은 받는 토렌트가 없어서였어요. 2026-09-30에 봇 토렌트 5개를 받기 시작하자 512M에서 다시 죽었고(16:46–17:23 UTC에 9번, 짧게는 34초 간격), 죽는 순간의 모습(`anon` 50MB 안팎, `inactive_file` 420–470MB)도 같았어요. MGLRU(`/sys/kernel/mm/lru_gen/enabled` `0x0007`)를 꺼도 죽어서 원인에서 뺐어요.
- 원인: 커널 회수 추적(`vmscan` tracepoint)에서 죽기 직전 1ms 동안 데몬 스레드가 회수를 **16번 연달아 모두 성공**(`nr_scanned=64 nr_reclaimed=64`, 못 비운 이유 0)한 뒤 OOM을 냈어요. 58초 동안 3,667번의 회수 중 아무것도 못 비운 회수는 없었어요. 캐시를 못 비운 게 아니라, 비운 자리를 다른 쓰기가 먼저 가져가는 동안 재시도 한도 16번을 다 쓴 거예요.
  `687.41.1`부터의 RHEL 커널에는 RHEL 전용 변경 "mm/memcg: refactor try_charge_memcg retry logic to use for loop"(RHEL-211058)이 들어 있어요. upstream은 회수가 진전을 내는 재시도를 한도에 세지 않는데, 이 변경은 모든 재시도를 16번 안에 세요. Red Hat은 2026-09-21 CentOS Stream 9에서 이 변경을 되돌렸어요(RHEL-255363, "unexpected oom kills"). 같이 들어온 `vm.mem_cgroup_reclaim_retries`는 범위가 1–16이라 한도를 늘리는 데 쓸 수 없어요.
- 한도는 1G로 올렸다가 사용자 결정으로 512M로 되돌렸고([docker-compose.yml](../../docker-compose.yml)), 한도를 없애는 우회는 사용자가 택하지 않았어요.
- 조치: 이 버그는 `memory.max`에 부딪힌 할당에서만 동작해요. `memory.high`를 넘은 쪽의 회수(`mem_cgroup_handle_over_high`)는 회수하고 늦추기만 하고 OOM을 부르지 않아요. 그래서 캐시를 384M 아래로 붙잡아 512M 벽에 닿지 않게 해요. 커널을 17.1로 되돌리는 방법도 있었지만, 보안 수정을 잃지 않는 이 방법을 택했어요.
  2026-09-30 서버에서 실행 중인 컨테이너의 cgroup에 `memory.high` 384M를 직접 써서 시험했고, 받는 동안 잘 된다고 사용자가 확인했어요(카운터 값은 받지 않았어요). 저장소에는 컨테이너를 다시 만들어도 남도록 [transmission.slice](../../deploy/transmission.slice)(`MemoryHigh=384M`)와 compose의 `cgroup_parent`로 넣었어요. 되돌림이 들어간 커널이 나와도 남겨 둬도 괜찮아요.
  2026-10-01 서버에 slice를 설치하고 컨테이너를 다시 만든 뒤 `transmission.slice/memory.high`가 `402653184`(384M), 컨테이너의 `oom_kill`이 0이었어요(사용자 확인).
- 이름 바꾸기가 되돌아가는 동안 생길 수 있는 일: Transmission(4.1.1 `renamePath`)은 바꿀 이름의 파일이 이미 있으면 파일을 옮기지 않고도 성공으로 답하고 토렌트를 그 파일에 이어요. 강제 종료 사이에 버려진 새 이름의 `.part`가 있으면, 받은 조각 기록과 파일 내용이 어긋날 수 있어요. 그래서 받는 중인 봇 토렌트를 한 번 verify해요.

#### 강제 종료 기간 뒤 정리 (2026-10-01)

- verify: 전체 verify 뒤 조각이 깨진 봇 토렌트는 진행률이 내려간 채 멈춰 있었어요(Hanaori S01E12·Mebius Dust S01E12 52%, Slime S04E24 40%, Yani Neko S01E11 39%, Re Zero S04E19 1%). 다시 시작해 모두 받았고, 정리 시점에는 봇 토렌트 16개가 모두 100%였어요.
- "worker가 받는 중인 토렌트도 멈춘다"고 보였던 것은 worker가 아니었어요. worker의 `Stopped` 로그에는 다 받은 토렌트만 있었고, 받는 중이던 토렌트는 데몬이 강제 종료된 뒤 저장돼 있던 멈춤 상태로 돌아온 것이었어요(`startDate` 0).
- 빠진 회차: 일회성 스크립트로 규칙마다 nyaa의 릴리스와 폴더·Transmission을 견줬어요. 빠진 회차는 없었어요. Re Zero S4와 Slime S4의 `missing`은 같은 규칙에 걸리는 이전 시즌 릴리스였어요(음수 보정이 1 미만이면 적용되지 않아 번호가 그대로 남음). Yani Neko는 nyaa에서 찾지 못해 폴더로 확인했고, 비어 있던 11화는 기본 받기 폴더(`/downloads/downloads`)로 받아지고 있었어요. 같은 내용의 사본이 규칙 폴더에도 있었어요.
- `incomplete`에 남은 부분 파일 83개는 모두 어느 토렌트도 쓰지 않았어요. 받는 중인 토렌트가 없었고, 이름 바꾸기와 강제 종료가 되풀이되며 한 회차가 바뀐 이름과 릴리스 이름으로 두 벌씩 남은 경우가 많았어요. Yani Neko 폴더의 07–10화 릴리스 이름 사본과 07·09화 `.part`, 기본 받기 폴더의 11화 사본(모두 `cmp`로 같음)과 함께 `/data/storage1/media/.trash`로 옮겼어요(95G). 비우는 것은 사용자가 며칠 뒤 해요.
- worker는 피드에서 빠진 봇 토렌트를 다 받았는지 보지 않고 Transmission에서 뺐어요(데이터는 남김). 이번에는 그렇게 빠진 토렌트가 없었지만(`Removed` 0), Transmission이 오래 멈추면 덜 받은 회차가 주인 없는 `.part`로 남을 수 있어요.
  사용자 결정으로 다 받은 뒤에만 빼도록 바꿨어요([수집 명세](../specs/collection.md)). `0.4.1`부터 적용돼요.

### 0.4.1 배포 (2026-10-01, 사용자 승인)

- `0.4.1`(`09dae38`)에는 다 받은 뒤에만 빼는 정리, 탭 전환 때 화면이 흔들리지 않게 한 웹 수정, `한 번 받기`를 대신한 `다시 받기`와 `추가함` 표기가 들어 있어요. DB 형식 변경은 없어요.
- 서버에서 `TRSS_VERSION=0.4.1`로 받아 올렸고, 두 컨테이너가 `0.4.1` 이미지로 떴어요. 첫 주기는 오류 없이 끝났어요(항목 250, 새 항목 1, 추가 0, 이미 있음 17, 실패 0, 제외 14, 정리 0).
- 웹의 채널 탭에서 두 채널의 주소·기본 저장 폴더·제외 조건이 가져온 그대로 보였어요. 편성·라이브러리·할 일·구독 탭이 빈 것은 결과 목표 2–4가 아직 구현되지 않았기 때문이에요.

### 남은 일

- 며칠 동안 봇 토렌트 여럿을 받는 사이 `oom_kill`이 0에 머무는지(`transmission.slice` 아래 경로), 봇 토렌트의 이름·항목 라벨이 주기를 넘어 유지되는지 봐요. 되돌림(RHEL-255363)이 들어간 커널이 나오면 올려요.
- 완료 기준의 나머지 실제 확인: cron과 같은 결과를 주기 수·차이와 함께 기록, 실제 배포 경로의 보호 경계(`ss -ltn`, 휴대폰 LTE에서 공인 IP 접속 불가), 휴대폰에서 0003·0006 수행, `.torrent` 추가 응답 시간, 자원 한도 재검토.
- 되돌리기가 필요 없어지면 ENTRYPOINT를 정하고 `cron.sh`와 `legacy` 서비스를 걷어내요.
