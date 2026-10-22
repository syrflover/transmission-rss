# 0146 web과 worker를 `trss` 프로세스 하나와 컨테이너 하나로 돌려요

- 상태: 완료 (2026-10-21, 아래 "결과")
- 출처: [구조 가볍게 하기 명세의 한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름), [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0145](0145-merge-web-and-worker-crates.md)(크레이트 합치기)

## 작업

`trss` 바이너리 하나가 화면과 API 서버, 그리고 worker가 돌리던 일을 함께 돌리고, 앱 이미지와 Compose도 프로세스 하나에 맞춰요.

이 티켓은 실행 단위만 합쳐요. web 쪽이 명령을 적고 깨우기 소켓으로 알리면 worker 쪽이 worker 잠금 아래에서 집는 인계, 잠금들, 하트비트는 같은 프로세스 안에서 지금처럼 돌게 두고, [0147](0147-commands-start-at-once.md)과 [0148](0148-one-instance-lock-and-cycle-stall.md)이 없애요. 그래서 이 티켓 뒤에도 기능의 동작은 그대로이고, 바뀌는 것은 프로세스와 컨테이너의 수, 이름, 미디어 마운트, 메모리 한도예요.

- 합친 프로세스는 미디어를 쓰기로 마운트해요. 접근 경계인 LAN·VPN과 origin 검사는 그대로예요(사용자 결정, 2026-10-21).
- 메모리 한도는 지금 두 한도의 합인 384M(worker 256M, web 128M)로 시작해요. 합친 프로세스의 사용량을 서버에서 측정하는 일은 [0162](0162-deploy-lighter-structure.md)가 해요.
- 지금 서버는 `trss-web`과 `trss-worker` 컨테이너로 돌아요. 업데이트 절차가 두 컨테이너를 없애고 `trss` 하나를 띄워야 해요.
- [0135](../../../specs/2026-09-30-web-redesign/0135-deploy-settings-item.md)는 web이 자기 환경 변수만 알아서 Transmission 주소를 보여줄 수 없다고 적어요. 합친 뒤에는 한 프로세스가 둘 다 알므로 그 티켓의 문장을 고쳐요.

## 완료 기준

