# 0055 원격 화면에서 브라우저처럼 뒤로·앞으로 가고 탭을 고르고 호스트를 봐요

- 상태: 완료 (2026-10-04, 뒤로·앞으로·호스트·탭 전환·닫기가 실제 Chromium 이미지에서 돼요. 실제 휴대폰은 확인하지 않았어요. 아래 "결과")
- 출처: [작업 화면 안의 인증과 브라우저 수명](../specs/jobs.md#작업-화면-안의-인증과-브라우저-수명), [작업 상세](../specs/jobs.md#작업-상세), 사용자 요청(2026-10-04)
- 막는 티켓: [0054](0054-popup-attach-delay.md)(새 창이 바로 실행의 페이지가 되어야 탭 시험이 30초를 기다리지 않아요)

## 배경

2026-10-04에 사용자가 휴대폰으로 직접 찾기의 원격 화면을 써 봤어요. 그 뒤 브라우저처럼 뒤로·앞으로 가고 새로고침할 수 있으면 좋겠다고 했어요.

지금 원격 화면 위에는 `새로고침`이 있어요. 터치 기기에서는 `키보드`도 있어요. 직접 찾기 화면은 게시물이 연 창(팝업)을 보여주는 동안 `이 창 닫기`도 둬요. 직접 찾기의 worker는 새 창이 열려 남으면 화면을 그 창으로 옮기고, 보이는 창이 닫히면 남은 가장 최근 창으로 옮겨요(`follow_pages`). 인증 화면은 서버가 준비한 페이지만 보여줘요.

## 사용자 결정 (2026-10-04)

- 뒤로·앞으로, 호스트 표시, 탭 목록과 전환을 넣어요. `새로고침`은 이미 있어요.
- 직접 찾기 화면과 인증 화면 모두에 넣어요.
- 뒤로는 서버가 페이지를 준비하며 거친 빈 페이지(`about:blank`)로 가지 않아요. 돌아갈 페이지가 없으면 뒤로 버튼을 누를 수 없어요.
- 주소 입력은 넣지 않아요. 사용자가 고르지 않았어요. 웹에는 로그인이 없어서, 주소 입력이 있으면 웹에 닿는 누구나 서버 브라우저로 아무 공개 사이트나 열 수 있어요. 송신 프록시(0053)는 사설 주소만 막아요.

## 정한 방식

- **호스트만 보여요**: 전체 URL은 서명된 다운로드 주소나 토큰을 담을 수 있어요. 그래서 웹은 경로·쿼리를 소켓 메시지, 로그, DB 어디에도 보내거나 남기지 않아요. 호스트가 없는 페이지(`about:blank`, `data:`, `blob:`)는 호스트 없이 보여요. 탭에는 페이지 제목도 보이고, 너무 긴 제목은 잘라 보내요.
- **뒤로·앞으로**: 웹이 보이는 페이지의 방문 기록(`Page.getNavigationHistory`)을 읽어요. `about:blank`가 아닌 첫 항목이 가장 앞이에요. 그보다 앞으로는 가지 않아요. 웹은 버튼을 누를 수 있는지를 기기에 보내요. 요청을 받을 때도 같은 규칙으로 다시 확인해요. 이동은 `Page.navigateToHistoryEntry`예요. 주 프레임이 이동하면(`Page.frameNavigated`, `Page.navigatedWithinDocument`) 웹이 상태를 다시 보내요.
- **탭 목록**: 실행에 어떤 페이지가 있는지는 worker가 알아요(`AuthBrowser::pages`). Chromium이 시작할 때 여는 창은 이 목록에 없어요(0046의 실제 이미지 시험에서 팝업을 닫은 뒤 목록이 처음 페이지 하나였어요). worker는 묶음의 페이지 목록을 온 순서대로 화면 행에 적어요. 웹은 그 목록의 페이지마다 제목과 호스트를 자기 CDP 연결로 읽어 기기에 보내요. 목록에 없는 페이지는 탭으로 보여주지 않아요.
- **탭 전환**: 웹은 두 조건이 맞을 때만 보여줄 페이지를 화면 행에 적어요. 사람이 본 묶음(`run`, `bound`)이 그대로여야 하고, 그 페이지가 목록에 있어야 해요. `이 창 닫기`와 같은 방식이에요. worker는 다음 확인에서 그 요청을 가져가요. 그 페이지가 살아 있으면 화면을 옮겨요(`retarget`, 새 묶음).
- **따라가기**: 새 창이 열려 남으면 화면이 그 창으로 옮겨요. 직접 찾기와 인증 화면이 같아요. 사람이 탭을 고른 뒤에는 두 경우에만 화면을 옮겨요. 새 창이 열려 남을 때와 보이는 창이 닫힐 때예요. 지금 `follow_pages`는 확인할 때마다 가장 최근 창으로 돌아가므로 바꿔야 해요.
- **닫기**: 처음 페이지가 아닌 탭마다 닫기를 둬요. 이 닫기가 `이 창 닫기`를 대신해요. 인증 화면에서도 처음 페이지가 아닌 창은 닫을 수 있어요. 닫기 요청은 보이지 않는 탭도 가리킬 수 있어요. 처음 페이지는 웹도 worker도 닫지 않아요.
- **사용 시간**: 뒤로·앞으로·탭 전환·닫기는 사람의 입력으로 세요. 그래서 실행의 유휴 시간이 새로 시작해요.
- **화면 배치**: 원격 화면 위 도구 줄에 뒤로·앞으로·새로고침 버튼과 호스트를 둬요. 터치 기기에서는 `키보드`도 둬요. 탭이 둘 이상이면 그 아래에 탭 줄을 둬요. 버튼은 기존 `Icon`과 버튼 관례를 따르고, 휴대폰 폭에서도 누르기 쉬운 크기예요.
- **기록**: 화면 행에 페이지 목록과 보여줄 페이지 요청을 적어요. 마이그레이션이 필요해요.

## 완료 기준

| 입력·상태 | 기대 결과 |
| --- | --- |
| 서버가 게시물을 연 처음 페이지 | 뒤로를 누를 수 없어요. 웹에 뒤로를 보내도 빈 페이지로 가지 않아요. |
| 게시물에서 다른 페이지로 이동 | 뒤로가 켜지고, 누르면 게시물로 돌아가요. 그 뒤 앞으로가 켜지고, 누르면 다시 가요. |
| 경로·쿼리에 서명이 든 주소의 페이지 | 화면에는 호스트만 보여요. 소켓 메시지, 로그, DB에 경로·쿼리가 없어요. |
| 새 창이 열려 남음(직접 찾기·인증 화면) | 탭이 하나 늘고 화면이 그 창을 보여줘요. |
| 사람이 이전 탭을 고름 | 화면이 그 페이지로 옮겨요. 다음 확인에서 가장 최근 창으로 되돌아가지 않아요. |
| 다른 묶음이나 목록에 없는 페이지로 전환·닫기를 요청 | 거절되고 아무것도 바뀌지 않아요. |
| 처음 페이지 | 닫기가 없어요. 닫기를 요청해도 거절돼요. |
| 보이지 않는 탭 닫기 | 그 창만 닫히고 화면은 그대로예요. |
| 보이는 탭 닫기 | 남은 가장 최근 창으로 돌아가요. |
| 뒤로·앞으로·탭 전환·닫기 | 실행의 유휴 시간이 새로 시작해요. |
| 실제 서버 브라우저 이미지 | 뒤로·앞으로·탭 전환·닫기가 실제 Chromium에서 돼요. |
| 휴대폰 폭(375px) | 도구 줄과 탭 줄이 가로 스크롤 없이 들어가고, 버튼을 누를 수 있어요. |

## 결과

### 만든 것 (2026-10-04)

- **도구 줄과 탭 줄**(`web/src/screens/todo/remote/RemoteScreen.tsx`): 제목 아래에 뒤로·앞으로·새로고침 아이콘 버튼과 호스트, 터치 기기의 `키보드`가 한 줄로 있어요. 실행의 페이지가 둘 이상이면 그 아래에 줄바꿈하는 탭 줄(보이는 탭 표시, 처음 페이지가 아닌 탭의 닫기)이 있어요. 인증 화면과 직접 찾기 화면이 같은 컴포넌트를 써서 `이 창 닫기`(`JobAuth.tsx`)는 없어졌어요. 탭 제목은 `tabs.ts`가 정해요(제목, 없으면 호스트, 그것도 없으면 `새 창`).
- **웹 소켓**(`crates/trss-web/src/screen_api/hub.rs`, `nav.rs`): 묶음마다 하나인 CDP 연결이 `Page.getNavigationHistory`로 `nav`(`back`·`forward`·`host`)를, `Target.getTargets`와 화면 행의 페이지 목록으로 `tabs`를 만들어 바뀐 것만 보내요(150ms 간격). 기기의 `back`·`forward`는 단계마다 기록으로 다시 판단해 `Page.navigateToHistoryEntry`로 옮기고 사람의 입력으로 세요. 호스트는 http·https 주소의 호스트만 보내요. 제목은 주소를 담았을 수 있으면 비워서 보내요. 페이지 자신의 주소(그대로, 스킴 없이, 이스케이프를 푼 꼴), `://`가 든 다른 주소, 호스트만이거나 호스트 뒤에 `/`·`?`·`#`·`:`가 붙은 꼴, 네 글자 이상인 경로·쿼리가 그 대상이에요(독립 검토 뒤 강화). 방문 기록을 읽거나 옮기다 실패해도 화면을 닫지 않고 그 걸음만 하지 않아요. 모듈 문서에 프로토콜과 엔드포인트를 적었어요.
- **엔드포인트**(`screen_api.rs`): `POST .../screen/switch`를 더하고 `.../screen/close`가 선택 `target`을 받게 했어요. 둘 다 묶음이 그대로이고 대상이 목록에 있을 때만 적고, 닫기는 처음 페이지를 거절해요. 직접 찾기에만 있던 제한을 없앴어요.
- **화면 행**(`crates/trss-jobs/src/screen.rs`, 마이그레이션 47 `screen_controls.sql`): `pages`(페이지 ID 목록)와 `switch_target_id`를 더하고, `close_target_id`는 공백으로 나눈 여러 ID를 담아요. 주소는 담지 않아요.
- **따라가기**(`crates/trss-jobs/src/runner/pages.rs`): 직접 찾기에 있던 `follow_pages`를 인증 묶음에도 붙였어요. 사람의 선택을 지키고, 새 창이 남으면 그 창을, 보이는 창이 닫히면 남은 가장 최근 창을 보여줘요. 닫기 요청은 처음 페이지를 뺀 모든 대상을 닫아요.

### 검증한 것

| 완료 기준 | 근거 |
| --- | --- |
| 서버가 게시물을 연 처음 페이지 | 실제 이미지(`remote_screen_docker`의 `a_find_screen_steps_back_and_forward_never_before_its_first_page_and_switches_and_closes_tabs`, 2026-10-04, `ghcr.io/syrflover/trss-browser:local`): 방문 기록이 `[about:blank, 게시물]`이고 `nav`가 `back:false`예요. `back`을 보내도 기록의 위치와 제목이 그대로였어요. 가짜 CDP 시험 `a_screen_tells_the_host_only_and_a_step_back_never_goes_before_the_first_page`와 `nav.rs`의 `back_stops_at_the_first_page_that_is_not_blank`도 같아요. |
| 게시물에서 다른 페이지로 이동 | 같은 실제 이미지 시험: 신뢰 클릭으로 이전 게시물로 가자 `back:true`, `back`으로 게시물로 돌아오자 `back:false, forward:true`, `forward`로 다시 가자 `back:true, forward:false`였어요. |
| 경로·쿼리가 든 주소의 페이지 | 실제 이미지 시험에서 소켓 메시지 19개(프레임 제외)에 `/blog`, `maker/`, `?`, `#`가 없었어요. 가짜 CDP 시험 `no_message_of_a_screen_carries_a_path_a_query_or_a_fragment`와 `nav.rs`의 같은 이름 시험은 경로·쿼리·조각이 든 주소와 주소 그대로인 제목을 넣어도 메시지에 없음을 확인해요. `nav.rs`의 `a_title_that_names_an_address_in_any_form_is_dropped`는 이스케이프를 푼 주소(`my file`, 한글 경로), 대문자 호스트, 포트·쿼리가 붙은 호스트, 쿼리만, 다른 주소의 인용을 넣은 제목이 비워지고, 사이트 이름만 든 제목은 남는 것을 확인해요. 웹과 worker는 주소를 로그에 남기지 않고 DB에는 페이지 ID만 적어요. |
| 새 창이 열려 남음(직접 찾기·인증 화면) | 실제 이미지 시험: `#popup`을 신뢰 클릭하자 화면이 새 묶음으로 옮겨 `tabs`가 둘이고 처음 탭만 `closable:false`였어요. 직접 찾기는 `find.rs`, 인증은 `screen.rs`의 `a_check_screen_follows_a_popup_that_stays_and_lists_its_pages`가 가짜 브라우저로 확인해요. 인증 묶음은 실제 이미지에서는 보지 않았어요. |
| 사람이 이전 탭을 고름 | 실제 이미지 시험: 처음 탭으로 전환한 뒤 3초가 지나도 화면이 처음 탭에 남았어요. `find.rs`의 `a_persons_choice_of_an_older_tab_is_kept_and_a_new_popup_is_shown_after_it`, `screen.rs`의 `a_persons_switch_on_a_check_screen_is_kept_and_a_closed_shown_tab_goes_to_the_newest_left`, `pages.rs`의 선택 유지 시험이 확인해요. |
| 다른 묶음이나 목록에 없는 페이지로 전환·닫기 | 저장소 시험 `a_request_for_another_binding_an_unlisted_page_or_the_first_page_is_refused`(`screen.rs`), 같은 실행을 다시 묶거나 worker가 다시 시작한 뒤 이전 묶음의 요청과 탭 목록이 남지 않는 것을 보는 `the_tabs_and_requests_of_a_binding_never_reach_the_next_one`(묶을 때 `switch_target_id`를 비우지 않게 바꾸면 실패해요)과 웹 시험 `a_person_asks_the_worker_to_show_a_tab_and_the_web_sends_the_browser_nothing`이 409와 변화 없음을 확인해요. 실제 이미지 시험도 이전 묶음의 전환, 없는 페이지의 전환이 409였어요. |
| 처음 페이지 | 웹 시험 `a_person_asks_the_worker_to_close_a_tab_but_never_the_first_page`, 저장소 시험 같은 위 항목, worker 시험 `the_worker_never_asks_the_browser_to_close_the_first_page`(`find.rs`)가 있어요. 실제 이미지 시험의 처음 페이지 닫기는 409였어요. |
| 보이지 않는 탭 닫기 | 실제 이미지 시험: 그 창만 닫혀 `tabs`가 하나가 됐고 화면 행의 `bound_at`이 그대로였어요. `find.rs`와 `screen.rs`의 `closing_a_tab_that_is_not_shown_closes_only_that_page`도 확인해요. |
| 보이는 탭 닫기 | 실제 이미지 시험: 보이는 새 창을 닫자 화면이 처음 페이지로 돌아오고 목록이 `[처음]`이 됐어요. `screen.rs`와 `pages.rs`의 `when_the_shown_page_closes_the_newest_page_left_is_shown`도 확인해요. |
| 뒤로·앞으로·탭 전환·닫기가 사람의 입력 | 저장소 시험 `a_request_that_was_written_is_taken_once_and_counts_as_the_persons_input`(전환·닫기)과 실제 이미지 시험(`back`·`forward` 뒤 실행의 입력 시각이 기록됨)이 확인해요. |
| 실제 서버 브라우저 이미지 | 위 시험이 2026-10-04에 통과했어요(`cargo test -p trss-web --test remote_screen_docker -- --ignored --nocapture`, 시험 둘 통과). 인증 화면의 기존 시험(`a_tap_relayed_through_the_remote_screen_passes_the_check_and_the_file_is_received`), `auth_sample`, `find_sample`도 같은 날 통과했어요. |
| 휴대폰 폭(375px) | 앱 안 브라우저를 375×812로 두고 가짜 소켓 서버로 화면을 띄워, 문서 `scrollWidth`가 375이고 모든 버튼 높이가 40px이며 탭 줄이 줄바꿈하는 것을 봤어요. 뒤로·새로고침이 소켓으로, 전환·닫기가 올바른 본문의 POST로 나가고 탭이 하나가 되면 탭 줄이 사라졌어요. 이 확인에 쓴 임시 화면과 서버는 지웠어요. |

그 밖의 시험: 독립 검토 뒤 고친 코드(2026-10-04, `e54e9df` 위의 작업 트리)에서 `cargo test --workspace --no-fail-fast`가 2,216개 통과, 실패 0, 무시 11개였고, `remote_screen_docker`의 실제 이미지 시험 둘도 다시 통과했어요. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo clippy -p trss-subtitles --features test-hooks --all-targets -- -D warnings`는 경고 0이에요. 웹은 `npm test` 75개 통과, `npm run typecheck`와 `npm run build`가 통과했어요.

### 한계

- 실제 휴대폰으로는 확인하지 않았어요. 375px 확인은 앱 안 브라우저의 크기 흉내와 가짜 소켓이에요. 터치로 탭을 누르고 닫는 느낌은 보지 못했어요.
- 인증 묶음의 팝업·전환·닫기는 가짜 브라우저 시험만 거쳤어요. 실제 이미지에서는 직접 찾기 묶음으로 확인했고, 두 묶음이 같은 따라가기와 같은 엔드포인트를 써요.
- 화면 정보의 `popup` 필드는 남아 있지만 웹 화면은 더는 쓰지 않아요.
- 실제 제작자 블로그에서는 확인하지 않았어요. 제목이 주소를 가리키는 경우를 비우는 판단은 가짜 시험과 단위 시험으로만 봤고, 실제 Chromium이 제목 없는 페이지에 붙이는 제목의 꼴은 확인하지 않았어요. 이 판단은 글자를 견주는 방식이라, 페이지가 스스로 정한 제목이 호스트 없이 세 글자 이하의 경로를 인용하는 경우처럼 가리지 못하는 꼴이 있어요.
- 사람이 탭을 고른 같은 확인(1초) 안에 새 창이 남으면, 그 창은 탭으로만 늘고 화면은 고른 탭에 남아요.
- 같은 확인에서 함께 남은 새 창 둘의 순서는 온 순서가 아니라 브라우저의 페이지 ID 순서예요.
- 인증 화면도 새 창을 따라가요. 실제 erulabo에서 확인을 푼 뒤 다운로드가 새 창으로 열려 1초 넘게 남는지는 보지 않았어요. 남으면 화면이 그 창으로 옮겨요.
