# 0060 실제 서버의 미디어 디스크에서 파일 쓰기를 확인해요

- 상태: 진행 중 (탐침과 실행 도구는 만들었어요. 서버에서 돌리는 일은 사용자가 해요. 아래 "실행 방법")
- 출처: [체크포인트와 중단 복구](../specs/jobs.md#체크포인트와-중단-복구)의 실제 파일시스템 검증, [보관본과 적용본](../specs/subtitles.md#보관본과-적용본)의 임시 파일 위치, [구현 경계](../specs/web-app.md#구현-경계와-실행-순서)의 실행 사용자
- 막는 티켓: 없음

## 작업

확인할 일이에요. 보관·적용이 기대는 파일 동작을 실제 서버(j4105)의 미디어 디스크에서 작은 탐침으로 확인해요. 실제 배포는 목표 5예요.

- 탐침은 worker와 같은 이미지, 같은 bind mount(`/downloads`)와 앱 데이터 폴더(`/data`), 같은 메모리 한도(128M)로 띄운 일회용 컨테이너에서 uid·gid 1000:1000으로 돌려요.
  운영 중인 작품 폴더가 아니라 미디어 디스크 안의 시험 폴더에 쓰고, 끝나면 지워요.
- 확인할 것이에요.
  - 시험 폴더가 있는 파일시스템의 종류(xfs로 알고 있어요)와 마운트 옵션
  - 같은 파일시스템에서 `renameat2(RENAME_NOREPLACE)`가 되고, 목적지가 있으면 `EEXIST`로 거절하는지
  - `/data`에서 `/downloads`로의 이름 바꾸기가 `EXDEV`로 실패하는지. 보관·적용의 임시 파일을 `.trss/` 안에 두는 까닭이에요.
  - 파일과 폴더의 `fsync`, 이름 바꾸기 뒤에도 `장치:inode`가 그대로인지
  - 1000:1000으로 만든 파일·폴더의 소유와 권한, Transmission이 만든 작품 폴더 안에 `.trss/`를 만들 수 있는지
- 128M memcg 안에서 수백 MiB 파일을 읽고 쓰는 동안 `memory.current`의 최대치, 페이지 캐시, `memory.events`의 `oom_kill`을 재요.
  [0058](0058-server-oom-and-resource-limits.md)의 커널 문제(memcg 재시도)가 worker에도 닿는지 보려는 거예요.
- 탐침은 저장소에 남겨 다시 돌릴 수 있게 해요. 서버에서 돌릴 명령은 사용자가 실행하고, 저는 결과를 받아 기록해요.

## 완료 기준

| 관찰 | 기대 결과 |
| --- | --- |
| 탐침 실행 기록 | 서버 커널, 파일시스템 종류, 마운트 옵션과 항목별 결과가 날짜와 함께 이 티켓에 있어요. |
| 이름 바꾸기 | 같은 파일시스템에서는 덮어쓰지 않는 이름 바꾸기가 되고 목적지가 있으면 거절돼요. 다른 파일시스템이면 실패해요. 결과가 다르면 임시 파일의 위치를 다시 정하고 명세를 고쳐요. |
| 1000:1000 | 만든 파일·폴더의 소유가 1000:1000이고, Transmission이 만든 작품 폴더에 `.trss/`를 만들 수 있어요. |
| 128M memcg | 큰 파일을 읽고 쓰는 동안의 최대 `memory.current`와 `oom_kill` 수가 있어요. `oom_kill`이 생기면 0058에 함께 적고 worker의 한도를 다시 정해요. |

## 만든 도구

- **탐침** `crates/trss-probe`는 `rustix`만 쓰는 정적 실행 파일(`trss-probe`)이에요. 앞으로 나올 릴리스의 이미지에는 `/usr/local/bin/trss-probe`로 들어가고, 지금 서버에 있는 0.5.0 이미지에는 없어서 아래처럼 바이너리를 따로 만들어 마운트해요.
- **실행 스크립트** `deploy/probe.sh`는 compose 폴더의 `.env`에서 `MEDIA_DIR`·`TRSS_DATA_DIR`·`TRSS_VERSION`·`TRSS_UID`·`TRSS_GID`를 읽어, 서버의 이미지로 일회용 컨테이너(`--user 1000:1000 --memory 128m --memory-swap 128m --cpus 0.25 --network none`)를 띄워요. worker와 같은 마운트(`/downloads`, `/data`)를 쓰고 탐침 바이너리는 읽기 전용으로 마운트해요.
- **빌드** `Dockerfile`의 `probe-binary` 대상이 탐침만 빌드해서 정적 실행 파일 하나를 내보내요.
- 탐침이 하는 일은 `trss-probe --help`와 모듈 설명에 있어요. 만든 폴더·파일은 모두 `.trss-probe-<pid>-<번호>`라는 이름이고 끝에 지워요. 그 이름이 아닌 것은 만들지도 지우지도 않아요. 작품 폴더(`--work`)에는 시험 하위 폴더 하나와 거기서 옮겨 올린 파일 하나만 만들고 지워요. 숨김 이름이라 라이브러리 검색(`discovery`)이 작품으로 읽지 않아요.

| 확인 | 탐침의 항목 |
| --- | --- |
| 파일시스템 종류·마운트 옵션 | `/proc/self/mountinfo`에서 `/downloads`·`/data`·작품 폴더가 놓인 마운트(종류, 장치, 마운트 옵션, 슈퍼블록 옵션)와 `st_dev`, 커널 버전 |
| 덮어쓰지 않는 이름 바꾸기 | 같은 마운트 안에서 `renameat2(RENAME_NOREPLACE)`가 되고, 있는 파일·폴더 위로는 `EEXIST`로 거절하며 아무것도 바꾸지 않는지(`/downloads`와 `/data` 각각) |
| `EXDEV` | `/data`의 파일을 `/downloads`로 옮기면 실패하는지. 두 폴더의 마운트 ID가 다르면 `EXDEV`가 나와야 해서 항목이 실패 판정이고, 같은 마운트면 결과만 알려 줘요. |
| `fsync`, `장치:inode` | 파일과 폴더의 `fsync`, 이름 바꾸기 앞뒤의 `장치:inode` |
| 소유와 권한 | 만든 파일·폴더의 소유가 `--owner`(기본 1000:1000)인지, 모드가 무엇인지 |
| 작품 폴더 | 주어진 작품 폴더 안에 시험 하위 폴더(`.trss/` 대신)를 만들고, 거기서 파일을 쓰고 `fsync`한 뒤 폴더 위로 `RENAME_NOREPLACE`로 옮겨 올릴 수 있는지, 소유는 무엇인지 |
| 128M memcg | 300MiB(`--size-mib`) 파일을 쓰고 `fsync`하고, `posix_fadvise(DONTNEED)`로 캐시를 버린 뒤 읽으며 바이트를 비교해요. 그동안 5ms마다 `memory.current`·`memory.stat`(`file`·`anon`·`file_dirty`·`file_writeback`)의 최대치를 재고, `memory.events`를 앞뒤로 보여 줘요. `oom_kill`이 늘면 실패 판정이에요. |

## 실행 방법

사용자가 실행해요. `[내 PC]`는 이 저장소가 있는 PC에서, `[서버]`는 서버(j4105, `~/trss`)에서 실행하는 명령이에요.

1. `[내 PC]` 저장소에서 정적 실행 파일을 빌드해요. 몇 분 걸리고 `probe-out/trss-probe`가 생겨요.
   ```sh
   docker build --target probe-binary --output type=local,dest=probe-out .
   ```
2. `[내 PC]` 실행 파일과 스크립트를 서버의 compose 폴더로 복사해요.
   ```sh
   scp probe-out/trss-probe deploy/probe.sh j4105:~/trss/
   ```
3. `[서버]` 확인에 쓸 작품 폴더를 정해요. Transmission이 만든 폴더 하나면 되고, 안의 파일은 건드리지 않아요. 호스트 경로가 `$MEDIA_DIR/downloads/<작품 폴더>`이면 컨테이너 안의 경로는 `/downloads/downloads/<작품 폴더>`예요.
4. `[서버]` 탐침을 돌리고 출력을 파일에도 남겨요. 도중에 컨테이너가 죽으면 출력이 중간에서 끊겨요.
   ```sh
   cd ~/trss
   chmod +x probe.sh trss-probe
   ./probe.sh -- --work "/downloads/downloads/<작품 폴더>" 2>&1 | tee probe-$(date +%F).txt
   ```
   작품 폴더가 여럿이면 `--work`를 더 붙여요.
5. `[서버]` 출력에 `make the test folder`의 `Permission denied`가 있으면 미디어 디스크의 루트에 1000:1000이 쓰지 못하는 거예요. 그 결과도 기록할 값이니 그대로 두고, 같은 명령에 `--media /downloads/downloads`를 더해 다시 돌려요.
   ```sh
   ./probe.sh -- --media /downloads/downloads --work "/downloads/downloads/<작품 폴더>" 2>&1 | tee probe-$(date +%F)-b.txt
   ```
6. `probe-<날짜>.txt`의 내용을 저에게 붙여 주세요. 제가 날짜·커널·파일시스템과 항목별 결과를 이 티켓에 적고, 결과가 다르면 임시 파일의 위치와 명세를 다시 정해요.

알아 둘 것이에요.

- `/data`는 서버의 앱 데이터 폴더가 아니라 `TRSS_DATA_DIR` 옆에 새로 만든 임시 폴더(`.trss-probe-data.XXXXXX`, 같은 디스크)이고 스크립트가 끝에 지워요. 서버의 앱 데이터 폴더는 [0062](0062-run-as-media-user.md)로 소유를 옮기기 전까지 root 소유라 1000:1000으로 쓸 수 없어요. 옮긴 뒤에 진짜 폴더로 돌리려면 `--data-dir "$HOME/trss/data"`처럼 호스트 경로를 줘요. 두 폴더 모두 컨테이너에서는 서로 다른 bind mount라서 `/data`에서 `/downloads`로의 이름 바꾸기는 어느 쪽이든 `EXDEV`여야 해요.
- 컨테이너 하나만 128M 한도로 띄워요. 서버의 다른 컨테이너를 멈추지 않아도 돼요. 다만 Transmission이 한창 받는 동안에는 디스크를 같이 써서 속도 수치가 낮게 나올 수 있어요. 수치(MiB/s)가 아니라 `oom_kill`과 최대 `memory.current`가 확인할 값이에요.
- 한도에 닿아 컨테이너가 죽으면 출력이 `writing 300 MiB …`에서 끊겨요. 그때는 `docker ps -a`와 `docker inspect <컨테이너> --format '{{.State.OOMKilled}}'`, `dmesg`의 OOM 줄을 함께 알려 주세요.
- 도중에 `Ctrl-C`로 끊으면 `.trss-probe-*` 폴더가 남을 수 있어요. 미디어 디스크와 작품 폴더 안의 그 이름만 지우면 돼요.
- 스크립트는 `--env`로 다른 `.env`를, `--image`로 다른 이미지를, `--memory`로 다른 한도를 받아요(`./probe.sh --help`).

## 로컬 시험 (2026-10-04, 개발 PC, 서버 결과가 아니에요)

탐침과 `deploy/probe.sh`가 돌아가는지만 개발 PC에서 봤어요. 커널은 `7.2.8-arch1-2`이고 파일시스템은 btrfs(`compress=zstd:3`)예요. 서버(Rocky Linux 9의 xfs, `5.14` 커널)의 동작은 이 시험에서 알 수 없어요.

- 이 브랜치에서 `probe-binary`로 빌드한 실행 파일을 `file`로 보면 `static-pie linked`이고 크기는 약 750KiB예요.
- 임시 `.env`(`MEDIA_DIR=./media`, `TRSS_DATA_DIR=./data`, `TRSS_VERSION=0062`)와 임시 작품 폴더 하나로 `./probe.sh -- --work "/downloads/downloads/Work A"`를 돌렸어요. 컨테이너는 `--user 1000:1000 --memory 128m --memory-swap 128m`이었고 이미지는 이 브랜치에서 만든 것이에요.
- 56개 항목이 모두 통과했고 종료 상태는 0이었어요. `/data`에서 `/downloads`로의 이름 바꾸기는 두 마운트 ID가 달라 `EXDEV`였어요(`st_dev`는 같은 `0:47`이었으니 같은 파일시스템이어도 마운트가 다르면 `EXDEV`예요). 만든 소유는 1000:1000이었어요.
- 128M에서 300MiB를 쓰고 읽는 동안 최대 `memory.current`는 128.0MiB(한도), 페이지 캐시(`file`)는 최대 약 126MiB, `anon`은 약 2MiB였고 `memory.events`의 `max`는 1,386번 늘고 `oom_kill`은 0이었어요. 이 PC의 커널에서는 한도에 닿아도 캐시를 줄이며 끝났다는 뜻이에요.
- 실패 경로도 봤어요. root 소유의 `/data` 폴더와 없는 작품 폴더를 주자 해당 항목이 `[FAIL]`이고 종료 상태가 1이었어요.
- 이 시험으로 알 수 없는 것은 실제 xfs의 동작, Transmission이 만든 폴더의 소유와 모드, 서버 커널에서의 `memory.events`예요. 서버에서 돌려야 알 수 있어요.
