# Frontend build. Only this stage has Node/bun; the final image has neither.
FROM oven/bun:1 AS web-builder

WORKDIR /usr/src/web

COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile

COPY web/ ./
RUN bun run build


# Rust build: `trss-web` (web server) and `trss-worker` (long-running
# collection worker).
FROM clux/muslrust:stable AS builder

WORKDIR /usr/src/transmission-rss

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# `--locked`: build the dependency versions the tests ran against.
RUN cargo build --release --locked


FROM alpine:edge

RUN apk update

WORKDIR /usr/local/bin

COPY --from=builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-web \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-worker \
    ./

# Static frontend that `trss-web` serves (no Node server at runtime).
COPY --from=web-builder /usr/src/web/dist /usr/local/share/trss/web

# Defaults for `trss-web`; `trss-worker` ignores them.
# The image runs the worker; run the web server by overriding the entrypoint
# with `--entrypoint /usr/local/bin/trss-web` (docker-compose.trss.yml sets the
# entrypoint of both services this way).
ENV TRSS_WEB_BIND=0.0.0.0 \
    TRSS_WEB_PORT=8080 \
    TRSS_WEB_STATIC_DIR=/usr/local/share/trss/web
EXPOSE 8080

# Not root: 1000:1000, the user Transmission writes the media as, so that what
# trss writes there has the same owner (the players read it over NFS, which
# shows the owner as numbers). The compose file's `user:` sets another uid and
# gid from TRSS_UID and TRSS_GID. The data folder must belong to this user:
# both binaries check at their start that they can write it.
USER 1000:1000

ENTRYPOINT [ "/usr/local/bin/trss-worker" ]
