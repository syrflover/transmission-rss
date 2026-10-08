# 0081 실제 서버를 새 릴리스로 올리고 1000:1000으로 바꿔요

- 상태: 완료 (2026-10-07)
- 출처: [구현 경계와 실행 순서](../../../specs/web-app.md#구현-경계와-실행-순서), [실제 재생 환경](../../../specs/subtitles.md#실제-재생-환경), [0062](../4-store-apply-and-replace/0062-run-as-media-user.md)의 남은 일
- 막는 티켓: [0079](0079-musl-test-run.md)(musl 테스트), [0080](0080-server-db-migration-check.md)(마이그레이션), [0086](0086-feed-redirect-referer.md)–[0089](0089-package-season-reason.md)와 [0091](0091-trname-long-and-half-episodes.md)(배포 전에 고칠 결함), [0116](0116-allocator-measurement.md)(메모리 할당기), [0060](../4-store-apply-and-replace/0060-server-filesystem-probe.md)(서버 미디어 디스크의 파일 쓰기)

## 작업

실제 서버(j4105, Rocky Linux 9)는 0.5.0을 root로 돌리고 있어요. 목표 3·4의 자막 기능, 서버 브라우저 컨테이너, 1000:1000 실행은 아직 올라가지 않았어요.
지금 코드를 새 릴리스로 올리고, readme의 절차대로 실행 사용자를 1000:1000으로 바꿔요.
이 배포에서 확인한 동작이 뒤의 리팩터링이 동작을 바꾸지 않았는지 견줄 기준선이 돼요.

- 0060의 탐침은 이 배포가 미디어 디스크에 쓰기 전에 그 디스크의 파일 동작을 확인하므로 막는 티켓이에요. [0065](../4-store-apply-and-replace/0065-archive-extraction.md)의 압축 해제 측정과 [0061](../4-store-apply-and-replace/0061-infuse-placement-check.md)의 Infuse 확인은 막지 않아요. 다만 0061에서 Infuse가 `.trss/` 안의 자막을 보여주면 배치를 다시 정해야 하므로, 그 결과가 먼저 나오면 이 배포 전에 반영해요.
- 릴리스 태그를 올리면 [배포 workflow](../../../../.github/workflows/deploy.yml)가 앱 이미지와 브라우저 이미지를 게시해요. 태그와 게시는 원격 쓰기라서, 버전 번호와 함께 사용자가 승인한 때만 해요.
- 서버 작업은 사용자가 실행하고, 명령과 확인할 출력은 이쪽에서 드려요. 서버 명령은 서버 터미널에서 실행한다고 명령 위에 적어요.
- 서버의 Docker Engine이 28 이상이고 Compose가 2.33 이상인지 먼저 봐요. 웹의 기본 경로를 정하는 `gw_priority`가 그 버전부터 있어요.
- 전환 순서는 readme의 [Running as 1000:1000](../../../../readme.md#running-as-10001000)이에요. 새로 생기는 설정(`TRSS_BROWSER_TOKEN`, `TRSS_WEB_HOST_IP`)과 `browser-downloads` 폴더를 먼저 준비해요.
- [0009](../1-app-owned-collection/0009-deploy-web-worker.md)처럼, 전환 전에 Transmission의 토렌트 목록을 저장해 두고 전환 뒤와 견줘요.
- 이 배포에서 알게 된 것은 readme의 운영 안내에 고쳐 넣어요.

## 완료 기준

- 서버에서 `trss-web`, `trss-worker`, `trss-browser`가 새 릴리스로 돌아요. `docker inspect`의 `Config.User`가 web과 worker 모두 `1000:1000`이에요.
- 두 로그에 `cannot write the app data folder`나 브라우저 폴더의 오류가 없어요.
- DB가 최신 마이그레이션까지 갔고, 0080에서 견준 행들이 남아 있어요.
- worker의 첫 주기 뒤 Transmission의 토렌트 목록을 전환 전과 견줘요. 빠진 토렌트, 폴더가 바뀐 토렌트, 의도하지 않은 새 토렌트가 없어요. 견준 결과가 결과 절에 있어요.
- 웹의 라이브러리, 수집, 할 일 화면이 실제 데이터를 보여주고, 수집 상태판에 멈춤 경고가 없어요.
- readme의 운영 안내가 이번 배포의 실제 순서와 맞아요.

## 결과 (2026-10-07, j4105)

### 릴리스

- 버전은 0.6.0이에요(사용자 결정, 2026-10-07).
- `101f470`에서 `cargo test --workspace -j 4`를 glibc와 musl 대상으로 각각 돌렸어요. 둘 다 test binary 44개에서 2,756개가 통과했고, 실패는 0개, ignored는 14개였어요. 태그를 단 `78e8efd`와의 차이는 문서와 `deploy/torrent-list.py`뿐이에요.
- `62cd5ec`에서 앱 이미지와 브라우저 이미지를 개발 PC에서 먼저 빌드했어요. 둘 다 성공했어요. `78e8efd`와의 차이는 이미지에 들어가지 않는 `deploy/torrent-list.py`와 readme뿐이에요.
- master를 `78e8efd`까지 push하고 annotated tag `0.6.0`을 push했어요(사용자 승인, 2026-10-07). Deploy workflow가 성공해 `ghcr.io/syrflover/transmission-rss`와 `ghcr.io/syrflover/trss-browser`의 `0.6.0`과 `latest`를 게시했어요. 경고는 GitHub Actions의 Node.js 20 지원 종료 안내뿐이었어요.

### 사전 확인

사용자가 서버 터미널에서 실행했어요. 모두 읽기만 하는 명령이에요.

| 확인 | 결과 |
| --- | --- |
| 사용자 | uid 1000 |
| Docker | Engine 29.8.1([0060](../4-store-apply-and-replace/0060-server-filesystem-probe.md)), Compose v5.5.1. `gw_priority`가 필요로 하는 Engine 28·Compose 2.33 이상이에요 |
| 서버 저장소 | `4b37f5e`(0.5.0의 기록). 추적되지 않는 것은 `data/`와 0060의 탐침 파일뿐이라 fast-forward와 겹치지 않았어요 |
| `.env` | `TRSS_BROWSER_TOKEN`과 `TRSS_WEB_HOST_IP`가 있어요. `TRSS_DATA_DIR`는 없어서 기본값 `./data`예요 |
| 미디어 폴더 | 미디어 루트, `Shows`, `Shows (current)` 모두 1000:1000, `0775`예요 |
| 1000이 아닌 폴더 | 두 감시 폴더 아래의 모든 폴더 중 0개예요. 미디어 쪽은 `chown`할 것이 없었어요 |

0060이 받지 못한 감시 폴더의 소유와 모드를 여기서 확인했어요.

### 전환

readme의 [Running as 1000:1000](../../../../readme.md#running-as-10001000)를 따랐지만 실제 순서는 아래였어요. readme는 이 순서로 고쳤어요.

1. trss를 멈추고(`down`), 서버 저장소를 `git pull --ff-only`로 `78e8efd`까지 올렸어요. 이어서 Transmission의 토렌트 목록을 `deploy/torrent-list.py`로 저장했어요(2개). Transmission은 계속 돌았어요.
2. 앱 데이터 폴더를 `sudo cp -a data ~/trss-data-before-0.6.0`으로 복사했어요. 이어서 테이블마다 행 수를 기록했어요(`user_version` 29). 행 수는 `chown` 전에 읽었어요. root로 DB를 읽으면 SQLite가 `-shm`을 root 소유로 만들 수 있는데, 뒤의 `chown -R`이 그 파일까지 옮겨요.
3. `.env`의 `TRSS_VERSION`을 `0.6.0`으로 바꿨어요. `TRSS_BROWSER_TOKEN`의 길이는 64였어요.
4. `chown -R 1000:1000 data`를 실행하고, `data/browser-downloads`를 만들어 1000:1000, `1777`로 뒀어요. `data` 아래에 1000이 아닌 소유는 0개였어요. `data`는 `0755`, `browser-downloads`는 `drwxrwxrwt`였어요.
5. `pull`(앱 이미지 6.5초, 브라우저 이미지 45.8초)과 `up -d`를 실행했어요.

### 컨테이너와 로그

- `docker inspect`의 `Config.User`는 `trss-web`과 `trss-worker`가 `1000:1000`, `trss-browser`가 `browser`예요. 세 컨테이너 모두 `0.6.0` 이미지예요.
- 세 로그에 `cannot write the app data folder`, `browser-downloads`의 오류, panic이 없었어요.
- web은 서버 브라우저의 네트워크(`172.20.0.0/16`)를 거절한다고 알리고 떴어요. worker는 서버 브라우저 pool을 켰고, 브라우저 컨테이너의 launcher와 Xvfb가 떴어요.
- worker가 inotify 감시를 건 폴더는 `Shows (current)` 76개, `Shows` 154개였고, 걸지 못한 폴더는 없었어요.

### 첫 주기

- `Cycle finished: 250 item(s) seen (0 new), 0 added, 2 already in Transmission, 0 failed, 237 without a rule, 11 excluded, 0 removed`
- 감시 폴더 스캔은 `Shows (current)` 작품 30개, `Shows` 작품 58개였어요. 새 작품, 더한 파일, 지운 파일, 없어진 파일이 모두 0개였어요. 합한 88개는 [0080](0080-server-db-migration-check.md)의 복사본과 같아요.
- Anissia 자막 관찰이 5쪽을 읽어 87개를 더했어요.

### 토렌트 비교

첫 주기 뒤의 목록도 2개였고, 전환 전 목록과의 `diff`는 비었어요. 빠진 토렌트, 폴더가 바뀐 토렌트, 새 토렌트가 없어요. 두 목록은 토렌트 hash를 담고 있어서 서버에만 둬요.

### DB

전환 전과 첫 주기 뒤의 행 수를 견줬어요.

- `user_version`이 29에서 62가 됐어요.
- 0080에서 견준 채널, 규칙, 수집 이력, 감시 폴더, 작품, 시즌 연결을 포함해 원래 있던 테이블의 행 수가 모두 같아요. `media_files`만 3,566에서 3,567이 됐어요. 아래 자막 작업이 놓은 적용본을 스캔이 읽은 것이에요.
- 새 테이블 중 행이 생긴 것은 Anissia 관찰(`caption_observations` 87, `subtitle_sources` 87, `season_anissia` 1, `anissia_caption_poll` 1)과 아래 자막 작업의 기록(`subtitle_jobs` 1, `subtitle_job_steps` 5, `subtitle_job_events` 9 등)이에요. `policy_settings`, `work_subtitle_policy`, `unrecognized_checks`를 포함한 나머지는 0이에요.

### 자막 작업과 미디어 디스크

첫 주기에서 구독 제작자(코코렛)의 자막 작업 하나가 만들어져 `done (1/1)`로 끝났어요. `FX Senshi Kurumi-chan` 1화예요. 1000:1000으로 바뀐 worker가 실제 미디어 디스크에 처음 쓴 것이에요. 사용자가 웹의 할 일 화면에서 이 작업을 봤어요.

`torrents-before-0.6.0.tsv`보다 ctime이 늦은 것을 두 감시 폴더에서 찾았어요(`find -cnewer`, 21:06 KST). 아래 8개뿐이었어요.

| 종류 | 경로(작품 폴더 기준) | 소유 | 모드 |
| --- | --- | --- | --- |
| 폴더 | 작품 폴더, `Season 01` | 1000:1000 | `0775` |
| 파일 | `Season 01/FX Senshi Kurumi-chan S01E01.smi`(적용본) | 1000:1000 | `0644` |
| 폴더 | `.trss`, `.trss/tmp`, `.trss/subtitles`, `.trss/subtitles/코코렛` | 1000:1000 | `0755` |
| 파일 | `.trss/subtitles/코코렛/` 아래의 받은 이름 그대로인 `.smi`(보관본) | 1000:1000 | `0644` |

- 작품 폴더와 `Season 01`은 Transmission이 만든 원래 폴더예요. 안에 항목이 더해져서 ctime만 바뀌었어요.
- 빈 `.trss/tmp`는 [자막 명세](../../../specs/subtitles.md)가 임시 파일과 옮겨 둔 파일(`.aside`)을 두는 자리예요.
- 처음에는 mtime(`find -newer`)으로 찾았어요. 그러자 사용자가 예전에 넣은 `.smi`가 여럿 섞였어요. 그 파일들의 mtime은 2107-12-31 23:59로, ZIP의 DOS 시각이 나타낼 수 있는 마지막 날이에요. 그런데 ctime은 2026년 5–7월이고 모드는 `0664`예요. 배포와 상관없는 파일이에요. 그래서 디스크에서 바뀐 것은 ctime으로 견줘요.

### 웹 화면

사용자가 브라우저로 서버의 웹을 열어 라이브러리, 수집, 할 일 화면과 수집 상태판을 보고 이상이 없다고 했어요(2026-10-07). 할 일 화면에는 위 자막 작업이 끝난 채로 보였어요.

### 되돌릴 때 쓸 것

- `~/trss-data-before-0.6.0`: 전환 직전의 앱 데이터 폴더예요. 0.5.0이 root로 돌던 때의 소유 그대로예요.
- 서버의 `ghcr.io/syrflover/transmission-rss:0.5.0` 이미지와, 서버 저장소의 `4b37f5e` compose 파일이에요.
- 되돌리면 0.6.0이 DB에 쓴 것은 사라져요. 미디어 디스크에 놓은 위의 자막 파일은 남아요.

### 남은 것

- 위 자막 작업은 [0083](0083-deployed-end-to-end.md)의 "구독 제작자의 자막" 흐름에 해당할 수 있어요. 적용본과 보관본의 바이트가 같은지는 보지 않았어요.
- 서버의 `~/trss`에 이번 확인에서 만든 파일(`torrents-*.tsv`, `db-*.txt`, `db-counts.py`)과 0060의 탐침 파일이 남아 있어요. 모두 git이 추적하지 않아요.
