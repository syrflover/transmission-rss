# trss web frontend

React + TypeScript + shadcn/ui (Tailwind CSS v4), built with Vite. The production
build is a static folder (`dist/`) that the Rust `trss-web` binary serves; there is
no Node server at runtime.

## Commands

```sh
bun install            # install (bun.lock is the lockfile)
bun run dev            # Vite dev server; proxies /api to http://127.0.0.1:8080
bun run build          # type-check, then build into dist/
bun run typecheck      # type-check the app and its tests without building
bun run test           # run the unit tests (node --test)
```

`npm` works as well for a one-off (`npm install`, `npm run build`), but the lockfile
is bun's, so use bun to change dependencies.

## Run it with the Rust server

```sh
bun run build
cargo run --bin trss-web      # http://127.0.0.1:8080, serves ./web/dist
```

`trss-web` reads these environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `TRSS_DB_PATH` | (none, required) | Path of the SQLite database file the worker shares; the server does not start without it |
| `TRSS_WEB_BIND` | `127.0.0.1` | IP address to listen on (there is no app login; bind wider only behind LAN/VPN/front-door auth) |
| `TRSS_WEB_PORT` | `8080` | Port to listen on |
| `TRSS_WEB_STATIC_DIR` | `web/dist` | Directory holding the frontend build (`index.html`, `assets/`) |
| `TRSS_WEB_HOSTS` | (none) | Host names, comma-separated, the server answers to besides IP addresses and `localhost`; a request for another name is refused (DNS rebinding) |

In the Docker image these are `0.0.0.0`, `8080` and `/usr/local/share/trss/web`.

## Layout

- `src/app/` app shell: main menu (top bar on PC, icon-only top bar on tablet, bottom bar on phone); the screen mode is set in `src/screens/settings/theme/`
- `src/screens/` one file per main menu; each screen owns its content
- `src/lib/theme.ts` screen mode (시스템 / 라이트 / 다크) store
- `src/components/ui/` shadcn/ui components (project-owned source)
- `src/index.css` design tokens (from the accepted redesign prototype) and their shadcn/ui mapping
