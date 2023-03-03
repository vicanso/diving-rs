# diving-rs

[中文](./README-zh.md)

Exploring each layer in a docker image, it's fast and simple. There are two modes: terminal(default) and web. 
It does not depend on anything, including docker client.

## config

The config file is `~/.diving/config.yml`, the options:

- `layer_path`: The path of layer cache, default is `~/.diving/layers`
- `layer_ttl`: The ttl of layer, default is `90d`

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
docker run -d --restart=always \
  -p 7001:7001 \
  -v $PWD/diving:/home/rust/.diving \
  --name diving \
  vicanso/diving
```

Open `http://127.0.0.1:7001/` in the browser.

![](./assets/diving-web.png)