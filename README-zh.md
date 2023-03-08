# diving-rs

用于展示docker镜像的每一层文件列表，它更快更简单，使用rust语言开发。它支持两种模式：命令行（默认模式）以及web模式，无需依赖任何东西，包括docker客户端。

`diving-rs`支持多个平台，包括：linux，windows，macos，可以在[release page](https://github.com/vicanso/diving-rs/releases)下载获取。

需要注意：由于镜像分层数据需要从镜像源下载，如docker hub，下载时长需要较长时间，如果超时则再次尝试即可，建议下载程序在本机执行。而对于私有化部署的镜像源，则可将diving的镜像部署运行在可访问镜像源的机器即可。

## config

默认配置文件为`~/.diving/config.yml`，其配置选项如下：

- `layer_path`: 分层数据缓存的目录，默认为`~/.diving/layers`
- `layer_ttl`: 分层数据缓存的有效期, 默认为`90d`

## terminal

```bash
diving redis:alpine

diving quay.io/prometheus/node-exporter
```

- `Current Layer Contents` 仅显示当前层的所有文件
- `Press 1` 仅显示当前`修改或删除` 的文件
- `Press 2` 仅显示当前层大于1MB的文件
- `Press Esc or 0` 重置显示模式 

![](./assets/diving-terminal.gif)

## web

```bash
docker run -d --restart=always \
  -p 7001:7001 \
  -v $PWD/diving:/home/rust/.diving \
  --name diving \
  vicanso/diving
```

在浏览器中打开`http://127.0.0.1:7001/`即可。

![](./assets/diving-web.png)