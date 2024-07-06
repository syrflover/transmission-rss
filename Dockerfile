FROM clux/muslrust:stable as builder

RUN cargo build --release

FROM alpine:edge

RUN apk update

COPY --from=builder /target/release/transmission-rss /transmission-rss

ENTRYPOINT [ "/transmission-rss" ]
