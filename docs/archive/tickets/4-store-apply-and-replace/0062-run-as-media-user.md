# 0062 web과 worker를 1000:1000으로 실행해요

- 상태: 완료 (2026-10-04, 개발 환경에서 확인했어요. 실제 서버의 전환은 목표 5예요. 아래 "결과")
- 출처: [구현 경계](../../../specs/web-app.md#구현-경계와-실행-순서)의 실행 사용자(사용자 결정, 2026-10-04)
- 막는 티켓: 없음

## 작업

trss가 미디어 디스크에 쓰는 보관본·적용본이 Transmission이 받은 영상과 같은 소유(1000:1000)가 되게 해요. 재생기는 미디어를 NFS로 읽고, NFS는 소유를 숫자로 보여요.
이미지와 Compose만 바꾸고, 실제 서버의 전환은 목표 5의 배포에서 해요(사용자 결정, 2026-10-04).

- 앱 이미지(`Dockerfile`)와 [trss Compose](../../../../docker-compose.trss.yml)가 `trss-worker`·`trss-web`을 1000:1000으로 실행해요.
  사용자가 정한 것은 worker이고, web은 같은 DB(WAL과 공유 메모리 파일 포함)와 수신 영역을 쓰므로 함께 바꿔요. uid·gid가 다른 배포를 위해 값은 `.env`로 바꿀 수 있게 하고, 기본은 1000:1000이에요.
- worker가 시작할 때 서버 브라우저의 다운로드 폴더(`browser-downloads`, 모드 1777)를 준비하는 동작이 root가 아니어도 되는지 확인해요. 폴더 주인이 아니면 모드를 바꾸지 못해요.
- 시작할 때 앱 데이터 폴더와 DB에 쓸 수 있는지 확인해, 쓸 수 없으면 경로와 필요한 uid를 알리고 멈춰요. 이전 배포에서 root로 만든 파일 때문에 조용히 실패하지 않게 하려는 거예요.
- 운영 안내(`readme.md`)에 전환 절차를 적어요. 앱 데이터 폴더의 소유를 옮기고(`chown -R 1000:1000`), 미디어 폴더의 권한을 확인하는 절차예요.
- worker가 root로 돈다고 적은 현재형 문장을 고쳐요. [보관과 복원의 폴더 이동](../../../specs/collection.md#보관과-복원의-폴더-이동)의 소유 설명이에요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 새 앱 데이터 폴더로 개발 Compose를 띄움 | web과 worker가 1000:1000으로 돌고, DB·잠금·소켓·수신 영역·`browser-downloads`의 소유가 1000:1000이에요. 직접 찾기 한 번이 받기까지 끝나요. |
| root 소유 파일이 남은 이전 앱 데이터 폴더로 띄움 | 시작할 때 쓸 수 없는 경로와 필요한 uid를 알리고 멈춰요. 안내대로 소유를 옮기면 이어서 돌아요. |
| worker가 미디어에 새로 만든 파일·폴더 | 소유가 1000:1000이에요. 규칙 보관이 옮긴 폴더는 원래 소유를 그대로 가져요. |

## 결과

### 만든 것 (2026-10-04)

- **실행 사용자**: `Dockerfile`이 `USER 1000:1000`을 두고, [trss Compose](../../../../docker-compose.trss.yml)가 두 서비스에 `user: "${TRSS_UID:-1000}:${TRSS_GID:-1000}"`를 줘요. uid·gid가 다른 배포는 `.env`의 `TRSS_UID`·`TRSS_GID`로 바꿔요. web도 같은 사용자예요.
- **시작할 때 쓰기 확인**: `trss-core`의 `access::check_app_data`를 두 바이너리가 DB를 열기 전에 불러요. trss가 쓰는 것만 봐요. 앱 데이터 폴더 자체, DB 이름에 붙는 바로 아래의 파일(`trss.db`와 `-wal`·`-shm`·`-journal`, 잠금 파일 여섯 개, 깨우기 소켓), `receive/`와 `artwork/`의 모든 하위 폴더에 이 프로세스가 쓸 수 있는지 봐요. 나머지(`lost+found`, 백업으로 떠 둔 `copy.db`, `browser-downloads/`)는 보지 않아요. 쓸 수 없으면 경로 다섯 개까지와 그 소유자·모드, 이 프로세스의 uid·gid, `chown -R` 안내를 적고 오류로 멈춰요. 두 폴더 안의 파일은 보지 않아요. 수신 영역은 브라우저 컨테이너가 받은 파일을 그 소유(uid 10001)로 옮겨 두기 때문이고, 폴더를 쓸 수 있으면 파일을 지우고 읽을 수 있어서예요.
- **`browser-downloads` 준비**: `trss_browser::prepare_downloads_root`가 폴더를 만들고, 폴더의 주인이 이 프로세스(또는 root)가 아니면 모드와 상관없이 오류를 돌려줘요. 스티키 비트 아래에서는 주인만 브라우저가 만든 실행 폴더를 지울 수 있어서예요. 주인이 맞고 모드가 1777이 아니면 바꿔요. worker는 이 오류로 멈추지 않고 경로·주인·`sudo chown 1000:1000 browser-downloads && sudo chmod 1777 browser-downloads`를 적은 뒤 서버 브라우저 없이 돌아요. 브라우저 풀이 처음 쓸 때 다시 부르면 같은 오류를 돌려줘요.
- **운영 안내**: `readme.md`에 `Running as 1000:1000` 절을 더했어요. root로 돌던 설치를 옮기는 절차(trss 멈춤, 앱 데이터 폴더 `chown -R 1000:1000`, 미디어 폴더 권한 확인, 새 릴리스로 시작, 로그 확인)와 앱 데이터 폴더를 미리 만들어야 하는 까닭(없는 경로는 Docker가 root 소유로 만들어요)을 적었어요. 개발 환경 안내와 백업 문장(컨테이너가 root로 돈다던 것)도 고쳤어요.
- **명세**: [collection.md](../../../specs/collection.md#보관과-복원의-폴더-이동)의 "worker는 root로 돌지만" 문장을 1000:1000에 맞게 고치고, 이름 바꾸기가 소유를 바꾸지 않는다는 설명은 그대로 뒀어요. [web-app.md](../../../specs/web-app.md#구현-경계와-실행-순서)에 시작할 때 쓰기 확인을, [jobs.md](../../../specs/jobs.md)에 `browser-downloads` 준비의 새 동작과 받은 파일의 소유를 적었어요.

### 검증한 것

개발 환경(`dev/compose.sh`가 쓰는 두 compose 파일, 이 브랜치에서 빌드한 이미지 `ghcr.io/syrflover/transmission-rss:0062`)을 임시 데이터·미디어 폴더로 따로 띄웠어요. 실제 서버는 건드리지 않았어요.

| 완료 기준 | 근거 |
| --- | --- |
| 새 앱 데이터 폴더, web과 worker의 실행 사용자 | `docker inspect`의 `Config.User`가 둘 다 `1000:1000`이고, 호스트의 `/proc/<pid>/status`에서 두 프로세스의 Uid·Gid가 모두 1000이었어요. |
| 새 앱 데이터 폴더의 소유 | 빈 폴더로 띄운 뒤 `trss.db`·`-wal`·`-shm`·잠금 파일 다섯 개·`trss.db.wake`·`browser-downloads`(모드 1777)가 모두 1000:1000이었어요. 서버 DB 사본 하나만 든 새 폴더로 다시 띄워 직접 찾기를 한 뒤에는 `receive/`·`receive/.tmp`·작업 폴더도 1000:1000이었어요. |
| 직접 찾기 한 번이 받기까지 | 서버 DB 사본에 가짜 제작자(`fake.trss.invalid/blog/fakemaker`, 개발 환경의 가짜 출처)를 넣고 `POST /api/subtitle-jobs/find`로 작업을 만들었어요. 웹 화면의 원격 화면에서 첨부 `fakemaker-3.srt`를 눌러 서버 브라우저가 받았고, `받기 끝내기` 뒤 작업이 `받은 파일: 자막 1개`로 끝났어요. 실제 사이트는 쓰지 않았어요. 받은 파일은 `receive/<작업>/`에 브라우저의 소유(10001:10001)로 있어요. |
| root 소유 파일이 남은 폴더 | 서버 DB 사본이 든 `dev/local/data`의 복사본(`receive/`와 잠금 파일 여섯 개, 소켓이 root 소유)으로 띄우자 web과 worker가 둘 다 `cannot write the app data folder /data as uid 1000 gid 1000`와 경로·소유자·모드를 적고 멈췄어요(컨테이너는 재시작 정책으로 되풀이했어요). `alpine` 컨테이너로 `chown -R 1000:1000` 한 뒤 다시 띄우자 둘 다 돌고 웹이 200으로 답했어요. |
| 미디어에 쓰는 일 | worker가 1000:1000으로 `보관` 명령(`rule_archive`)을 마쳤어요(처음에는 Transmission의 받다 만 토렌트가 막았고, 그 토렌트를 지우자 옮겼어요). 이 명령의 이름 바꾸기 시험 파일은 보관 폴더와 수집 폴더에 1000:1000으로 만들어졌다가 지워졌어요. 옮긴 폴더 안의 파일은 일부러 2000:2000으로 바꿔 둔 소유 그대로였어요. |
| `browser-downloads` 준비 | worker 이미지를 `--user 1000:1000`으로 직접 띄워 봤어요. root 소유·모드 755인 폴더에서는 `cannot open the downloads folder … it needs mode 1777 and has 0755, it is owned by uid 0 …`를 적고 멈췄고, root 소유·모드 1777인 폴더에서는 그대로 쓰며 시작했어요. 자기 소유인 폴더를 1777로 바꾸는 것은 새 폴더 시험에서 봤어요(`prepare`가 만든 755가 1777로). **검토 뒤 바뀜**: 이 관찰의 두 동작은 아래 검토와 고침으로 대체됐어요. 폴더 주인이 worker가 아니면 모드와 상관없이 worker가 서버 브라우저 없이 돌고(멈추지 않아요), root 소유 1777 폴더도 이제 쓰지 않아요. |

단위 시험은 아래 검토 뒤의 것이 `trss-core::access` 열두 개, `trss-browser`의 `prepare_downloads_root` 셋, `trss-worker`의 파일 이름 목록 시험 둘이에요(검토와 고침 참고).

확인하다가 고친 결함이 하나 있어요. 처음 시작 확인은 앱 데이터 폴더의 모든 파일을 봤어요. 직접 찾기가 받은 파일이 브라우저의 소유(10001:10001, 모드 644)로 `receive/`에 있자 다음 재시작이 그 파일 때문에 멈췄어요. 이제 폴더 바로 아래의 파일과 모든 폴더만 봤어요. 고친 이미지로 같은 상태에서 재시작해 두 프로세스가 시작하는 것을 확인했어요. **검토 뒤 바뀜**: 확인 대상은 아래 검토와 고침에서 trss가 쓰는 파일과 `receive/`·`artwork/`의 폴더로 더 좁아졌어요(그때 확인한 재시작은 이 시점의 동작에 대한 기록이에요).

### 검토와 고침 (2026-10-04)

독립 검토가 막는 결함은 찾지 못했고, 아래를 고치라고 했어요. 모두 이 티켓의 커밋에 합쳤어요.

| 지적 | 고친 것 |
| --- | --- |
| 첫 설치에서 Docker가 없는 `browser-downloads`를 root:root 755로 만들면 worker가 `prepare_downloads_root`의 오류로 종료하고 `restart: unless-stopped`로 계속 되풀이해요. | worker가 종료하지 않아요. 경로·주인·고치는 명령을 적고 서버 브라우저 없이 돌아요(0062 이전과 같아요). `readme.md`와 compose 주석이 첫 `up` 전에 폴더를 1000:1000, 모드 1777로 만들라고 해요. |
| root 소유 1777 폴더(예전 root worker가 남김)는 통과하지만 1000 worker가 브라우저(uid 10001)의 실행 폴더를 지우지 못해 빈 폴더가 조용히 쌓여요. | 주인이 이 프로세스가 아니면(root는 예외) 모드와 상관없이 같은 오류로 서버 브라우저 부분을 꺼요. 경고만 하는 쪽 대신 1번과 같은 길을 골랐어요. 폴더 안 파일 지우기는 그대로예요. |
| 시작 확인이 `lost+found`나 `copy.db` 같은 상관없는 항목에서도 멈춰요. | 확인 대상을 앱 데이터 폴더, DB 이름에 붙는 파일(`-wal`·`-shm`·`-journal`과 잠금 파일 여섯 개·깨우기 소켓), `receive/`·`artwork/` 아래의 폴더로 좁혔어요. 나머지는 건너뛰어요. 파일 이름 목록이 `lock_path_for`·`wake_path_for`와 어긋나지 않게 `trss-worker/tests/app_data_files.rs`가 각 함수의 결과를 목록과 견줘요. |
| 앱 데이터 폴더가 심볼릭 링크를 지나면 거부해요. | 폴더 자체는 `metadata()`로 봐서 링크를 따라가요. 폴더 안의 링크는 여전히 따라가지 않아요. |
| web과 worker가 건너뛰는 목록이 달라요. | 건너뛰는 인자를 없앴어요. 두 바이너리가 `check_app_data(&db_path)`로 같은 대상을 봐요. |
| `readme.md`의 명령이 `.env` 값인 `"$TRSS_DATA_DIR"`을 셸 변수처럼 써서 빈 문자열이 돼요. | compose 폴더 기준 기본 경로 `./data`로 쓰고, `.env`가 다른 경로를 정하면 그 경로를 쓰라고 적었어요. uid 1000이 아니면 `sudo`가 필요하다는 안내와 `browser-downloads` 명령(`mkdir -p`, `chown`, `chmod 1777`)을 더했어요. |

확인: 단위 시험 `cargo test --workspace`와 clippy를 다시 돌렸어요. `lost+found`(모드 000)와 `copy.db`(모드 444)가 있는 폴더는 통과하고, DB 이름에 붙는 파일 열한 가지는 각각 쓸 수 없을 때 멈춰요. 링크를 지난 폴더는 통과하고, `/tmp`(root 소유, 1777)는 이제 보고돼요. 빌드한 `trss-worker`를 root 소유 `browser-downloads`(`/tmp`)로 직접 띄우자 `running without a server browser: the downloads folder /tmp is owned by uid 0 …`와 `sudo chown 1000:1000 browser-downloads && sudo chmod 1777 browser-downloads`를 적고 타임아웃까지 계속 돌았어요. Docker가 root로 만든 폴더로 컨테이너를 다시 띄우는 시험은 하지 않았어요.

다시 확인한 검토(2026-10-04)가 위 항목에 더해 지적한 것이 있어요. 이 문서의 관찰 두 곳에 "검토 뒤 바뀜"을 달았어요. 또 브라우저 없이 도는 worker는 `clear_screens`가 일찍 끝나 이전 worker의 원격 화면 묶음을 닫지 않았어요. 인증 필요로 기다리던 작업이 사라진 실행의 `ready` 화면을 보일 수 있었어요. 이제 `browser-downloads`를 준비하지 못해 브라우저 없이 도는 worker는 서버 브라우저의 잠금을 쥔 채(다른 worker가 브라우저를 쓰지 못해요, 쥔 worker 하나만 묶음을 가져요) 시작할 때 묶음을 닫아요. 브라우저가 설정되지 않았거나 다른 worker가 잠금을 쥔 worker는 그대로 두어요. 시험은 `trss-worker`의 `jobs::tests` 둘이에요(잠금을 쥔 worker는 묶음을 닫고, 쥐지 않은 worker는 그대로 둬요).

### 남은 한계

- 실제 서버(Rocky Linux, root로 돌던 0.5.0)에서의 전환은 하지 않았어요. 목표 5의 배포에서 `readme.md`의 절차로 해요. 서버의 앱 데이터 폴더와 미디어 폴더가 어떤 소유인지는 이 확인에 없어요.
- worker가 미디어에 새로 만든 파일·폴더를 남기는 코드는 아직 없어요(0063부터). 그래서 새 파일의 소유 1000:1000은 이 프로세스의 uid로 만든 시험 파일(위 보관 명령의 이름 바꾸기 시험 파일)로만 봤고, 남는 파일로는 보지 못했어요. 보관과 적용을 만드는 티켓이 확인해요. 서버에서의 확인은 [0060](0060-server-filesystem-probe.md)의 탐침이에요.
- 받은 파일은 브라우저의 소유(10001)로 수신 영역에 있고, 이름 바꾸기로는 미디어 디스크로 옮길 수 없어서 적용할 때 복사해요. 복사본은 worker의 소유(1000:1000)예요.
- 앱 데이터 폴더가 없으면 Docker가 root 소유로 만들어서 두 프로세스가 멈춰요. `browser-downloads`만 없으면 worker는 멈추지 않고 서버 브라우저 없이 돌아요. 개발 환경은 `dev/compose.sh`가 둘 다 만들어 주고, 서버는 `readme.md`의 절차가 먼저 만들게 해요. 브라우저 없이 도는 worker가 쥔 서버 브라우저 잠금은 폴더를 고치고 worker를 다시 시작할 때까지 풀리지 않아요.
- 개발 환경의 직접 찾기는 `fake.trss.invalid`의 가짜 블로그라서 실제 사이트의 다운로드 경로(외부 이름 해석, 실제 호스트)는 이 확인에 없어요. 그 부분은 [0046](../3-subtitle-candidates-and-receiving/0046-find-in-browser.md)이 실제 사이트로 봤어요.
- 독립 검토를 2026-10-04에 받았고, 고친 뒤 다시 확인했어요(검토와 고침 참고). 다시 확인에서 나온 P3 하나(브라우저 없는 worker의 화면 묶음)를 고쳤고, 나머지 하나(`/tmp`·`/usr` 시험이 root 소유 시스템 폴더를 전제해요)는 그대로 뒀어요.
- 이 변경 뒤 개발 환경의 `dev/local/data`에는 root가 만든 `receive/`와 잠금 파일, 소켓이 있어서 `chown -R 1000:1000` 한 번이 필요해요.

### 커밋

- `feat(runtime): run trss-web and trss-worker as 1000:1000 and check the data folder at start` (자신의 해시를 이 문서에 적을 수 없어서 제목으로 찾아요)
