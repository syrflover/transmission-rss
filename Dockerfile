# Frontend build. Only this stage has Node/bun; the final image has neither.
FROM oven/bun:1 AS web-builder

WORKDIR /usr/src/web

COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile

COPY web/ ./
RUN bun run build


# Rust build: `trss-web` (web server), `trss-worker` (long-running collection
# worker), `trss-extract` (the child process the worker unpacks a received
# archive in, beside it) and `trss-probe` (the file probe, ticket 0060).
FROM clux/muslrust:stable AS builder

WORKDIR /usr/src/transmission-rss

COPY Cargo.toml Cargo.lock ./
# The SQLite build options (`LIBSQLITE3_FLAGS`).
COPY .cargo ./.cargo
COPY crates ./crates

# `--locked`: build the dependency versions the tests ran against.
RUN cargo build --release --locked


# The probe alone, with `trss-extract` for its --unpack, for a host that runs a
# release without them (deploy/probe.sh mounts them into the release's image).
# Not part of the image: build them with
#   docker build --target probe-binary --output type=local,dest=probe-out .
# which writes probe-out/trss-probe and probe-out/trss-extract, static
# executables.
FROM clux/muslrust:stable AS probe-builder

WORKDIR /usr/src/transmission-rss

COPY Cargo.toml Cargo.lock ./
# The SQLite build options (`LIBSQLITE3_FLAGS`).
COPY .cargo ./.cargo
COPY crates ./crates

RUN cargo build --release --locked -p trss-probe -p trss-jobs \
    --bin trss-probe --bin trss-extract

FROM scratch AS probe-binary

COPY --from=probe-builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-probe \
    /trss-probe
COPY --from=probe-builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-extract \
    /trss-extract


FROM alpine:edge

RUN apk update

WORKDIR /usr/local/bin

COPY --from=builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-web \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-worker \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-extract \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/trss-probe \
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
