# trss web frontend

React + TypeScript + shadcn/ui (Tailwind CSS v4), built with Vite. The production
build is a static folder (`dist/`) that the Rust `trss` binary serves; there is
no Node server at runtime.

## Commands

Run these in the `web/` folder.

| Command | What it does |
| --- | --- |
| `bun install` | Installs the dependencies (`bun.lock` is the lockfile) |
| `bun run dev` | Starts the Vite dev server, which proxies `/api` to `http://127.0.0.1:8080` |
| `bun run build` | Type-checks, then builds into `dist/` |
| `bun run typecheck` | Type-checks the app and its tests without building |
| `bun run test` | Runs the unit tests (`node --test`) |

`npm` works as well for a one-off (`npm install`, `npm run build`), but the lockfile
is bun's, so use bun to change dependencies.

## Run it with the Rust server

Run both commands in the repository root, not in `web/`: `trss` finds the build through `TRSS_WEB_STATIC_DIR`, which defaults to `web/dist` relative to the current directory. The first command builds the frontend:

```sh
bun run --cwd web build
```

The second starts `trss` on `http://127.0.0.1:8080` with the two variables it requires (see the table below): a new database in a temporary folder, and the RPC address Transmission publishes in `docker-compose.yml`.

Not tested:

- A `TRANSMISSION_URL` that nothing answers at (2026-10-22).

```sh
TRSS_DB_PATH="$(mktemp -d)/trss.db" TRANSMISSION_URL=http://127.0.0.1:9091/transmission/rpc cargo run --bin trss
```

<details>
<summary>Tested runs</summary>

Tested:

- Both commands, in this order, from the repository root with `TRSS_WEB_PORT=18080` added because port 8080 was in use, against a Transmission answering at the URL above: `trss` started, logged `static files: web/dist`, and answered `200` with the built `index.html` (2026-10-22, commit 3eb14067).

</details>

`trss` reads these environment variables for its web:

| Variable | Default | Meaning |
| --- | --- | --- |
| `TRSS_DB_PATH` | (none, required) | Path of the SQLite database file; the server does not start without it |
| `TRANSMISSION_URL` | (none, required) | Transmission's RPC address, for the collection cycles that run in the same process; the server does not start without it. The other collection settings are in the readme's [Setup](../readme.md#setup) |
| `TRSS_WEB_BIND` | `127.0.0.1` | IP address to listen on (there is no app login; bind wider only behind LAN/VPN/front-door auth) |
| `TRSS_WEB_PORT` | `8080` | Port to listen on |
| `TRSS_WEB_STATIC_DIR` | `web/dist` | Directory holding the frontend build (`index.html`, `assets/`) |
| `TRSS_WEB_HOSTS` | (none) | Host names, comma-separated, the server answers to besides IP addresses and `localhost`; a request for another name is refused (DNS rebinding) |

In the Docker image these are `0.0.0.0`, `8080` and `/usr/local/share/trss/web`.

## Layout

- `src/app/` app shell: main menu (top bar on PC, icon-only top bar on tablet, bottom bar on phone); the screen mode is set in `src/screens/settings/theme/`
- `src/screens/` one file per main menu; each screen owns its content
- `src/lib/theme.ts` screen mode (시스템 / 라이트 / 다크) store
- `src/lib/api-types/` the API's TypeScript types, generated from its Rust types and committed; do not edit them by hand, but run `cargo types` in the repository root after changing an API type, as the web app requirements' [공통 라이브러리의 모듈 구성](../docs/requirements/web-app.md#공통-라이브러리의-모듈-구성) describes
- `src/components/ui/` shadcn/ui components (project-owned source)
- `src/index.css` design tokens (from the accepted redesign prototype) and their shadcn/ui mapping
