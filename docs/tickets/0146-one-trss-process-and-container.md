# 0146 web과 worker를 `trss` 프로세스 하나와 컨테이너 하나로 돌려요

- 상태: 대기
- 출처: [구조 가볍게 하기 명세의 한 프로세스와 이름](lighter-structure.md#한-프로세스와-이름), [한 프로세스 ADR](../adr/0019-one-process-for-web-and-worker.md)
- 막는 티켓: [0145](0145-merge-web-and-worker-crates.md)(크레이트 합치기)

## 작업

`trss` 바이너리 하나가 화면과 API 서버, 그리고 worker가 돌리던 일을 함께 돌리고, 앱 이미지와 Compose도 프로세스 하나에 맞춰요.

이 티켓은 실행 단위만 합쳐요. web 쪽이 명령을 적고 깨우기 소켓으로 알리면 worker 쪽이 worker 잠금 아래에서 집는 인계, 잠금들, 하트비트는 같은 프로세스 안에서 지금처럼 돌게 두고, [0147](0147-commands-start-at-once.md)과 [0148](0148-one-instance-lock-and-cycle-stall.md)이 없애요. 그래서 이 티켓 뒤에도 기능의 동작은 그대로이고, 바뀌는 것은 프로세스와 컨테이너의 수, 이름, 미디어 마운트, 메모리 한도예요.

- 합친 프로세스는 미디어를 쓰기로 마운트해요. 접근 경계인 LAN·VPN과 origin 검사는 그대로예요(사용자 결정, 2026-10-21).
- 메모리 한도는 지금 두 한도의 합인 384M(worker 256M, web 128M)로 시작해요. 합친 프로세스의 사용량을 서버에서 측정하는 일은 [0162](0162-deploy-lighter-structure.md)가 해요.
- 지금 서버는 `trss-web`과 `trss-worker` 컨테이너로 돌아요. 업데이트 절차가 두 컨테이너를 없애고 `trss` 하나를 띄워야 해요.
- [0135](0135-deploy-settings-item.md)는 web이 자기 환경 변수만 알아서 Transmission 주소를 보여줄 수 없다고 적어요. 합친 뒤에는 한 프로세스가 둘 다 알므로 그 티켓의 문장을 고쳐요.

## 완료 기준

- [ ] `trss` 크레이트의 바이너리는 `trss` 하나예요.
- [ ] `trss` 프로세스 하나가 화면과 API, 5분 수집 주기, 감시 폴더 읽기(알림과 자동 감시), 명령, 자막 작업, 서버 브라우저의 수명, 보관 파일 정리를 돌려요. 개발 환경에서 구독 추가의 받기, 자막 작업 하나, 보관 파일 정리 하나를 실행해 확인하고 결과에 적어요.
- [ ] 앱 이미지에는 `trss`, `trss-extract`, `trss-probe` 바이너리와 정적 화면이 들어가고, 이미지는 `trss`로 시작해요.
- [ ] Compose의 `trss-web`·`trss-worker` 서비스는 `trss` 서비스 하나가 돼요. 이 서비스는 1000:1000으로 돌고, 미디어를 쓰기로 마운트하고, `trss_net`(`gw_priority`)과 `browser_net`에 붙어요. `stop_grace_period`는 15초, 메모리 한도는 384M이고, 환경 변수는 두 서비스가 받던 것을 합친 것이에요.
- [ ] 시작할 때 web이 하던 서브넷 확인을 그대로 해요.
- [ ] 0.7.0 이미지와 그때의 `docker-compose.trss.yml`로 로컬에 띄운 환경을 readme의 업데이트 절차대로 이 버전으로 올리면, `trss`와 `trss-browser` 컨테이너만 남고 데이터는 그대로예요. 해 본 명령과 결과를 적어요.
- [ ] `dev/compose.sh`, `dev/measure/run.sh`, `deploy/probe.sh`가 `trss` 컨테이너와 그 한도·마운트를 써요.
- [ ] 같은 커밋에서 다음 문서를 고쳐요.
  - [readme](../../readme.md)의 Docker Compose, 실행·중지·업데이트, 1000:1000 실행, 자원 한도, 개발 환경
  - [개요](../overview.md)의 구성 요소와 지금 상태를 보는 곳
  - [웹 앱 공통 명세의 구현 경계와 실행 순서](../specs/web-app.md#구현-경계와-실행-순서)에서 프로세스와 컨테이너를 적은 부분
  - [자막 명세의 압축 해제의 격리와 한도](../specs/subtitles.md#압축-해제의-격리와-한도)에서 worker 컨테이너의 한도를 적은 부분
  - [RSS 수집 명세](../specs/collection.md), [설정과 이전 명세](../specs/settings.md), [작업과 인증 명세](../specs/jobs.md)에서 worker를 실행 주체로 적은 문장
  - 열린 티켓 [0058](0058-server-oom-and-resource-limits.md)의 한도 문장과 [0135](0135-deploy-settings-item.md)의 환경 변수 문장
- [ ] [명세의 동작 보존 조건](lighter-structure.md#동작-보존)을 지켜요.
- [ ] 구현이 [한 프로세스 ADR](../adr/0019-one-process-for-web-and-worker.md)과 맞아요. 다르게 해야 하면 마치기 전에 사용자 결정을 받아 ADR을 고쳐요.
