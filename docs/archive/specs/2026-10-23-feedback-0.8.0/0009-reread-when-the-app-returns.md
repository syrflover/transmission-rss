# 0009 앱으로 돌아올 때 다시 읽고, 끝나지 않는 읽기가 다음 읽기를 막지 않게 해요

- 상태: 완료
- 출처: [명세의 화면 갱신](spec.md#화면-갱신), [바뀜 알림 ADR](../../../adr/0021-change-events-from-the-db-connection.md), 사용자 피드백 1번(2026-10-23)
- 막는 티켓: 없음

## 작업

사용자는 할 일 등의 갱신이 제대로 되지 않는다고 알렸어요. 어느 화면에서 무엇이 바뀌지 않았는지는 기억하지 못했어요. 사용자는 PC의 Firefox와, iOS Safari로 홈 화면에 추가한 앱을 써요. 2026-10-23 데스크톱 Chromium에서 서버의 바뀜 스트림은 `ready`와 15초마다의 `alive`를 보냈고, 할 일 화면에 콘솔 오류가 없었어요. 재현하지 못했으므로, 코드에서 찾은 두 후보를 고쳐요.

- **돌아올 때 다시 읽지 않음.** 화면은 탭이 숨겨지면 스트림을 닫고(`visibilitychange`), 다시 보이면 열어 `ready`로 다시 읽어요(`web/src/lib/events.ts`). 그런데 `pageshow`, `focus`, `online` 처리가 없어요. 휴대폰 브라우저가 숨김을 알리지 않고 페이지를 얼렸다가 깨우면, 스트림은 열린 줄 알고 다시 맺지 않아요. 그러면 45초 감시가 늦게 돌 때까지 갱신이 늦거나 멈춰요.
- **끝나지 않는 읽기.** 다시 읽기는 한 번에 하나만 하고, 도중에 온 바뀜은 다음 읽기 하나로 모아요(`web/src/lib/rereads.ts`). 그런데 할 일, 자막 작업, 편성의 조회에는 시간 제한이 없어요. 죽은 연결의 요청이 끝나지 않으면 그 화면은 다시 읽지 않아요. 시간 제한이 있는 `timedRead`는 이미 있지만 명령, 보관, 규칙, 지난 회차 검색만 써요.

사용자가 정한 것은 갱신을 모든 화면에서 고치는 것과, 앱으로 돌아올 때와 네트워크가 돌아올 때 다시 읽는 것이에요(2026-10-23). 열 때만 읽는 화면이 바뀜을 듣게 하는 일은 [0010](0010-every-screen-listens.md)이 해요.

## 완료 기준

- [x] 페이지가 다시 보일 때(`pageshow` 포함), 창이 다시 포커스를 받을 때, 네트워크가 돌아올 때 바뀜 스트림을 다시 맺고 보이는 화면을 다시 읽어요. 스트림이 이미 살아 있으면 연결을 두 개 열지 않아요. 웹 단위 테스트로 확인해요.
- [x] 이벤트로 하는 모든 다시 읽기에 시간 제한이 있어요. 끝나지 않는 요청은 시간이 지나면 실패로 다뤄 재시도 간격 뒤에 다시 읽고, 그 사이 온 바뀜도 다음 읽기로 이어져요. 웹 단위 테스트로 확인해요.
- [x] 개발 환경의 Chromium에서 할 일을 연 채 네트워크를 끊었다가 이으면 다시 읽는 것을 확인하고, 결과에 적어요.
- [x] [웹 앱 공통 명세의 웹 명령과 상태 갱신](../../../requirements/web-app.md#웹-명령과-상태-갱신)에 돌아올 때의 다시 읽기와 시간 제한을 적고, 검증 표의 행을 고쳐요.
- [x] `npm test`, `npm run typecheck`가 통과해요.

## 결과

### 만든 것

화면이 쓰는 바뀜 스트림(`web/src/lib/events.ts`)은 듣는 화면이 있는 동안 창의 `pageshow`, `focus`, `online`을 들어요(`sharedEvents.ts`). 이때 스트림이 20초(`ALIVE_MS`) 동안 열리지도 무언가 보내지도 않았거나 브라우저가 포기해서 다시 열기를 기다리는 중이면, 그 스트림을 닫고 곧바로 새로 맺어요. 새 스트림의 `ready`에 보이는 화면이 모두 다시 읽어요. 서버는 15초마다 `alive`를 보내므로, 살아 있는 스트림은 20초 안에 무언가 보냈고 그대로 둬요. 페이지가 숨겨져 있거나 듣는 화면이 없으면 아무것도 하지 않아요.

다시 읽기(`web/src/lib/rereads.ts`)는 읽기마다 15초(`READ_TIMEOUT_MS`, `api.ts`의 `timedRead`와 같은 값)의 시간 제한을 둬요. 시간이 지나면 읽기의 `signal`을 `ReadTimedOut`으로 끊고 실패로 쳐서, 화면의 재시도 간격 뒤에 다시 읽어요. 그 사이 바뀜이 왔으면 곧바로 다시 읽어요. 끊은 읽기가 나중에 답해도 그 답은 듣지 않아요. `useRead`는 `ReadTimedOut`으로 끊긴 읽기를 멈춤이 아니라 실패로 다뤄요. 작품 상세의 보관 파일 정리는 다시 읽기의 `signal`을 작품 읽기까지 넘겨서, 시간이 지난 읽기의 늦은 답이 더 새 답을 덮지 않아요.

이벤트로 하는 다시 읽기는 모두 `createRereader`를 거치거나(`useRead`, 메뉴의 할 일 개수, 작품 상세의 표지·시즌 정보·자막 적용·보관 파일 정리, 표지 고르기 대화 상자), `timedRead`를 쓰는 조회예요(명령, 일괄 보관, 규칙, 지난 회차 검색). 리뷰에서 시간 제한이 없던 두 곳을 찾아 `timedRead`를 더했어요. 명령이 끝난 뒤 기록 화면이 행 하나를 다시 읽는 조회(`getHistoryItem`)와, 작품 상세가 `다시 받기`나 작품 폴더 이동이 끝난 뒤 작품을 다시 읽는 조회예요. 시간이 지나 끊긴 읽기는 본문을 받는 도중이었어도 연결을 잃은 것(`network`)으로 다뤄요.

### 정한 것

- 돌아올 때 스트림을 바꾸는 기준은 20초 동안 아무것도 받지 않은 것이에요. 서버의 `alive` 간격 15초보다 조금 길어서 살아 있는 스트림은 바꾸지 않고, 얼었다 깬 페이지의 죽은 스트림은 45초 감시를 기다리지 않고 바꿔요(구현 결정).
- 읽기의 시간 제한은 `timedRead`의 15초를 그대로 써요. 이벤트로 하는 다시 읽기와 명령의 조회가 같은 값을 쓰게 하려고 상수를 `rereads.ts`로 옮겼어요(구현 결정).

### 테스트

2026-10-23에 b1b0cf3d 위의 작업 트리에서 아래 리뷰에서 고친 뒤 `npm test`(324개)와 `npm run typecheck`가 통과했어요. 서버 코드는 바꾸지 않아서 `cargo test`는 돌리지 않았어요.

- `web` `src/lib/events.test.ts`의 `a page that comes back after a quiet while replaces its stream, and the new stream's ready reads everything again`, `a return keeps a stream that spoke lately or just opened, so it never makes a second connection`, `the network coming back opens at once the stream the browser gave up on`, `a return opens nothing while the page is hidden or nobody listens, and is not heard after the last listener left`는 돌아올 때의 동작을 가짜 시계와 가짜 스트림으로 겨눠요.
- `web` `src/lib/rereads.test.ts`의 `a read that takes longer than its time counts as failed: it is aborted, and the retry reads again`, `a change that comes during a read that runs out of time reads again at once`는 끝나지 않는 읽기를 겨눠요.

### 개발 환경 확인

2026-10-23에 이 작업 트리를 Vite 개발 서버(5173)로 띄워 `/api`를 개발 환경의 `trss` 컨테이너(서버 DB 사본)로 넘기고, 내장 브라우저(Chromium)로 할 일 화면을 열었어요. 페이지에서 `fetch`와 `EventSource`를 감싸 요청과 이벤트를 기록했어요. 원본 기록은 이 PC의 `dev/local/measure/feedback-0.8.0-0009/`에 있어요. 이 확인은 아래 리뷰에서 고치기 전의 코드로 했어요. 고친 것은 할 일 화면이 쓰지 않는 조회와, 끊긴 읽기의 오류 문구여서 이 관찰과 관계가 없어요.

- `trss` 컨테이너를 `docker pause`로 멈추고 25초 기다린 뒤, 멈춘 채로 `online`을 보내자 새 `EventSource`를 열었어요. 컨테이너를 다시 띄우자 새 스트림의 `ready`가 왔고, 곧바로 할 일 화면의 조회 6개(`/api/todo/count`, `/api/todo`, `/api/subtitle-jobs`, `/api/subscriptions/candidates`, `/api/archive-suggestions`, `/api/todo/subtitle-follow`)를 한 번씩 읽어 23 ms 안에 모두 `200`을 받았어요.
- 바로 뒤에 `focus`, `online`, `pageshow`를 보내자 새 스트림도 읽기도 없었어요.
- 앞선 시도에서는 컨테이너를 다시 띄운 뒤에 `online`을 보냈는데, 서버가 밀린 `alive`를 먼저 보내서 스트림을 그대로 뒀어요. 의도한 동작이에요.

### 리뷰와 고친 것

`stora:reviewer`가 커밋 전의 변경을 리뷰했어요. 시간 제한과 늦은 답이 엇갈리는 순서, 멈춤과 재시도, 돌아올 때 스트림을 두 번 열지 않는 것, 45초 감시와 5초 다시 열기의 관계, 시간 제한이 없는 이벤트 읽기, 새 테스트가 이름대로의 동작에 닿는지를 봤고, 막는 문제는 찾지 않았어요. 리뷰어는 엇갈리는 순서 네 가지를 따로 돌려 봤어요. 다음을 고쳤어요.

- 표지 고르기 대화 상자의 첫 읽기가 시간 제한에 걸리면 오류 없이 `불러오는 중`에 머물렀어요. 시간이 지난 읽기를 실패로 보이게 고쳤어요.
- 시간 제한이 없던 이벤트 읽기 두 곳에 `timedRead`를 더했어요(위 만든 것).
- 본문을 받는 도중에 끊긴 읽기가 `알 수 없는 응답`이라고 말했어요. 연결을 잃었다는 문구로 고쳤어요.
- `a read that takes longer than its time counts as failed…`의 마지막 확인은 늦은 답을 듣든 말든 통과했어요. 늦은 답이 진행 중인 다음 읽기를 끝내지 않는 것을 확인하게 고쳤어요.

### 한계

- 실제로 네트워크를 끊지 않았어요. 내장 브라우저는 오프라인으로 바꿀 수 없어서, `online`은 스크립트로 보냈어요. 휴대폰 브라우저가 페이지를 얼렸다가 깨우는 것도 보지 않았어요. iOS 홈 화면 앱에서 갱신을 보는 것은 [0011](0011-deploy-and-check-on-server.md)에서 해요.
- 끝나지 않는 요청은 컨테이너를 멈춰 만든 것이 아니라 테스트로만 확인했어요. 컨테이너를 멈추는 동안 화면이 다시 읽지 않았기 때문이에요.
- 사용자가 알린 갱신 문제는 재현하지 못했으므로, 이 변경이 그 문제를 고쳤는지는 서버에서 써 봐야 알아요.
- 화면의 읽기 효과(`useRead`, 작품 상세)가 시간이 지난 읽기의 늦은 답을 버리는 것은 자동 테스트가 없어요.
- `useRead`는 시간이 지난 읽기가 `signal`을 무시하고 한참 뒤에 실패하면, 더 새 읽기의 성공 뒤에 오류 상태를 쓸 수 있어요. 지금의 조회는 모두 `signal`을 `fetch`에 넘겨서 끊기는 즉시 실패하므로 생기지 않는다고 봤어요. 리뷰에서 나왔어요.
