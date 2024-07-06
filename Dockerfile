FROM clux/muslrust:stable as builder

WORKDIR /usr/src/transmission-rss

COPY . .

RUN cargo build --release

FROM alpine:edge

RUN apk update

WORKDIR /usr/local/bin

COPY --from=builder \
    /usr/src/transmission-rss/target/x86_64-unknown-linux-musl/release/transmission-rss .

ENTRYPOINT [ "./transmission-rss" ]
