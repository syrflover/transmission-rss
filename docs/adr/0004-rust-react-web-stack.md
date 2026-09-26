# Rust 서버와 React 화면을 사용해요

기존 Rust·Tokio 기반 영상 처리 코드를 유지하고, 웹 서버는 axum, 화면은 React·TypeScript로 구현하기로 했어요.
작업 목록·자막 비교·승인·원격 인증 화면의 상태를 프런트엔드에서 구성하되, 작업과 파일 변경의 권위는 서버에 남겨요.
프런트엔드는 정적 번들로 빌드해 Rust 서버에서 제공하며, 운영용 Node 서버는 요구하지 않아요.

Svelte·TypeScript와 Rust 서버 HTML에 필요한 JavaScript만 더하는 대안도 비교했어요.
React 기반 UI 생태계를 사용하는 방향을 선택한 것이며, 다른 방식보다 성능이 우월하다고 검증한 결과는 아니에요.
[React의 앱 구성 안내](https://react.dev/learn/build-a-react-app-from-scratch)는 독립적인 프런트 빌드 구성을 설명해요.
정적 번들 빌드는 [Vite](https://vite.dev/guide/build)를 사용해요.
이는 개발·빌드 도구의 선택이며, 운영용 Vite 서버나 SSR 서버를 추가하는 결정이 아니에요.
UI 컴포넌트는 shadcn/ui를 사용해요.
MUI의 기본 시각 스타일 대신, 컴포넌트 소스를 소유하고 화면 외형을 직접 조정하는 방향을 선택했어요.
표·폼의 동작 조합은 프로젝트에서 관리하며, 테마·프리셋과 세부 소스 파일 배치·설치 버전은 구현 시 구체화해요.
PC뿐 아니라 태블릿·휴대폰에서도 주요 작업을 완료하는 반응형 화면을 제공하되, 원격 인증의 터치·가상 키보드 조작은 별도로 검증해야 해요.

선택 시 비교한 후보의 제공 기능과 프로젝트가 소유할 부분은 다음과 같아요.
이는 성능·모바일 사용성의 실측 순위가 아니에요.

| 후보 | 판단에 필요한 차이 |
| --- | --- |
| MUI·무료 Data Grid Community | [공식 무료 그리드](https://mui.com/x/introduction/licensing/)의 기본 정렬·필터·체크박스 선택·페이지 기능을 활용할 수 있어요. 폼 상태와 업무 필터는 별도로 구성하며, 일부 고급 유료 기능을 현재 요구의 전제로 두지 않아요. |
| Mantine | [별도 공식 폼 패키지](https://mantine.dev/form/use-form/)가 있고, [기본 Table](https://mantine.dev/core/table/)의 업무 동작은 앱이나 추가 도구로 조합해요. |
| Ant Design | [Table](https://ant.design/components/table/)·[Form](https://ant.design/components/form/)을 함께 제공하지만, [공식 FAQ](https://ant.design/docs/react/faq/#antd-doesnt-work-well-in-mobile)는 데스크톱 `antd`가 모바일에 최적화되지 않았다고 명시해요. |
| Chakra UI | [반응형 스타일 속성](https://chakra-ui.com/docs/styling/responsive-design)과 조합형 부품을 사용하며, [표의 업무 기능](https://chakra-ui.com/docs/components/table)과 폼 상태는 앱·외부 도구와 연결해요. |
| shadcn/ui | [컴포넌트 소스](https://ui.shadcn.com/docs)를 프로젝트가 소유하고 수정하며, 소스 변경과 상위 변경의 병합도 관리해요. |

어느 라이브러리도 자막 비교·승인의 서버 권한이나 원격 Chromium의 터치·가상 키보드 동작을 대신 구현하지 않아요.

웹과 작업은 [공통 라이브러리와 별도 바이너리](0006-separate-web-worker-binaries.md)로 구성하고, 같은 이미지의 별도 컨테이너에서 실행해요.
현재 요구는 [구현 명세](../specs/web-gui-subtitles.md)에 있고, 이 기록은 구현 완료를 뜻하지 않아요.
