# diving-rs

Exploring each layer in a docker image, it's fast and simple. There are two modes: terminal(default) and web.

## terminal

```bash
diving redis:alpine

diving quay.io/prometheus/node-exporter
```

- `Current Layer Contents` only show the files of current layer
- `Press 1` only show the `Modified/Removed` files of current layer
- `Press 2` only show the files >= 1MB
- `Press Esc or 0` reset the view mode

![](./assets/diving-terminal.gif)

## web

```bash
docker run -d --restart=always -p 7001:7001 vicanso/diving
```

Open `http://127.0.0.1:7001/` in the browser.

![](./assets/diving-web.png)