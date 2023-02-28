# diving-rs

Exploring each layer in a docker image, it's fast and simple. There ary two modes: terminal(default) and web.

## terminal

```bash
diving redis:alpine

diving quay.io/prometheus/node-exporter
```

## web

```bash
docker run -d --restart=always -p 7001:7001 vicanso/diving
```

![](./assets/diving-web.png)