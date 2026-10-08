# 0065 압축 파일을 한도를 건 별도 프로세스에서 풀어요

- 상태: 완료 (2026-10-08, 실제 서버의 측정은 아래 "실제 서버의 측정")
- 출처: [압축 해제의 격리와 한도](../../../specs/subtitles.md#압축-해제의-격리와-한도)(사용자 결정, 2026-10-04), [묶음 분석](../../../specs/subtitles.md#자막-묶음-분석과-안전한-배치-판단)의 위험 사례, 올리기가 넘긴 ZIP이 아닌 압축 파일([0047](../3-subtitle-candidates-and-receiving/0047-subtitle-upload.md))
- 막는 티켓: [0059](0059-archive-tools-and-limits.md)(도구와 한도), [0064](0064-multi-file-packages.md)(멤버를 받을 분석)

## 작업

- worker가 띄운 자식 프로세스가 0059에서 고른 도구로 압축 파일을 풀고, 그 프로세스에 0059의 메모리·시간 한도를 걸어요. 풀린 멤버는 0064의 분석으로 넘겨요.
  압축 폭탄, 해석기 오류, 멈춤이 그 묶음의 실패로 끝나고, worker의 RSS 수집과 다른 작업은 계속되게 하려는 거예요.
- 형식은 ZIP, RAR(4·5), 7z, gzip, bzip2, xz, tar와 나뉜 조각이에요. 같은 묶음의 조각을 모아 풀어요. 0059에서 풀지 못한다고 정한 형식은 `풀지 못함`과 그 까닭이에요.
- 경로 이탈(`..`, 절대 경로), 심볼릭·하드 링크, 장치 파일, 한도 초과, 중첩 한도 초과, 암호는 거부해요. 그 묶음만 실패하고 까닭을 작업 상세에 적으며, 이미 보관한 파일은 그대로예요.
- 풀지 못한 압축 파일은 수신 영역에 남겨, 나중에 풀 수 있게 되면 다시 받지 않고 분석해요.
  이 컴퓨터 쪽의 실패(디스크, 메모리 한도, 시간 한도, 자식 프로세스 오류)는 처음을 포함해 3번까지 원 수신물에서 다시 풀어요. 다음 시도는 1시간 뒤와 worker의 다음 시작 가운데 먼저 오는 때예요. 압축 파일 탓의 거절은 다시 풀지 않아요(사용자 결정, 2026-10-05). 자세한 규칙은 [압축 해제의 격리와 한도](../../../specs/subtitles.md#압축-해제의-격리와-한도)에 있어요.
- 풀던 중 worker가 죽으면 원 수신물에서 다시 풀어요. 자식 프로세스가 남지 않게 해요.
- 실제 서버의 256M worker 컨테이너 안에서 큰 정상 묶음과 폭탄 표본을 풀어, 최대 메모리와 `oom_kill`을 재요. [0060](0060-server-filesystem-probe.md)의 탐침 방식을 써요.
  worker 한도는 자식 프로세스의 `RLIMIT_AS` 256MiB가 들어가도록 128M에서 256M로 올렸어요(사용자 결정, 2026-10-05). web은 128M 그대로예요.
- 작업 상세에서 압축 파일의 `묶음 분석을 기다려요`가 분석 결과로 바뀌어요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| Drive `1-12` 회차 ZIP(개발 환경의 실제 묶음)에서 4화를 고른 작업 | 4화가 적용되고 나머지 11회차는 보관만 돼요. |
| 올린 RAR5, 7z, `tar.xz`, 나뉜 RAR(`.part1.rar`·`.part2.rar`) | 각각 풀려 분석 결과가 작업 상세에 나와요. 배치 확인 표는 [0066](0066-placement-confirmation.md)이에요. |
| 압축률 폭탄, 멤버 수 폭탄, 중첩 폭탄, 사전이 큰 xz | 그 묶음만 한도 초과로 실패해요. worker 프로세스는 죽지 않고, 다른 작업과 RSS 수집이 계속돼요. |
| `../../x.ass`, 절대 경로, 심볼릭 링크 멤버 | 묶음 밖에 아무것도 만들지 않고 실패해요. 경계 밖의 파일은 그대로예요. |
| 자식 프로세스가 시간 한도에 닿음 | 자식이 끝나고 그 묶음은 실패해요. 남은 프로세스가 없어요. |
| 풀던 중 worker를 죽였다 살림 | 원 수신물에서 다시 풀고, 다시 받지 않아요. |
| 암호가 걸린 ZIP | `풀지 못함`과 까닭이 있고, 수신 영역에 남아요. |
| 디스크에 쓰지 못해 풀지 못한 압축 파일(시험) | 작업이 `자막 대기`로 남고, 1시간 뒤나 worker의 다음 시작 때 원 수신물에서 다시 풀어요. 다시 받지 않아요. 작업 상세와 작업 기록에 몇 번째 시도인지 나와요. |
| 같은 컴퓨터 쪽 실패가 3번 | 작업이 실패나 일부 실패로 끝나고, 더는 다시 풀지 않아요. |
| 압축 파일 탓의 거절(암호, 압축률·멤버 수 한도 초과) | 다시 풀지 않아요. |
| 실제 서버 256M에서 정상 묶음과 폭탄 표본 | 최대 메모리와 `oom_kill` 수가 날짜와 함께 이 티켓에 있어요. `oom_kill`이 생기면 0058에서 한도를 다시 정해요. |

## 결과

### 만든 것 (2026-10-05)

- **푸는 크레이트** `crates/trss-archive`: ZIP(`zip`), RAR 4·5(`rars`), 7z(`sevenz-rust2`), gzip·bzip2·xz(xz만 C의 liblzma), tar와 나뉜 조각(`.partN.rar`·`.rNN`·`.7z.001`·`.zip.001`)을 풀어요. 0059의 한도(멤버 2000개, 모두 512MiB, 하나 200MiB, RAR 멤버 64MiB, 압축률, 중첩 3단, 경로, 사전 64MiB)를 풀기 전의 목록에서 먼저 보고, 쓰는 동안에도 다시 봐요.
  - ZIP은 끝 기록이 파일 끝 64KiB 안에 없으면 거절해요. 중앙 디렉터리의 머리를 직접 세어 끝 기록의 멤버 수와 다르면 거절해요.
  - 7z는 목록(헤더)의 크기를 `멤버 한도 × 512 + 64KiB`(기본 약 1MiB)로 묶어, 꾸민 헤더가 크레이트에 시키는 할당을 100MiB 남짓으로 막아요.
  - 실패는 둘로 나눠요. 압축 파일 탓인 거절(`풀지 못함`과 까닭)과, 메모리 한도·디스크·자식 프로세스 오류 같은 이 컴퓨터 쪽의 실패예요.
- **자식 프로세스** `trss-extract`(`crates/trss-jobs/src/bin/`): worker 옆의 실행 파일이에요. 시작하면서 `RLIMIT_AS` 256MiB, `RLIMIT_FSIZE` 201MiB, `RLIMIT_NOFILE` 64, `RLIMIT_CORE` 0을 걸고 `oom_score_adj`를 1000으로 올려요. 하나라도 걸지 못하면 아무것도 풀지 않아요. 결과는 JSON 한 줄로 알려요.
  - worker 쪽(`trss_archive::run::Unpacker`)은 자식을 새 프로세스 그룹으로 띄우고 120초가 지나거나 작업이 취소되면 그룹째 죽여요. 자식이 알린 멤버의 경로·크기·SHA-256은 worker가 푼 폴더에서 다시 확인해요. 자식이 알린 실패 문구는 비어 있지 않고 2048바이트 이하이며 제어 문자가 없을 때만 믿어요.
- **DB**(마이그레이션 53 `unpack.sql`): 받은 파일에 `volume_of`(뒤 조각이 속한 첫 조각), `unpacked_at`, `unpack_error`를 더하고, 풀린 멤버를 `subtitle_job_members`(경로, 크기, SHA-256, 검사한 형식이나 버린 까닭)에 적어요.
- **배치 앞 단계**(`crates/trss-jobs/src/place/unpack.rs`): 받은 압축 파일을 수신 영역의 `<작업 ID>/.unpack/<파일 ID>/`에 풀고, 멤버마다 받은 파일과 같은 내용 검사를 한 뒤 그 압축 파일 대신 멤버를 0064의 분석에 넘겨요.
  - 나뉜 조각은 한 게시물의 같은 폴더에 있는 같은 이름끼리 모아요.
  - 멤버 안의 ZIP(`.docx` 포함)을 검사하며 풀어 보는 양은 압축 파일 하나에 모두 512MiB까지예요(`verify::check_within`). 넘으면 그 멤버만 까닭을 적고 버려요.
  - `trss-extract`가 없는 worker에서는 후보 작업과 올리기 작업이 `자막 대기`로 기다려요. 다음 시작 때 다시 해 봐요.
  - 풀던 중 worker가 죽으면 다음 실행이 `.unpack/<파일 ID>/`를 지우고 원 수신물에서 다시 풀어요. 정리(`clear`)는 푼 폴더도 지우고, 푼 폴더만 남은 작업 폴더도 정리해요.
- **이미지와 Compose**: `Dockerfile`이 `trss-extract`를 `/usr/local/bin`에 넣어요. `docker-compose.trss.yml`의 worker 한도를 256M로 올렸어요. web은 128M 그대로예요.
- **탐침**: `trss-probe --unpack FILE[,FILE...]`가 worker와 같은 방식으로 자식 프로세스를 띄워 풀고, 그동안 `memory.current`·`memory.stat`의 최대치와 `memory.events`의 `oom_kill`을 재요. `deploy/probe.sh`는 `--samples DIR`(컨테이너의 `/samples`)과 `--extract FILE`을 받아요. `probe-binary` 빌드 대상이 `trss-probe`와 `trss-extract`를 함께 내보내요.
- **API와 화면**: 작업 상세의 받은 압축 파일마다 `파일 N개를 풀었어요 (자막 a · 폰트 b)`, `풀지 못함`과 까닭, `나뉜 조각 · 첫 조각과 함께 풀어요`가 나와요. 올리기 결과와 작품 상세의 올리기 칸은 압축 파일에 `받은 뒤 풀어요`를 붙이고, 풀기 전에는 `풀기를 기다려요`라고 적어요.
- **가짜 출처**: `fake.trss.invalid/pack/` 게시물의 이름에 `%2F`가 있으면 그 앞을 폴더로 줘요. 같은 이름의 조각이 두 폴더에 있는 묶음을 만들 수 있어요.
- **명세·문서**: [압축 해제의 격리와 한도](../../../specs/subtitles.md#압축-해제의-격리와-한도)에 자식 프로세스, 한도 표, 실패의 두 갈래, 멤버 검사의 512MiB를 적었어요. [작업](../../../specs/jobs.md)과 readme에 압축 파일을 받은 올리기 작업과 자원 한도를 적었어요.

### 컴퓨터 쪽 실패의 재시도 (2026-10-06)

- **DB**(마이그레이션 61 `unpack_retry.sql`): 받은 파일에 `unpack_tries`(이 컴퓨터 쪽 실패로 끝난 시도 수), `unpack_failure`(마지막 실패의 까닭), `unpack_retry_at`(다음 시도 때)을 더했어요. 시도가 남은 동안 `unpack_error`는 비어 있고, 3번째 실패가 그것을 채워요.
- **배치 앞 단계**(`place/unpack.rs`): 자식 프로세스의 답을 둘로 나눠요. 압축 파일 탓(`Refused`, 빠진 조각 같은 묶음 거절, 안에 파일이 없음)은 그대로 `풀지 못함`이에요. 이 컴퓨터 쪽(`Failed`: 디스크, 메모리·시간 한도, 자식이 죽거나 뜨지 못함, 읽을 수 없는 답)은 `UNPACK_TRIES`(3)번까지 다시 풀어요.
  - 실패한 시도는 1시간 뒤(`UNPACK_RETRY_AFTER`)를 다음 시도 때로 적고, 그때까지 그 압축 파일을 건너뛰어요. 묶음은 `압축 파일을 풀지 못해 다시 풀기를 기다려요`(`UNPACK_AGAIN`)로 기다리고, 작업은 `자막 대기`예요.
  - 작업 기록에 `pack.zip: 압축 파일을 풀지 못해 다시 풀어요`와 `<까닭> (3번 중 1번째 시도). 1시간 뒤나 worker가 다시 시작할 때 다시 풀어요`를 남겨요. 3번째 실패는 `pack.zip: 압축 파일을 풀지 못했어요`와 `<까닭> (3번 중 3번째 시도)`예요. 다시 풀어 성공하면 `파일 2개 (3번 중 3번째 시도)`처럼 붙여요.
- **다시 줄에 세우기**: worker가 시작할 때 부르는 `requeue_waiting_for_sources`가 모든 `unpack_retry_at`을 비워, 다음 실행이 바로 다시 풀어요. 작업 루프는 명령을 살필 때마다 `requeue_unpack_retries`로 1시간이 지난 시도가 있는 `자막 대기` 작업을 다시 줄에 세워요. 그런 작업이 없으면 쓰기 잠금을 잡지 않아요.
- **API와 화면**: 작업 상세의 압축 파일 `unpack`에 `retry` 상태와 `tries`, `retry_at`을 더했어요. 화면은 `다시 풀기를 기다려요 (3번 중 1번째 시도 실패)`, 까닭, `다음 시도: 오늘 13:00 또는 worker가 다시 시작할 때`(worker가 이미 다시 시작했으면 `다음 실행 때`)를 적어요. 3번 실패한 `풀지 못함`은 까닭 뒤에 `3번 시도했어요`를 붙여요. 문구는 `web/src/screens/todo/unpack.ts`로 옮겼어요.
- **다시 푼 시도가 거절로 끝남**: 압축 파일 탓의 거절은 다음 시도 때를 지우고, 앞선 실패가 있으면 그 시도도 `unpack_tries`에 세요. 기록은 `<까닭> (3번 중 2번째 시도)`, 압축 파일 줄은 `2번 시도했어요`예요. `requeue_unpack_retries`는 아직 풀지도 거절하지도 않은 압축 파일만 봐요.
- **프로그램이 없는 worker**: `trss-extract`가 없는 runner는 `requeue_unpack_retries`에서 아무것도 줄에 세우지 않고, 그 묶음의 까닭은 `ARCHIVE_LATER`예요. 시도 횟수는 그대로 남아요.
- **구현 결정**(2026-10-06): 이 규칙 전에 이 컴퓨터 쪽 실패로 `unpack_error`가 생긴 압축 파일은 다시 풀지 않아요. 남은 까닭 문구만으로 두 갈래를 가르는 일은 하지 않았어요. 안에 파일이 없는 압축 파일은 압축 파일 탓으로 봐요(전에도 다시 풀지 않았어요). 다시 푼 시도가 거절로 끝나면 그 시도도 시도 수에 넣어, 화면의 `N번 시도했어요`가 실제로 푼 횟수가 되게 했어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| Drive `1-12` 회차 ZIP에서 4화 | 실제 Drive 묶음은 관찰하지 못했어요. 통합 시험 `the_candidates_episode_of_a_season_zip_is_applied_and_the_rest_stored`가 12회차 ZIP에서 4화 후보의 작업이 4화만 적용하고 11개를 보관만 하는 것을 실제 파일과 기록으로 확인해요. 개발 환경(2026-10-05)에서는 Naver 묶음 `네죽사 1~8화 자막.zip`(작업 `6e8c97b4`)이 풀려, 고른 회차 하나가 적용되고 나머지 회차는 `고르지 않은 회차라 보관만 해요`가 됐어요. |
| 올린 RAR5, 7z, `tar.xz`, 나뉜 RAR | 시험 `uploaded_archives_of_each_format_are_unpacked_by_the_worker`(다섯 형식, 받기 단계 기록 `압축 파일 5개`)와 `the_volumes_of_a_split_archive_are_unpacked_and_cleared_together`. 같은 이름의 조각이 두 폴더에 있으면 따로 묶어요(`volumes_of_one_name_in_two_folders_are_two_sets`). 첫 조각이 없으면 그 까닭을 적어요(`a_split_set_without_its_first_volume_says_why`). API 시험 `each_received_archive_says_what_came_of_unpacking_it`가 작업 상세의 압축 파일 줄을 확인해요. 형식별 해제는 `trss-archive`의 `formats` 시험 34개예요. |
| 압축률·멤버 수·중첩 폭탄, 사전이 큰 xz | 시험 `bombs_and_a_password_fail_alone_and_the_next_job_goes_on`: 폭탄과 암호 ZIP이 그 묶음만 `풀지 못함`이 되고 다음 작업이 이어져요. 한도마다의 거절은 `refusals` 시험 30개예요. 자식 프로세스가 메모리 한도에 닿아도 worker가 아니라 자식이 OOM killer의 대상이고 core 파일이 남지 않아요(`the_child_is_the_process_the_oom_killer_picks_and_leaves_no_core_dump`). |
| `../../x.ass`, 절대 경로, 심볼릭 링크 | 시험 `an_archive_whose_member_leaves_it_fails_and_stays_received`: 묶음 밖에 아무것도 생기지 않고, 압축 파일은 수신 영역에 남아요. 형식마다의 경로·링크·장치 파일 거절은 `refusals` 시험이에요. |
| 시간 한도 | 시험 `a_child_that_hangs_on_its_input_is_killed_at_the_time_limit`, `the_whole_process_group_is_killed_at_the_time_limit`, `what_a_child_left_running_is_killed_when_it_ends`: 자식과 그 그룹이 끝나고 남은 프로세스가 없어요. 취소도 같아요(`a_cancellation_kills_the_child_and_leaves_nothing`). |
| 풀던 중 worker를 죽였다 살림 | 시험 `an_archive_waits_for_the_program_and_a_killed_unpacking_starts_anew`: 반쯤 푼 폴더를 지우고 원 수신물에서 다시 풀며, 다시 받지 않아요. |
| 암호가 걸린 ZIP | 위 폭탄 시험: `풀지 못함`과 `암호가 걸려 있어요`가 있고 수신 영역에 남아요. |
| 디스크에 쓰지 못한 압축 파일(시험) | 시험 `an_archive_this_machine_failed_to_unpack_is_tried_again_from_its_receipt`: 디스크가 찼다고 답하는 프로그램(`tests/fixtures/full-disk-extract.sh`)으로 풀면 작업이 `자막 대기`와 `UNPACK_AGAIN`으로 남고, 기록에 `3번 중 1번째 시도`가 있어요. 1시간 전에는 다시 줄에 서지 않고, 1시간이 되면 2번째 시도가 돌아요. worker 시작(`requeue_waiting_for_sources`) 뒤 실제 `trss-extract`로 3번째 시도에 풀려 적용되고, 받은 파일은 처음의 것 하나예요. API 시험 `each_received_archive_says_what_came_of_unpacking_it`가 `retry` 줄의 `tries`·`retry_at`을, 웹 시험 `unpack.test.ts`가 줄의 문구를 확인해요. 개발 DB 사본(2026-10-06)에서 압축 파일 하나를 SQL로 재시도 상태로 바꾸자, 작업 상세에 `다시 풀기를 기다려요 (3번 중 1번째 시도 실패) · 압축을 풀 자리에 쓰지 못했어요: … · 다음 시도: 오늘 18:02 또는 worker가 다시 시작할 때`가 나왔어요. 실제 worker가 디스크를 채운 상태에서 실패한 것은 아니에요. |
| 같은 컴퓨터 쪽 실패가 3번(화면) | 같은 사본에서 올리기 작업의 압축 파일을 3번 실패한 상태로 바꾸자, 올린 파일의 `풀지 못함` 아래에 `<까닭> · 3번 시도했어요`가 나왔어요(2026-10-06). |
| 같은 컴퓨터 쪽 실패가 3번 | 시험 `the_third_failed_try_leaves_the_archive_not_unpacked`: 3번째 실패에 `unpack_error`가 생기고 작업이 실패로 끝나며, 기록에 `3번 중 3번째 시도`가 있어요. 그 뒤 몇 시간이 지나거나 worker가 다시 시작해도 기록이 늘지 않고 원 수신물이 남아요. 일부 실패로 끝나는 길은 기존 `a_refused_archive_beside_a_placed_file_leaves_the_job_partial`과 같은 판정을 써요. |
| 압축 파일 탓의 거절 | 시험 `an_archive_whose_member_leaves_it_fails_and_stays_received`(경로 이탈, `unpack_tries` 0, 다시 시작해도 다시 풀지 않음), `an_empty_archive_is_not_tried_again`(폴더만 든 ZIP), 폭탄과 암호 시험이에요. 디스크 실패 뒤의 시도가 거절로 끝나면 시도가 끝나고, 그 작업이 다른 까닭으로 `자막 대기`여도 다시 줄에 서지 않아요(`a_refusal_after_a_failed_try_ends_the_tries`). |
| 재시도의 나머지 경로(구현 범위) | 나뉜 압축 파일은 조각을 함께 기다렸다가 함께 풀어요(`a_split_archive_is_tried_again_as_one`). 올리기 작업은 다시 풀기를 먼저 기다리고, 풀린 뒤 배치 확인을 기다려요(`an_uploads_archive_is_tried_again_before_its_placement_is_confirmed`). 시각이 안 된 시도는 작업이 다른 까닭으로 돌아도 풀지 않아요. 프로그램이 없는 worker는 까닭을 `ARCHIVE_LATER`로 두고 줄에 세우지 않아요. 이 둘은 `an_archive_this_machine_failed_to_unpack_is_tried_again_from_its_receipt`에 있어요. |
| 실제 서버 256M | 2026-10-08 실제 서버(j4105)의 256M 컨테이너에서 정상 묶음 4개가 풀리고 폭탄 표본 8개가 거절됐어요. 모든 표본에서 `oom_kill`이 0이었어요. 아래 [실제 서버의 측정](#실제-서버의-측정-2026-10-08)에 있어요. |

이 PC(Arch Linux, 커널 7.2.8, Docker)의 256M 컨테이너(`--memory-swap 256m --cpus 0.25`)에서 `trss-probe --unpack`으로 잰 값이에요(2026-10-05, 이 작업 트리로 지은 정적 실행 파일). 서버의 값이 아니에요.

| 표본 | 결과 | 걸린 시간 | 최대 `memory.current` | 그 가운데 익명 메모리 | `oom_kill` |
| --- | --- | --- | --- | --- | --- |
| `big.zip`(멤버 30개, 200.3MiB) | 풀림 | 2.3초 | 245.8MiB | 0.4MiB | 0 |
| `big.7z` | 풀림 | 10.1초 | 256.0MiB | 8.5MiB | 0 |
| `big.tar.xz` | 풀림 | 8.2초 | 249.7MiB | 1.4MiB | 0 |
| `big.rar` | 풀림 | 12.3초 | 254.6MiB | 15.4MiB | 0 |
| 멤버 10만 개 ZIP, 30단 중첩 ZIP, 사전 1.5GiB xz·4GiB 7z·1GiB RAR5, 1GiB 0 멤버 RAR, 사전 512MiB 7z·xz, 암호 ZIP, `../../` ZIP | 모두 거절 | 각 0.1초 미만 | 44.9MiB 이하 | 0.3MiB 이하 | 0 |

최대 `memory.current`가 한도에 닿은 것은 거의 모두 쓴 파일의 페이지 캐시예요. `big.7z`와 `big.rar`에서 `memory.events`의 `max`가 늘었지만(149, 9) 회수로 끝났고 `oom` 0, `oom_kill` 0이었어요.

### 실제 서버의 측정 (2026-10-08)

사용자가 [실제 서버에서 재는 방법](#실제-서버에서-재는-방법)대로 서버(j4105)에서 돌렸어요(07:59 UTC). 서버는 Rocky Linux 9.8, 커널 5.14.0-687.53.1.el9_8, Docker 29.8.1이에요. 이미지는 배포된 `0.6.2`이고, 컨테이너는 1000:1000, 메모리 256m(스왑도 같음), CPU 0.25개로 worker와 같아요. `trss-probe`와 `trss-extract`는 2026-10-08에 `ef90d40`에서 지었어요. 그 커밋의 `crates/`는 0.6.2에 worker 시험 하나만 더한 거예요. 원본 출력은 `dev/local/measure/0065/unpack-2026-10-08.txt`(저장소에 없음)에 있어요.

| 표본 | 결과 | 걸린 시간 | 최대 `memory.current` | 그 가운데 익명 메모리 | `memory.events`의 `max` | `oom_kill` |
| --- | --- | --- | --- | --- | --- | --- |
| `big.zip`(멤버 30개, 200.3MiB) | 풀림 | 5.7초 | 232.6MiB | 0.4MiB | 0 | 0 |
| `big.7z` | 풀림 | 26.0초 | 256.0MiB | 8.5MiB | 65 | 0 |
| `big.tar.xz` | 풀림 | 21.0초 | 256.0MiB | 1.4MiB | 135 | 0 |
| `big.rar` | 풀림 | 37.9초 | 256.0MiB | 13.3MiB | 184 | 0 |
| 멤버 10만 개 ZIP, 30단 중첩 ZIP, 사전 1.5GiB xz·4GiB 7z·1GiB RAR5, 1GiB 0 멤버 RAR, 암호 ZIP, `../../` ZIP | 모두 거절 | 각 0.1초 이하 | 49.4MiB 이하 | 0.3MiB 이하 | 0 | 0 |

- 이 PC와 같이 한도에 닿은 것은 쓴 파일의 페이지 캐시예요. 익명 메모리는 가장 많을 때 13.3MiB였고, `memory.events`의 `max`가 늘어도 회수로 끝나 `oom`과 `oom_kill`이 0이었어요. 0058에서 한도를 다시 정할 일은 없어요.
- 거절의 까닭은 명세의 한도 그대로였어요. `멤버가 2000개를 넘어요`, `압축 파일 안의 압축 파일이 3단을 넘어요`, `사전 크기가 64MiB를 넘어요`(세 형식), `z1g.bin: 멤버 하나가 200MiB를 넘어요`, `암호가 걸려 있어요`, `../../escape1.ass: 풀 수 없는 경로예요`예요.
- 정상 묶음은 이 PC의 측정(CPU 0.25개)보다 2.5–3.1배 오래 걸렸어요. 이 PC에서 측정한 사전 512MiB 7z·xz는 서버 명령에 들어 있지 않아 서버에서는 측정하지 않았어요.
- 같은 실행에서 [0060](0060-server-filesystem-probe.md)의 파일시스템 탐침도 0.6.2 이미지로 다시 돌았어요. 압축 해제를 포함한 59개 검사가 모두 통과했어요.

### 독립 검토

검토 두 번(2026-10-05)이 있었어요. 하나는 `trss-archive`와 자식 프로세스를, 다른 하나는 worker의 배치 앞 단계를 봤어요. 아래 결함을 고쳤어요. jobs 쪽은 검토가 다시 봐서 닫았어요. 압축 해제 쪽의 마지막 수정(7z 목록 한도, RAR 메모리 오류)은 제가 다시 확인했고, 7z 시험은 옛 한도로 되돌리면 실패해요.

- ZIP 끝 기록 뒤에 다른 끝 기록을 붙이면 크레이트가 앞의 기록으로 돌아가 같은 이름이나 남는 멤버가 검사를 피했어요. 이제 끝 기록의 위치와 중앙 디렉터리의 머리 수를 직접 확인해요.
- 7z 목록의 크기 한도가 느슨해, 2.5MB 헤더 하나로 자식이 237MiB까지 썼어요. 이제 약 1MiB에서 거절해요.
- 자식 프로세스가 OOM killer의 대상이 아니었고 죽으면 core 파일을 남겼어요. 이제 `oom_score_adj` 1000, `RLIMIT_CORE` 0이에요.
- 디스크·메모리 같은 이 컴퓨터 쪽의 실패와, 푼 파일을 다시 읽다 난 오류를 압축 파일이 깨진 것으로 적었어요. 이제 실패로 적고 문구는 `압축을 풀다가 메모리 한도(256MiB)에 닿았어요`처럼 한도를 말해요. RAR의 메모리 오류도 같아요.
- 멤버 안의 ZIP 검사가 멤버마다 따로 512MiB를 풀 수 있었어요. 이제 압축 파일 하나에 모두 512MiB이고, 멤버 사이에서 취소를 봐요.
- 올린 압축 파일을 이름이 아닌 내용 형식으로도 알아보지 않았고, `trss-extract`가 없으면 올리기 작업이 기다리지 않았어요. 이제 둘 다 고쳤어요.
- 같은 이름의 조각이 게시물의 다른 폴더에 있으면 한 묶음으로 합쳤어요. 이제 폴더마다 따로예요.
- 다시 대기로 돌았다 끝난 올리기 작업의 `올린 파일: …` 기록이 사라졌어요. 이제 받은 파일에서 다시 만들어요.
- 뒤 조각과 바뀌지 않은 개정 항목의 수신물이 처리 대기로 남았어요. 이제 빼요.
- 푼 폴더만 남은 작업은 정리하지 않았어요. 이제 지우고 기록해요.

### 시험

검토 뒤 고친 코드(2026-10-05, `ab2b458` 위의 작업 트리)에서 `cargo test --workspace`가 2,510개 통과, 실패 0이었어요. `cargo clippy --workspace --all-targets -- -D warnings`와 `cargo fmt --all --check`가 통과해요. 웹은 `npm test` 96개 통과, `npm run typecheck`가 통과했어요.

재시도를 더하고 검토 뒤 고친 코드(2026-10-06, `0ce2f42` 위의 작업 트리)에서 `cargo test --workspace`가 2,720개 통과, 실패 0, 무시 13이었어요. clippy(`-D warnings`)와 `cargo fmt --all --check`가 통과했어요. 웹은 `npm test` 197개 통과, `npm run typecheck`가 통과했어요. 새 마이그레이션 때문에 같은 번호 마이그레이션(60)의 시험이 뒤의 마이그레이션이 더하거나 지운 색인도 받아들이게 고쳤어요.

재시도의 mutation 23개(2026-10-06, `probe-out/0065r-mutations.txt`, 저장소에 없음) 가운데 22개를 시험이 잡았어요. 잡지 못한 U20은 위 한계의 이중 방어예요. 처음 돌렸을 때 놓친 두 개(시각이 안 된 시도를 건너뛰기, 프로그램 없는 runner의 줄 세우기)는 시험을 고친 뒤 잡혔어요.

#### 재시도의 독립 검토 (2026-10-06)

검토 한 번이 분류, 멈춤과 반복, 마이그레이션, 나뉜 조각, 정리, 동시 실행, API·화면을 봤어요. 막는 결함은 없었고 아래를 고쳤어요.

- 다시 푼 시도가 압축 파일 탓으로 거절되면 다음 시도 때가 남았어요. 그 작업이 다른 까닭으로 `자막 대기`면 명령을 살필 때마다 다시 줄에 섰어요. 이제 거절이 그때를 지우고, 줄 세우기도 아직 풀지 않은 압축 파일만 봐요. 화면의 시도 수도 그 시도를 세요.
- `trss-extract`가 없는 worker에서 시각이 된 시도가 계속 줄에 섰어요. 이제 그런 runner는 줄에 세우지 않고, 까닭은 프로그램이 없다는 것이에요.
- 다음 시도 때가 지난 줄이 그 시각을 그대로 적었어요. 이제 `다음 실행 때`예요.

검토가 고친 코드를 다시 읽고 앞의 두 결함을 닫았어요. 새 결함은 없었어요.

나머지 지적(재시작이 시도를 빨리 씀, 해석기가 죽어도 다시 시도함, 읽기 오류가 거절임, `UNPACK_TRIES`가 두 곳에 있음)은 위 한계에 적었어요. [0066](0066-placement-confirmation.md)의 `기계 쪽 실패를 다시 시도할지는 아직 정하지 않았어요`는 재시도 결정(2026-10-05 18:33)보다 앞서(같은 날 15:51) 완료한 기록이라 고치지 않았어요.

### 한계

- 컴퓨터 쪽 실패의 재시도(2026-10-06) 전에 그런 실패로 `풀지 못함`이 된 압축 파일은 그대로 남아요(위 구현 결정).
- 재시도의 1시간과 worker 시작은 시험의 시계와 `requeue_waiting_for_sources` 호출로 확인했어요. 실제 worker를 다시 띄우거나 실제로 디스크를 채워 보지는 않았어요. worker가 명령을 살필 때 `requeue_unpack_retries`를 부르는 연결(`trss-worker`의 `run_jobs_once`)은 시험하지 않았어요. 영상 도착의 같은 연결도 시험이 없어요.
- worker가 짧은 사이에 여러 번 다시 시작하면(배포 중, 재시작 반복) 시작마다 바로 다시 풀어서, 3번이 1시간 간격 없이 금방 다 쓰일 수 있어요. 사용자가 정한 `worker의 다음 시작`을 그대로 따른 결과예요.
- 해석기가 깨진 압축 파일에서 SIGSEGV 같은 시그널로 죽거나 시간 한도에 닿아도 명세대로 이 컴퓨터 쪽 실패로 보고 3번 시도해요. 같은 파일이면 대개 같은 결과일 거예요.
- 받은 조각을 읽지 못한 오류(열기·읽기)는 전처럼 `압축 파일을 읽지 못했어요`라는 압축 파일 탓의 거절이라 다시 풀지 않아요(`trss-archive`의 `input_error`). 명세의 이 컴퓨터 쪽 실패 목록에는 쓰기만 있어요.
- 화면의 `3번 중`은 웹의 `UNPACK_TRIES`가 worker의 값과 같다고 보고 적어요. API는 그 값을 보내지 않아요.
- `requeue_unpack_retries`가 아직 풀지도 거절하지도 않은 압축 파일만 보는 조건은 이중 방어예요. 풀거나 거절하면 다음 시도 때를 지우므로, 그 조건을 빼도 시험이 실패하지 않아요(아래 mutation U20).
- 직접 찾기 작업은 아직 분석을 거치지 않아(배치 확인 [0066](0066-placement-confirmation.md)), 받은 압축 파일이 풀리지 않고 `풀기를 기다려요`에 머물러요.
- ZIP 끝 기록을 꾸며 크레이트가 앞의 기록으로 돌아가게 하면, 그 중앙 디렉터리는 멤버 수를 세기 전에 크레이트가 읽어요. 이 경우는 자식의 `RLIMIT_AS`와 OOM 점수가 막아요. 압축된 7z 목록도 같아요.
- RAR이 안쪽의 압축 파일을 여는 길(`rars::read_path`)에서 난 메모리가 아닌 I/O 오류는 아직 거절로 적어요.
- 멤버 검사의 512MiB를 압축 파일 하나에서 다 쓰는 경로는 단위 시험만 있고 통합 시험은 없어요.
- WinRAR로 만든 파일, `.tar.001`·`.gz.001`, PPMd 7z, BCJ2, 4GiB 넘는 ZIP64, deflate64는 시험하지 않았어요(0059).
- 위 개발 환경 관찰(작업 `6e8c97b4`)은 검토 전의 코드로 지은 이미지에서 했어요.
- 시간 한도의 여유는 실제 서버에서 풀린 속도로 어림한 것이고 측정하지 않았어요. 정상 묶음의 속도가 같다면 모두 512MiB인 묶음은 RAR 97초, 7z 66초, xz 54초, ZIP 15초쯤 걸려요. RAR은 120초 한도에 20%쯤 여유가 있고, 같은 때 worker가 같은 CPU 0.25개로 다른 일을 하면 넘을 수 있어요. 넘으면 이 컴퓨터 쪽 실패로 3번까지 다시 풀어요. 실제로 받은 자막 묶음의 크기를 모아 보지는 않았어요.
- 이 PC의 측정에 쓴 표본은 `probe-out/samples`(저장소에 없음)에 있어요. 0059의 시험용 생성 코드와 그때 만든 폭탄 표본에서 복사했어요.

### 실제 서버에서 재는 방법

사용자가 실행해요. `[내 PC]`는 이 저장소가 있는 PC에서, `[서버]`는 서버(j4105, `~/trss`)에서 실행하는 명령이에요. [0060](0060-server-filesystem-probe.md)의 탐침과 같은 스크립트이고, 그 탐침과 함께 돌려도 돼요.

1. `[내 PC]` 저장소에서 정적 실행 파일을 빌드해요. 몇 분 걸리고 `probe-out/trss-probe`와 `probe-out/trss-extract`가 생겨요.
   ```sh
   docker build --target probe-binary --output type=local,dest=probe-out .
   ```
2. `[내 PC]` 실행 파일, 스크립트, 표본 폴더를 서버의 compose 폴더로 복사해요. 표본은 175MB쯤이에요.
   ```sh
   scp -r probe-out/trss-probe probe-out/trss-extract deploy/probe.sh probe-out/samples j4105:~/trss/
   ```
3. `[서버]` 표본을 풀어 재고 출력을 파일에도 남겨요. `--size-mib 0`은 0060의 큰 파일 쓰기를 건너뛰어요.
   ```sh
   cd ~/trss
   chmod +x probe.sh trss-probe trss-extract
   ./probe.sh --samples ./samples -- --size-mib 0 --unpack /samples/big.zip --unpack /samples/big.7z --unpack /samples/big.tar.xz --unpack /samples/big.rar --unpack /samples/many100k.zip --unpack /samples/nested30.zip --unpack /samples/dict1536m.xz --unpack /samples/dict4095m.7z --unpack /samples/dict_rar50_1g.rar --unpack /samples/zeros1g.rar --unpack /samples/enc_aes.zip --unpack /samples/esc.zip 2>&1 | tee unpack-$(date +%F).txt
   ```
4. `unpack-<날짜>.txt`의 내용을 저에게 붙여 주세요. 제가 날짜·커널과 표본마다의 최대 `memory.current`·`oom_kill`을 이 티켓에 적어요. `oom_kill`이 생기면 [0058](../../../tickets/0058-server-oom-and-resource-limits.md)에서 한도를 다시 정해요.
