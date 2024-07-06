FROM clux/muslrust:stable as builder

WORKDIR /usr/src/transmission-rss

COPY . .

RUN cargo build --release

FROM alpine:edge

RUN apk update

WORKDIR /usr/local/bin

COPY --from=builder /usr/src/transmission-rss/target/release/transmission-rss .

ENTRYPOINT [ "./transmission-rss" ]
