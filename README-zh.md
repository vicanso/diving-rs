# diving-rs

用于展示docker镜像的每一层文件列表，它更快更简单，使用rust语言开发。它支持两种模式：命令行（默认模式）以及web模式，无需依赖任何东西，包括docker客户端。

`diving-rs`支持多个平台，包括：linux，windows，macos，可以在[release page](https://github.com/vicanso/diving-rs/releases)下载获取。

需要注意：由于镜像分层数据需要从镜像源下载，如docker hub，下载时长需要较长时间。大分层下载中断时会自动重试并从断点续传，若仍失败再次尝试即可，建议下载程序在本机执行。而对于私有化部署的镜像源，则可将diving的镜像部署运行在可访问镜像源的机器即可。


## 安装


```bash
curl -fsSL https://raw.githubusercontent.com/vicanso/diving-rs/main/install.sh | sh
```


## config

默认配置文件为`~/.diving/config.yml`，其配置选项如下：

- `layer_path`: 分层数据缓存的目录，默认为`~/.diving/layers`
- `layer_ttl`: 分层数据缓存与分析结果缓存的有效期，默认为`90d`，若超过指定时间未再访问则该条目被清除
- `analysis_path`: 分析结果缓存目录，默认为`~/.diving/analysis`
- `cleanup_interval_hours`: 扫描并清除过期缓存的间隔时间（单位：小时），默认为`1`
- `threads`: 并发 layer 拉取 + 解压任务数，默认为 `min(layers.len(), 逻辑 CPU 数 × 2)`。在配置文件中显式设置优先级最高（网络快且层数多时可调大，机器还跑其它负载时可调小）
- `lowest_efficiency`: CI 检查——最低可接受的镜像效率（0–1），默认为`0.95`
- `highest_wasted_bytes`: CI 检查——最大允许的浪费字节数，默认为`20971520`（20 MB）
- `highest_user_wasted_percent`: CI 检查——最大允许的浪费比例（0–1），默认为`0.1`

`~/.diving/config.yml` 示例：

```yaml
layer_ttl: 30d
cleanup_interval_hours: 6
threads: 4
lowest_efficiency: 0.95
highest_wasted_bytes: 20971520
highest_user_wasted_percent: 0.1
```

## 缓存

diving 在 `~/.diving/` 下维护两层缓存，均受 `layer_ttl` 控制并每小时清理一次：

- **Layer blobs**（`~/.diving/layers/`）——从 registry 下载的压缩 layer，按 layer digest 索引。命中时省去网络下载，但解压与文件树构建仍会运行。
- **分析结果**（`~/.diving/analysis/`）——完整的 `DockerAnalyzeResult` JSON，以「`HEAD` 镜像 manifest 返回的 `Docker-Content-Digest` + 请求的架构」为键。命中时**整条流水线被短路**——不再拉 layer、不解压、不走文件树。

分析缓存是**内容寻址**的：当 `:latest` 这类可变 tag 被重新推送时，digest 自然变化，缓存条目自动失效。如果 `HEAD` 探测失败（网络、4xx/5xx、缺少 header 等任何原因），diving 会静默回退到完整分析流程——缓存绝不会阻塞请求。

## sensitive-files

分析过程中，diving 会对每个文件路径执行内置规则扫描（`.env` 文件、SSH 私钥、AWS 凭证、TLS 证书等），命中结果会以 **Security Warnings** 的形式出现在分析报告中。

可通过创建 `~/.diving/sensitive-files` 文件来扩展或屏蔽这些检查，每行一条规则：

| 行格式 | 作用 |
|--------|------|
| `<glob-pattern>` | 将匹配的文件标记为敏感（原因显示为 "Custom sensitive file"） |
| `<glob-pattern> \| <原因>` | 标记为敏感，并附加自定义原因说明 |
| `!<glob-pattern>` | 忽略/屏蔽匹配项（同时覆盖内置规则和上方自定义规则） |

`#` 开头及空行会被跳过。Glob 模式大小写不敏感；`*` 可跨目录分隔符匹配，同时也会对文件名单独匹配，因此 `*.pem` 能命中 `a/b/cert.pem`。

`~/.diving/sensitive-files` 示例：

```
# ── 额外规则 ─────────────────────────────────────────────────
**/*.vault-token | Vault token
**/app-secrets.json | 应用密钥

# ── 屏蔽内置规则中的误报 ──────────────────────────────────────
!**/.env.example
!**/.env.template
!**/certs/nginx.crt
!**/testdata/**
!**/fixtures/**
```

## terminal

镜像数据支持三种数据源模式，具体形式如下：

- `registry` 简写的形式为docker registry，私有或其它的registry则使用完整地址
- `docker` 基于本地安装了docker客户端的形式
- `file` 基于本地导出的tar包

```bash
diving redis:alpine

# 指定架构
diving redis:alpine?arch=arm64

diving quay.io/prometheus/node-exporter

diving docker://redis:alpine

diving file:///tmp/redis.tar

# CI 模式——输出效率评分，检查不通过时以退出码 1 退出
CI=true diving redis:alpine

# 将分析结果保存为 JSON 文件
diving redis:alpine --output-file result.json

# 将分析结果保存为 Markdown 格式（通过 .md 后缀自动识别）
diving redis:alpine --output-file result.md

# 将 Markdown 分析结果直接输出到控制台（默认已自动识别并隐藏基础镜像层）
diving myimage:latest --output-file -

# 加上 --no-skip-base 可包含基础镜像层
diving myimage:latest --output-file - --no-skip-base
```

- `Current Layer Contents` 仅显示当前层的所有文件
- `Press 1` 仅显示当前`修改或删除` 的文件
- `Press 2` 仅显示当前层大于1MB的文件
- `Press Esc or 0` 重置显示模式 

![](./assets/diving-terminal.gif)

## AI 分析

提供 OpenAI 兼容的 API Key 后，diving 会输出 AI 生成的优化分析报告，而不进入交互式 TUI。程序会将完整的 Markdown 分析（分层、反推的 Dockerfile、浪费空间、大文件、安全发现）发送给模型，并将其诊断结果打印到标准输出。当 ENTRYPOINT/CMD 指向镜像内的脚本时，会从分层中读取该脚本内容一并发送，便于模型审查容器实际运行的逻辑。

```bash
# 启用 AI 分析（打印报告，跳过 TUI）
diving redis:alpine --ai-api-key sk-xxxx

# 自定义 OpenAI 兼容的接口地址与模型
diving redis:alpine \
  --ai-api-key sk-xxxx \
  --ai-base-url https://your-gateway/v1 \
  --ai-model gpt-4o

# Key / 接口地址 / 模型也可通过环境变量提供
export OPENAI_API_KEY=sk-xxxx
diving redis:alpine

# 控制报告语言（同时影响终端/Markdown 输出）
diving redis:alpine --ai-api-key sk-xxxx --lang zh
```

| 参数 | 环境变量 | 默认值 | 说明 |
|------|----------|--------|------|
| `--ai-api-key` | `OPENAI_API_KEY` | — | OpenAI 兼容的 API Key。提供该参数即启用 AI 分析。 |
| `--ai-base-url` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | 接口地址，也可直接传入完整的 `.../chat/completions` 地址。 |
| `--ai-model` | `OPENAI_MODEL` | `gpt-4o` | 模型名称。 |
| `--lang` | `DIVING_LANG` | 系统语言 | 输出语言：`en` 或 `zh`。 |
| `--no-ai-history` | — | 关闭 | 跳过读取上一次快照，本次报告不做劣化对比；本次分析仍会作为新基线被记录。 |

每次运行会将本次分析快照保存到 `~/.diving/ai_history/`。下次分析同一镜像时，会把上一次的快照与本次一并发送给模型，便于其识别新老版本之间的体积劣化/膨胀。加上 `--no-ai-history` 可在单次运行中跳过该对比（例如基线已过期或与当前镜像无关）；快照仍会刷新，后续运行将以本次为基线对比。

## 企微推送

指定企业微信群机器人 webhook，即可把分析结果直接推送到群里，而不进入交互式 TUI。推送内容会智能选择，确保不超过机器人 ~4096 字节的 markdown 上限：

- 已设置 `--ai-api-key` → 推送精简的 AI 报告
- 未启用 AI → 推送精简摘要（效率评分、浪费空间、优化建议）

```bash
# 使用机器人 key（自动展开为标准 webhook 地址）
diving redis:alpine --wecom-webhook 693a91f6-7aoc-4bc4-97a0-0ec2sifa5aaa

# 或使用完整 webhook 地址
diving redis:alpine --wecom-webhook "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=KEY"

# 推送 AI 报告而非摘要
diving redis:alpine --ai-api-key sk-xxxx --wecom-webhook KEY

# webhook 也可通过环境变量提供
export WECOM_WEBHOOK=KEY
diving redis:alpine
```

| 参数 | 环境变量 | 默认值 | 说明 |
|------|----------|--------|------|
| `--wecom-webhook` | `WECOM_WEBHOOK` | — | 企业微信群机器人 webhook 地址，或裸 key（自动展开）。提供该参数即推送结果并跳过 TUI。 |

超长内容会截断到企微上限并附 `… (truncated)` 提示。

## web

```bash
# 创建数据目录并将所有权授予容器内用户（UID/GID 均为 1000）
mkdir -p $PWD/diving
chown -R 1000:1000 $PWD/diving

docker run -d --restart=always \
  -p 7001:7001 \
  -v $PWD/diving:/home/rust/.diving \
  --name diving \
  vicanso/diving
```

容器以非 root 身份（UID `1000:1000`）运行。上方的 `chown` 命令将宿主机目录的所有权交给该 UID，省略此步骤会导致容器无法写入 layer 缓存文件而启动失败。

镜像基于 `debian:bookworm-slim`（glibc 运行时，带 shell 与 apt 便于 `docker exec` 排查；构建期已安装 `ca-certificates` 与 `tzdata`）。由于默认不含 `wget`/`curl`，未设置 Dockerfile 级别的 `HEALTHCHECK`——请改用编排层探针访问 `GET /ping`（Kubernetes `livenessProbe`、Docker Compose 中的外部 `curl` sidecar 等），或在派生镜像里装 `wget`/`curl` 自行重启 in-image 探针。

如需修改监听地址，可通过 `--listen` 参数指定：

```bash
diving --mode web --listen 0.0.0.0:8080
```

在浏览器中打开`http://127.0.0.1:7001/`即可。

![](./assets/diving-web.png)

### API

#### `GET /api/analyze`

分析 Docker 镜像并返回结果。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `image` | string | 是 | 镜像引用（格式与命令行模式相同） |
| `format` | string | 否 | 设为 `markdown` 时返回 Markdown 报告，默认返回 JSON |
| `skipBase` | bool | 否 | 当 `format=markdown` 时，自动识别并隐藏基础镜像层（默认 `true`）；设为 `false` 则包含 |

**示例：**

```bash
# JSON 响应（默认）
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine"

# 指定架构
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine%3Farch%3Darm64"

# Markdown 报告
curl "http://127.0.0.1:7001/api/analyze?image=redis:alpine&format=markdown"

# Markdown 报告并包含基础镜像层（默认隐藏）
curl "http://127.0.0.1:7001/api/analyze?image=myimage:latest&format=markdown&skipBase=false"
```