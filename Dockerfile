FROM node:18-alpine as webbuilder

COPY . /diving-rs
RUN apk update \
  && apk add git make \
  && cd /diving-rs \
  && make build-web

FROM rust:alpine as builder

COPY --from=webbuilder /diving-rs /diving-rs

RUN apk update \
  && apk add git make build-base pkgconfig openssl \
  && cd /diving-rs \
  && make release 

FROM alpine 

EXPOSE 7001

# tzdata 安装所有时区配置或可根据需要只添加所需时区

RUN addgroup -g 1000 rust \
  && adduser -u 1000 -G rust -s /bin/sh -D rust \
  && apk add --no-cache ca-certificates tzdata

COPY --from=builder /diving-rs/target/release/diving /usr/local/bin/diving
COPY --from=builder /diving-rs/entrypoint.sh /entrypoint.sh

USER rust

WORKDIR /home/rust

HEALTHCHECK --timeout=10s --interval=10s CMD [ "wget", "http://127.0.0.1:7001/ping", "-q", "-O", "-"]

CMD ["diving", "--mode", "web"]

ENTRYPOINT ["/entrypoint.sh"]
