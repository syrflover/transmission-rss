# 0119 보관본을 적용하면 작품 상세가 적용이 끝난 뒤 바로 바뀌어요

- 상태: 완료 (2026-10-07)
- 출처: [할 일과 회차 목록](../specs/library.md#할-일과-회차-목록), [0083](0083-deployed-end-to-end.md)의 관찰(2026-10-07)
- 막는 티켓: 없음. 목표 5의 확인을 마친 뒤 0118–0123을 함께 마쳐 0.6.1로 올려요(사용자 결정, 2026-10-07).

## 작업

0083에서 사용자가 휴대폰으로 `All Works Maid` 9–11화의 보관본을 회차 줄에서 적용했는데, 화면이 바로 바뀌지 않았어요.
회차 줄의 `적용`은 보관본을 받은 작업에 적용을 맡기고(웹의 `applyStored`), 응답을 받자마자 작품을 다시 읽어요(`EpisodeList.tsx`의 `StoredLine`). 그때는 worker가 아직 적용하지 않아서 다시 읽어도 그대로이고, 회차 줄에는 `적용을 맡겼어요`만 남아요.
코드로 본 원인이고, 재현해 보지는 않았어요.

- 맡긴 적용이 끝나면 작품 상세가 그 결과로 바뀌어요. 실패하면 그 까닭이 회차 줄에 보여요.
- 어떻게 기다릴지(작업 상태 읽기, worker가 끝을 알리기)는 구현 때 정해요.

## 완료 기준

| 상태 | 기대 결과 |
| --- | --- |
| 자막이 없는 회차에서 보관본을 적용 | 새로고침하지 않아도 회차 줄의 자막이 `보유`가 되고, 그 보관본이 적용본으로 보여요. |
| 맡긴 적용이 실패 | 새로고침하지 않아도 회차 줄에 실패 까닭이 보여요. |

## 결과

### 바꾼 것 (2026-10-07)

- `적용`을 눌러 접수 응답(`sent`)을 받으면 줄은 `적용을 맡겼어요. 작업 보기`를 두고, 새 `ApplyStatus`가 그 작업을 2초마다 읽어요(`fetchJob`). 작업이 `done`·`failed`·`partial`로 끝나거나 사람을 기다리면(`waiting`·`held`) 그때 작품 상세를 다시 읽어요. 전에는 접수 응답 바로 뒤에 읽어서 worker가 적용하기 전의 상태를 다시 받았어요.
- 끝난 적용은 보관본이 `stored`에서 빠져 회차 줄의 자막이 `보유`가 되고 그 줄이 사라져요. 실패(`failed`·`partial`)는 작업이 적은 한 문장(`note`)을 줄에 `role="alert"`로 보이고 `작업 보기`와 버튼을 남겨 다시 누를 수 있어요. 사람을 기다리면 그 문장과 `작업 보기`를 보이고, 작품 상세를 읽은 결과로 `교체 승인` 칸이 나타나요.
- (구현 결정, 2026-10-07) 기다리는 방법은 앱이 다른 화면에서 쓰는 HTTP 주기 조회예요([웹 명령과 상태 갱신](../specs/web-app.md#웹-명령과-상태-갱신)). 새 푸시나 worker 알림을 만들지 않았어요. 조회 방식은 `usePolled`·`useItemRetry`와 같아요. 앞선 조회가 끝나야 다음을 잡고, 화면이 숨겨지면 멈추고 다시 보이면 바로 읽고, 읽기에 실패하면 실패로 치지 않고 다시 읽어요.
- (구현 결정, 2026-10-07) `usePolled`와 달리 작업 상세의 캐시(`todo:job:{id}`)를 쓰지 않아요. 같은 작업이 이전에 `done`으로 끝난 값이 캐시에 남아 있으면 새 적용이 끝난 것으로 오해하기 때문이에요. 서버는 접수 응답 전에 같은 트랜잭션에서 작업을 `pending`으로 돌려 두므로(`choose_stored`), 응답 뒤의 첫 조회는 끝난 상태가 아니에요.
- (구현 결정, 2026-10-07) 기다림은 줄이 사라지면(작품 상세를 읽었거나 화면을 떠나면), 작업이 끝나거나 사람을 기다리면, 늦어도 5분 뒤에 끝나요. worker가 멈춰 있으면 작업이 `pending`으로 남으므로 5분 뒤에는 `아직 끝나지 않았어요. 작업 보기`로 두고 더 읽지 않아요.
- `자막` 카드의 `SubtitlesCard.tsx`에도 같은 문제가 있었어요(`sent`에서 곧바로 `onChanged`). 같은 `ApplyStatus`로 같이 고쳤어요.
- [오른쪽 카드 열](../specs/library.md#오른쪽-카드-열)의 `자막`에 이 동작을 한 줄 더했어요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 자막이 없는 회차에서 보관본을 적용 | `subtitles.test.ts`의 `a job that was only queued or is running is waited for, and the work is read once it is done`가 작업이 `pending`·`running`이면 기다리고 `done`이면 끝난 것으로 읽는지 확인해요. 전에는 이 판단이 없었고 접수 응답 바로 뒤에 읽었어요. |
| 맡긴 적용이 실패 | `a failed job says why in its own sentence, and a plain one when it wrote none`이 `failed`·`partial`이 작업의 문장(없으면 정한 문장)을 돌려주는지 확인해요. |
| (기다림의 한계) | `a job that stops for a person ends the wait and says what it waits for`, `a state this build does not know is waited for, not taken as the end`, `the wait is over after five minutes and not before`가 확인해요. |

`npm run typecheck`, `npm test`(206개), `npm run build`가 통과해요.

### 확인하지 않은 것

- 원인은 코드로 읽은 것이고 재현하지 않았어요. 고친 뒤에도 브라우저에서 실제 서버로 `적용`을 눌러 줄이 바뀌는 것, 실패가 줄에 보이는 것을 보지 않았어요.
- `ApplyStatus`의 조회 순환(React 효과)은 자동 테스트가 없어요. 판단(`applyProgress`, `waitOver`)만 테스트해요.
- 0.6.1을 올린 뒤 실제 서버에서 확인하는 것은 아직 하지 않았어요.