- [x] `trss` 크레이트의 바이너리는 `trss` 하나예요.
- [x] `trss` 프로세스 하나가 화면과 API, 5분 수집 주기, 감시 폴더 읽기(알림과 자동 감시), 명령, 자막 작업, 서버 브라우저의 수명, 보관 파일 정리를 돌려요. 개발 환경에서 구독 추가의 받기, 자막 작업 하나, 보관 파일 정리 하나를 실행해 확인하고 결과에 적어요.
- [x] 앱 이미지에는 `trss`, `trss-extract`, `trss-probe` 바이너리와 정적 화면이 들어가고, 이미지는 `trss`로 시작해요.
- [x] Compose의 `trss-web`·`trss-worker` 서비스는 `trss` 서비스 하나가 돼요. 이 서비스는 1000:1000으로 돌고, 미디어를 쓰기로 마운트하고, `trss_net`(`gw_priority`)과 `browser_net`에 붙어요. `stop_grace_period`는 15초, 메모리 한도는 384M이고, 환경 변수는 두 서비스가 받던 것을 합친 것이에요.
- [x] 시작할 때 web이 하던 서브넷 확인을 그대로 해요.
- [x] 0.7.0 이미지와 그때의 `docker-compose.trss.yml`로 로컬에 띄운 환경을 readme의 업데이트 절차대로 이 버전으로 올리면, `trss`와 `trss-browser` 컨테이너만 남고 데이터는 그대로예요. 해 본 명령과 결과를 적어요.
- [x] `dev/compose.sh`, `dev/measure/run.sh`, `deploy/probe.sh`가 `trss` 컨테이너와 그 한도·마운트를 써요.
- [x] 같은 커밋에서 다음 문서를 고쳐요.
  - [readme](../../../../readme.md)의 Docker Compose, 실행·중지·업데이트, 1000:1000 실행, 자원 한도, 개발 환경
  - [개요](../../../overview.md)의 구성 요소와 지금 상태를 보는 곳
  - [웹 앱 공통 명세의 구현 경계와 실행 순서](../../../requirements/web-app.md#구현-경계와-실행-순서)에서 프로세스와 컨테이너를 적은 부분
  - [자막 명세의 압축 해제의 격리와 한도](../../../requirements/subtitles.md#압축-해제의-격리와-한도)에서 worker 컨테이너의 한도를 적은 부분
  - [RSS 수집 명세](../../../requirements/collection.md), [설정과 이전 명세](../../../requirements/settings.md), [작업과 인증 명세](../../../requirements/jobs.md)에서 worker를 실행 주체로 적은 문장
  - 열린 티켓 [0058](../../specs/2026-10-04-server-resource-limits/0058-server-oom-and-resource-limits.md)의 한도 문장과 [0135](../../../specs/2026-09-30-web-redesign/0135-deploy-settings-item.md)의 환경 변수 문장
- [x] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.
- [x] 구현이 [한 프로세스 ADR](../../../adr/0019-one-process-for-web-and-worker.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.

## 결과

2026-10-21에 `trss` 크레이트의 두 바이너리를 `src/main.rs` 하나로 합쳤어요. 시작 코드는 두 바이너리의 것을 이어 붙였고, tokio 런타임 하나에서 웹을 먼저 띄운 뒤 worker 쪽을 준비해 돌려요. 그래서 worker 쪽이 서버 브라우저에 연결하는 동안에도 화면은 답해요.

- 설정을 먼저 모두 읽고(worker, 웹, AniList, Anissia, DB 경로), 그 뒤에 정적 화면과 앱 데이터 폴더를 확인해요. `TRANSMISSION_URL`이나 `TRSS_DB_PATH`가 없을 때의 오류는 지금처럼 그 이름을 알려요.
- 서버 브라우저가 있으면 서브넷을 찾고, 찾지 못하면 시작하지 않아요. 찾은 뒤에야 포트를 열어요. web이 하던 확인 그대로예요.
- SIGTERM이나 SIGINT를 받으면 취소 신호 하나로 웹은 연결 받기를 멈추고 worker 쪽은 하던 주기를 마무리해요. 서버 브라우저의 주소를 찾거나 브라우저에 연결하는 시작 도중에 받아도 곧바로 멈춰요. 웹이 혼자 끝나거나 패닉해도 나머지가 함께 멈춰요.
- 웹은 DB 연결을 따로 열어요. 두 프로세스였을 때처럼, 수집 주기가 오래 쓰는 동안에도 화면이 기다리지 않고 읽게 하려는 것이에요.
- web 쪽이 명령을 적고 깨우기 소켓으로 worker 쪽을 깨우는 인계, worker 잠금, 대기열마다의 잠금, 브라우저 잠금, 하트비트는 그대로예요. [0147](0147-commands-start-at-once.md)과 [0148](0148-one-instance-lock-and-cycle-stall.md)이 없애요.
- 로그 접두어는 `trss-web:`·`trss-worker:`에서 `trss:`로, 시작과 끝 줄은 `trss started`·`trss stopped`로 바꿨어요.
- CPU 한도는 두 컨테이너 한도의 합인 0.5로 했어요(구현 결정, 2026-10-21). 메모리 한도를 두 한도의 합으로 시작한 것과 같은 까닭이에요.

바이너리를 직접 띄우는 테스트 세 곳(`worker_process`, `receive_once`, `live_watch`)은 하네스의 `trss_process`로 띄워요. 이제 웹도 함께 뜨므로, 이 함수가 정적 화면 한 쪽과 빈 포트를 줘요. Compose 파일을 읽는 `trss-browser`의 테스트 3개는 `trss` 서비스를 보도록 고치며 이름에서 worker와 web을 뺐어요. 검증 표가 이름으로 가리키는 테스트는 아니에요.

### 테스트

`cargo test --locked --workspace -j 4`를 빌드한 뒤 실행하니 3,086개가 통과하고, 실패 0개, 건너뛴 것 14개였어요. [0145](0145-merge-web-and-worker-crates.md)와 같은 수예요. 테스트 바이너리는 30개에서 29개가 됐어요. 테스트가 없던 두 바이너리의 테스트 하네스가 하나로 합쳐졌기 때문이에요. 바이너리 시간의 합은 43.0초에서 43.2초였어요. 아래 리뷰에서 고친 뒤 다시 실행해도 통과 3,086개, 실패 0개, 건너뛴 것 14개였어요. 지우거나 옮긴 테스트는 없고, API의 경로와 JSON 모양도 바꾸지 않았어요. `cargo clippy --workspace --all-targets`에 경고가 없어요. 원본 로그는 이 PC의 `dev/local/measure/0146/`에 있어요.

### 개발 환경

`dev/compose.sh up`이 이 작업 트리로 이미지를 빌드해 `trss`, `trss-browser`, Transmission을 띄웠어요. 이미지에는 `trss`, `trss-extract`, `trss-probe`와 정적 화면이 있고, `trss`로 시작해요. `trss` 컨테이너는 1000:1000으로 돌았고, 메모리 한도 384M, CPU 0.5, `/data`와 `/downloads`를 쓰기로 붙였고, `trss_net`(`gw_priority` 1)과 `browser_net`에 붙었어요. 로그에는 서버 브라우저의 서브넷을 거절한다는 줄, 8080에서 듣는다는 줄, 서버 브라우저 풀, 수집 주기, 감시 폴더 읽기, 자막 다시 읽기와 그 작업, Anissia 관찰이 나왔어요.

- 구독 추가의 받기: 내장 브라우저에서 `편성표에서 추가`로 `명탐정 코난`을 subsplease.org 채널, 릴리스 제목 `Detective Conan`으로 구독하며 `[SubsPlease] Detective Conan - 1217 (1080p) [845D5162].mkv` 하나를 체크했어요. `receive_once` 명령이 한 번 시도해 17.0초 뒤 `done`으로 끝났고, 화면에는 `추가함`이 보였어요.
- 자막 작업: `All Works Maid` 시즌 1에 13화 ASS 하나를 든 ZIP을 올림 API로 올렸어요. `trss`가 `trss-extract`로 풀었고, 13화가 시즌의 1–12화 밖이라 배치 확인을 기다렸어요. 화면에서 `적용하지 않음`으로 적용하자, 작품 폴더의 `.trss/subtitles/제작자 알 수 없음/`에 1000:1000 소유로 보관하고 작업을 마쳤어요.
- 보관 파일 정리: 작품 상세의 `파일` 카드에서 그 보관본을 정리하자 `Stored files: cleanup … done`이 나왔고, 파일과 비게 된 제작자 폴더가 지워졌어요.

`dev/measure/run.sh`도 이 이미지로 측정 사본에서 돌렸어요. 웹은 0.77초에, 첫 주기는 2.29초에 끝났고, 웹 부하 30번에 실패가 없었고, 다시 읽기 30번이 모두 끝났으며, 최고 메모리는 22.6MiB, `oom_kill`은 0이었어요. 다만 이 PC의 `perf_event_paranoid`가 2라서 `perf record`가 열리지 않았고, 스크립트는 부하를 마친 뒤 perf를 멈추는 줄에서 끝났어요. 남은 두 줄(`memory.stat`과 로그 저장)은 같은 명령을 손으로 돌려 확인했어요.

`dev/compose.sh pull`이 서버에서 돌리는 스크립트는 ssh 대신 이 PC에서, 새 구성(`trss`)과 0.7.0 구성(`trss-worker`·`trss-web`)에 각각 돌렸어요. 둘 다 앱 컨테이너를 멈추고 `/data`를 복사한 뒤 다시 띄웠고, `data/`와 폴더·파일 이름 목록을 담은 tar를 냈어요. 리뷰 뒤에는 이 스크립트가 Compose 파일 대신 돌고 있는 컨테이너 이름으로 앱 컨테이너를 찾게 바꾸고, 새 구성과 앱 컨테이너가 없는 경우에 다시 돌렸어요. 새 구성에서는 같은 tar를 냈고, 컨테이너가 없으면 `no trss container is running on the server`로 끝났어요. `deploy/probe.sh`는 로컬 이미지의 `trss-probe`로 임시 폴더에서 돌렸고, 메모리 384m·CPU 0.5로 검사 48개가 모두 통과했어요.

확인을 마친 뒤 `dev/local`의 `data`, `media`, `transmission-config`를 확인 전에 떠 둔 사본으로 되돌렸어요.

### 0.7.0에서 올리기

0.7.0 이미지(`transmission-rss:0.7.0`, `trss-browser:0.7.0`)를 받고, 임시 폴더에 0.7.0 태그의 `docker-compose.trss.yml`과 `TRSS_VERSION=0.7.0`인 `.env`를 두고, 개발 데이터의 사본(스키마 버전 62, 0.7.0의 마이그레이션 수와 같아요)으로 띄웠어요. `trss-web`, `trss-worker`, `trss-browser`가 떴고 첫 주기가 끝났어요.

그 폴더에서 readme의 업데이트 절차대로 Compose 파일을 이 버전의 것으로 바꾸고 `TRSS_VERSION`을 로컬 이미지 태그로 바꾼 뒤 다음을 실행했어요. 로컬 이미지는 레지스트리에 없어서 `pull`은 건너뛰었어요.

```sh
docker compose -f docker-compose.trss.yml up -d --remove-orphans
```

`trss-web`과 `trss-worker`가 지워지고 `trss`와 `trss-browser`가 떴어요. 채널 2개, 규칙 27개, 작품 88개, 수집 기록 632개가 올리기 전과 같았고, 스키마 버전은 62에서 63이 됐으며, `/api/health`가 200으로 답했어요. 첫 주기는 0.7.0이 바로 전에 돈 주기 때문에 `A cycle started recently`로 건너뛰었어요. 지금의 시작 표시 규칙 그대로예요.

`--remove-orphans` 없이 `up -d`를 먼저 실행하면, 남은 `trss-web`이 포트를 쥐고 있어 `trss`가 뜨지 못했어요. 이때 만들어진 `trss` 컨테이너는 `trss_net`에만 붙어 있었고, 그 뒤의 `up -d --remove-orphans`는 이 컨테이너를 그대로 써서 `trss-browser`를 찾지 못한 채 재시작을 되풀이했어요. `--force-recreate`를 붙여 다시 만들자 두 네트워크에 붙어 돌았어요. readme의 업데이트 절차에 이 복구를 적었어요.

### 리뷰와 고친 것

`stora:reviewer`가 커밋 전의 변경을 리뷰했어요. 시작·멈춤 순서, 한 프로세스에 둘씩 생긴 Artwork·Anissia, Compose 파일, `dev/compose.sh`의 서버 스크립트, 문서가 맞는지를 봤고, 다음을 찾아 고쳤어요.

- 시작 도중의 SIGTERM을 무시했어요. 두 바이너리였을 때는 이때 신호 처리기가 없어서 곧바로 끝났는데, 합친 뒤에는 처리기가 먼저 생겨 취소만 하고 시작을 이어 갔어요. 리뷰어가 풀리지 않는 브라우저 주소로 재현하니 27.2초 뒤에 exit 1로 끝났어요. 주소 찾기와 브라우저 연결을 취소 신호와 겨루게 고친 뒤, 같은 재현은 곧바로 exit 0으로 끝났어요. 응답하지 않는 서버를 브라우저 주소로 준 연결 단계에서도 0.01초 만에 `trss stopped`로 끝났어요.
- 웹과 worker 쪽이 DB 연결 하나를 나눠 써서, 한쪽의 긴 쓰기가 다른 쪽의 읽기를 막을 수 있었어요. 웹이 연결을 따로 열게 했어요.
- 웹 작업이 패닉하면 나머지가 멈추지 않고 화면 없이 돌 수 있었어요. 취소를 `drop_guard`로 옮겨, 웹 작업이 끝나든 패닉하든 나머지를 멈추게 했어요.
- `settings_api.rs`·`import_api.rs`의 주석과 수집 명세 검증 표의 두 칸이 아직 웹의 읽기 전용 마운트를 적고 있어서 고쳤어요.
- 서버의 Compose 파일만 먼저 새 것으로 바뀌고 컨테이너는 아직 0.7.0이면 `dev/compose.sh pull`이 `docker cp`에서 실패했어요. 위처럼 컨테이너 이름으로 찾게 고쳤어요.

### 문서와 스크립트

`trss-web`, `trss-worker`, 실행 주체로 쓴 `worker`로 찾았어요. 찾은 범위는 readme, `web/README.md`, 개요, 명세 6개, 용어집, 열린 티켓이에요.

- readme는 티켓이 적은 절과 함께, 기능 절에서 `trss-worker`·`trss-web`을 실행 주체로 쓴 곳도 `trss`로 고쳤어요. `web/README.md`는 `cargo run --bin trss`와 이제 필요한 `TRANSMISSION_URL`을 적어요.
- 수집·설정·작업 명세에서 worker 혼자 실행 주체인 문장은 `trss`로 고쳤어요. web이 적고 worker가 집는 인계를 적은 문장은 같은 프로세스 안에서 지금도 맞아서 그대로 뒀고, 인계를 없애는 [0147](0147-commands-start-at-once.md)이 고쳐요.
- 수집 명세에서 web이 미디어를 읽기 전용으로 붙인다고 적은 두 곳은, 쓰는 것이 명령·자막 작업·보관 파일 정리의 실행 쪽뿐이고 API 처리기는 쓰지 않는다는 문장으로 바꿨어요.
- 날짜를 붙인 관찰과 완료한 티켓의 `trss-web`·`trss-worker`는 그대로예요.

### 확인하지 않은 것

- 서버(j4105)에서는 아무것도 하지 않았어요. 서버에 올리는 것은 [0162](0162-deploy-lighter-structure.md)이고, `dev/compose.sh pull`의 ssh 경로도 돌리지 않았어요.
- `dev/measure/run.sh`의 perf 기록은 이 PC의 `perf_event_paranoid`가 2라서 확인하지 못했어요(2026-10-21).
- musl 컨테이너의 테스트(`dev/musl-test.sh`)는 돌리지 않았어요. 이미지 빌드가 musl로 `trss`를 컴파일하고 그 이미지가 개발 환경에서 돈 것까지 확인했어요.
- `remote_screen_docker`의 무시된 테스트 4개와 원격 화면은 이번에 열지 않았어요.
- 웹이 패닉할 때 나머지가 멈추는 것은 테스트하지 않았어요.
- 개발 환경의 확인(구독 추가의 받기, 자막 작업, 보관 파일 정리, `run.sh`)과 0.7.0에서 올리기는 리뷰에서 고치기 전의 이미지로 했어요. 고친 뒤에는 워크스페이스 테스트와 위의 재현만 다시 돌렸어요.
- 컨테이너 이름으로 찾게 바꾼 `dev/compose.sh pull`의 서버 스크립트는 0.7.0 구성에서 다시 돌리지 않았어요. 0.7.0 이미지를 지운 뒤에 바꿨기 때문이에요.
