FROM clux/muslrust:stable as builder

WORKDIR /usr/src/transmission-rss

RUN cargo build --release

FROM alpine:edge

RUN apk update

COPY --from=builder /usr/src/transmission-rss/target/release/transmission-rss /transmission-rss

ENTRYPOINT [ "/transmission-rss" ]
