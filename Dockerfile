# Frontend build. Only this stage has Node/bun; the final image has neither.
FROM oven/bun:1 AS web-builder

WORKDIR /usr/src/web

COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile

COPY web/ ./
RUN bun run build


# Rust build: `transmission-rss` (cron RSS run), `trss-web` (web server) and
# `trss-worker` (long-running collection worker).
FROM clux/muslrust:stable as builder

WORKDIR /usr/src/transmission-rss

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# `--locked`: build the dependency versions the tests ran against.
RUN cargo build --release --locked


FROM alpine:edge

RUN apk update

WORKDIR /usr/local/bin

COPY --from=builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/transmission-rss \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-web \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-worker \
    ./

# Static frontend that `trss-web` serves (no Node server at runtime).
COPY --from=web-builder /usr/src/web/dist /usr/local/share/trss/web

# Defaults for `trss-web`; the cron `transmission-rss` run ignores them.
# The ENTRYPOINT below still runs `transmission-rss` as before (cron); run the
# web server or the worker by overriding it:
# `docker run --entrypoint ./trss-web <image>` / `--entrypoint ./trss-worker`.
ENV TRSS_WEB_BIND=0.0.0.0 \
    TRSS_WEB_PORT=8080 \
    TRSS_WEB_STATIC_DIR=/usr/local/share/trss/web
EXPOSE 8080

ENTRYPOINT [ "./transmission-rss" ]
