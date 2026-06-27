# diving-rs

[![Release](https://img.shields.io/github/v/release/vicanso/diving-rs)](https://github.com/vicanso/diving-rs/releases)
[![Docker Pulls](https://img.shields.io/docker/pulls/vicanso/diving)](https://hub.docker.com/r/vicanso/diving)
[![License](https://img.shields.io/github/license/vicanso/diving-rs)](./LICENSE)

[English](./README.md)

**深入 Docker 镜像的每一层 —— 几秒钟定位浪费空间、泄漏密钥与体积膨胀。**

一个快速、独立的 Rust 二进制：直接从任意 registry 拉取镜像，看清里面到底装了什么。**无需 Docker daemon、无需 root、零依赖。** 支持 Linux、macOS 与 Windows。

![](./assets/diving-terminal.gif)

## 为什么选 diving？

- ⚡ **快且独立** —— 单个静态二进制。可从 Docker Hub / 任意 V2 registry、本地 docker 客户端或 `.tar` 文件读取镜像。分层自动缓存，下载中断自动续传。
- 🔍 **逐层文件浏览** —— 交互式 TUI 遍历每一层的文件系统，新增 / 修改 / 删除的文件以颜色区分。
- 📉 **浪费与膨胀检测** —— 效率评分、浪费字节、跨层重复文件、超大层、包管理器缓存、开发/构建产物，并反推 Dockerfile 做反模式 lint。
- 🛡️ **安全卫生检查** —— 标记泄漏的密钥文件（`.env`、SSH / 云厂商密钥、证书）、`ENV`/label/Dockerfile 中的硬编码凭证（仅报字段名，绝不回显值）、setuid 与全局可写文件，以及以 root 运行的容器。
- 🤖 **AI 优化报告** —— 把完整分析交给任意 OpenAI 兼容模型，得到按优先级排序的修复清单，并支持版本间的劣化对比。
- 🚦 **CI 卡口** —— 镜像效率 / 浪费字节低于阈值时让流水线失败。
- 🌐 **终端 · Web · JSON / Markdown · 企微** —— 交互浏览、暴露 HTTP API、导出报告，或推送到群聊。

> **能力边界说明：** diving 聚焦于体积、结构与基础安全检查（密钥泄漏、文件权限、是否以 root 运行等）。它扫描文件**路径**与镜像元数据 —— **不做** CVE/漏洞扫描，也**不扫描文件内容**。漏洞覆盖请配合 Trivy / grype / docker scout 使用。

## 快速开始

```bash
# 1. 安装 —— 任选其一：
curl -fsSL https://raw.githubusercontent.com/vicanso/diving-rs/main/install.sh | sh   # 预编译二进制
cargo install diving                                                                  # 从 crates.io 安装

# 2. 开始分析
diving redis:alpine
```

就这么简单 —— 无需 Docker daemon。Linux / macOS / Windows 的预编译二进制也可在 [release page](https://github.com/vicanso/diving-rs/releases) 下载；也可用 `cargo install --git https://github.com/vicanso/diving-rs` 从源码安装最新版。

TUI 内快捷键：

| 按键 | 作用 |
|------|------|
| `1` | 仅显示当前层 `修改` / `删除` 的文件 |
| `2` | 仅显示 ≥ 1 MB 的文件 |
| `Esc` / `0` | 重置显示模式 |

## 分析任意镜像

diving 支持三种数据源：

```bash
# 来自 registry（默认）—— Docker Hub、quay.io、私有 registry…
diving redis:alpine
diving quay.io/prometheus/node-exporter

# 为多架构镜像指定架构
diving redis:alpine?arch=arm64

# 来自本地 docker 客户端
diving docker://redis:alpine

# 来自导出的 tar 包
diving file:///tmp/redis.tar
```

## 导出报告

```bash
# JSON
diving redis:alpine --output-file result.json

# Markdown（通过 .md 后缀自动识别）
diving redis:alpine --output-file result.md

# 将 Markdown 输出到控制台 —— 默认自动识别并隐藏基础镜像层
diving myimage:latest --output-file -

# 包含基础镜像层
diving myimage:latest --output-file - --no-skip-base
```

## CI 卡口

在 CI 中运行 diving 以保持镜像精简。设置 `CI=true` 后，它会输出效率评分，并在任一阈值超标时**以退出码 `1` 退出**。

```bash
CI=true diving redis:alpine
```

阈值可在 `~/.diving/config.yml` 中配置：

| 选项 | 默认值 | 含义 |
|------|--------|------|
| `lowest_efficiency` | `0.95` | 最低可接受的效率评分（0–1） |
| `highest_wasted_bytes` | `20971520`（20 MB） | 最大允许浪费字节数 |
| `highest_user_wasted_percent` | `0.1` | 最大允许浪费比例（0–1） |

## AI 分析

提供 OpenAI 兼容的 API Key 后，diving 会将完整的 Markdown 分析（分层、反推的 Dockerfile、浪费空间、大文件、安全发现）发送给模型，并打印按优先级排序的优化报告，而不进入交互式 TUI。当 `ENTRYPOINT`/`CMD` 指向镜像内脚本时，会从分层读取该脚本一并发送，便于模型审查容器实际运行的逻辑。

```bash
# 启用 AI 分析（打印报告，跳过 TUI）
diving redis:alpine --ai-api-key sk-xxxx

# 自定义接口地址与模型
diving redis:alpine \
  --ai-api-key sk-xxxx \
  --ai-base-url https://your-gateway/v1 \
  --ai-model gpt-4o

# 通过环境变量配置
export OPENAI_API_KEY=sk-xxxx
diving redis:alpine

# 控制报告语言（同时影响终端 / Markdown 输出）
diving redis:alpine --ai-api-key sk-xxxx --lang zh
```

| 参数 | 环境变量 | 默认值 | 说明 |
|------|----------|--------|------|
| `--ai-api-key` | `OPENAI_API_KEY` | — | OpenAI 兼容的 API Key。提供该参数即启用 AI 分析。 |
| `--ai-base-url` | `OPENAI_BASE_URL` | `https://api.openai.com/v1` | 接口地址，也可直接传入完整的 `.../chat/completions` 地址。 |
| `--ai-model` | `OPENAI_MODEL` | `gpt-4o` | 模型名称。 |
| `--ai-system-prompt` | `OPENAI_SYSTEM_PROMPT` | 内置 DevSecOps 模板 | 覆盖系统提示词，完全替换内置模板。 |
| `--lang` | `DIVING_LANG` | 系统语言 | 输出语言：`en` 或 `zh`。 |
| `--no-ai-history` | — | 关闭 | 本次跳过劣化对比（快照仍会刷新）。 |

每次运行会将本次分析快照保存到 `~/.diving/ai_history/`。下次分析同一镜像时，会把上一次快照与本次一并发送给模型，便于识别新老版本间的体积劣化/膨胀。`--no-ai-history` 可在单次运行中跳过该对比（例如基线已过期）；快照仍会刷新，后续运行将以本次为基线。

> 安全提示：API Key、接口地址与 webhook **仅支持 CLI/环境变量** —— 不会作为 web 查询参数接收，因此不会落入访问日志。

## 企微推送

指定企业微信群机器人 webhook，即可把结果直接推送到群里，而不进入交互式 TUI。推送内容会智能选择，确保不超过机器人 ~4096 字节的 markdown 上限：

- 已设置 `--ai-api-key` → 推送精简的 AI 报告
- 未启用 AI → 推送精简摘要（效率评分、浪费空间、优化建议）

```bash
# 使用机器人 key（自动展开为标准 webhook 地址）
diving redis:alpine --wecom-webhook 693a91f6-7aoc-4bc4-97a0-0ec2sifa5aaa

# 或使用完整 webhook 地址
diving redis:alpine --wecom-webhook "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=KEY"

# 推送 AI 报告而非摘要
diving redis:alpine --ai-api-key sk-xxxx --wecom-webhook KEY

# 也可通过环境变量提供
export WECOM_WEBHOOK=KEY
diving redis:alpine
```

| 参数 | 环境变量 | 默认值 | 说明 |
|------|----------|--------|------|
| `--wecom-webhook` | `WECOM_WEBHOOK` | — | 企业微信群机器人 webhook 地址，或裸 key（自动展开）。提供该参数即推送结果并跳过 TUI。 |

超长内容会截断到企微上限并附 `… (truncated)` 提示。

## Web 模式

将 diving 作为带 React 前端的 HTTP 服务运行，用于远程分析。

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

在浏览器中打开 `http://127.0.0.1:7001/` 即可。

![](./assets/diving-web.png)

容器以非 root 身份（UID `1000:1000`）运行；上方的 `chown` 让它能写入 layer 缓存（省略会导致启动失败）。镜像基于 `debian:bookworm-slim`，带 `ca-certificates` 与 `tzdata`。由于默认不含 `wget`/`curl`，未设置 in-image `HEALTHCHECK` —— 请改用编排层探针访问 `GET /ping`（Kubernetes `livenessProbe`、sidecar 等）。

通过 `--listen` 修改监听地址：

```bash
diving --mode web --listen 0.0.0.0:8080
```

### API

#### `GET /api/analyze`

分析 Docker 镜像并返回结果。

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `image` | string | 是 | 镜像引用（格式与命令行模式相同） |
| `format` | string | 否 | 设为 `markdown` 时返回 Markdown 报告，默认返回 JSON |
| `skipBase` | bool | 否 | 当 `format=markdown` 时，自动识别并隐藏基础镜像层（默认 `true`）；设为 `false` 则包含 |

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

## 敏感文件扫描

分析过程中，diving 会对每个文件**路径**执行内置规则扫描（`.env` 文件、SSH 私钥、AWS/GCP 凭证、TLS 私钥、kubeconfig、`.htpasswd`、误拷入的 `.git` 目录等），命中结果以 **Security Warnings** 形式出现在报告中。（它扫描路径与元数据，不扫描文件内容。）

可通过创建 `~/.diving/sensitive-files` 扩展或屏蔽规则，每行一条：

| 行格式 | 作用 |
|--------|------|
| `<glob-pattern>` | 将匹配文件标记为敏感（原因显示为 "Custom sensitive file"） |
| `<glob-pattern> \| <原因>` | 标记为敏感并附加自定义原因 |
| `!<glob-pattern>` | 忽略/屏蔽匹配项（同时覆盖内置规则与上方自定义规则） |

`#` 开头及空行会被跳过。Glob 大小写不敏感；`*` 可跨目录分隔符匹配，同时也会对文件名单独匹配，因此 `*.pem` 能命中 `a/b/cert.pem`。

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

## 配置

配置文件：`~/.diving/config.yml`。

| 选项 | 默认值 | 说明 |
|------|--------|------|
| `layer_path` | `~/.diving/layers` | layer blob 缓存目录 |
| `layer_ttl` | `90d` | layer blob **与**分析结果缓存的有效期；超过该时长未访问则清除 |
| `analysis_path` | `~/.diving/analysis` | 分析结果缓存目录 |
| `cleanup_interval_hours` | `1` | 扫描并清除过期缓存的间隔（小时） |
| `threads` | `min(层数, 2 × CPU 数)` | 并发 layer 拉取 + 解压任务数。网络快且层数多时调大，与其它负载共享主机时调小 |
| `lowest_efficiency` | `0.95` | CI 检查 —— 最低效率评分（0–1） |
| `highest_wasted_bytes` | `20971520` | CI 检查 —— 最大浪费字节数（20 MB） |
| `highest_user_wasted_percent` | `0.1` | CI 检查 —— 最大浪费比例（0–1） |

```yaml
layer_ttl: 30d
cleanup_interval_hours: 6
threads: 4
lowest_efficiency: 0.95
highest_wasted_bytes: 20971520
highest_user_wasted_percent: 0.1
```

## 缓存机制

diving 在 `~/.diving/` 下维护两层缓存，均受 `layer_ttl` 控制并每小时清理一次：

- **Layer blobs**（`~/.diving/layers/`）—— 从 registry 下载的压缩 layer，按 layer digest 索引。命中时省去网络下载，但解压与文件树构建仍会运行。
- **分析结果**（`~/.diving/analysis/`）—— 完整的分析结果，以「`HEAD` 镜像 manifest 返回的 `Docker-Content-Digest` + 架构」为键。命中时整条流水线被短路。

分析缓存是**内容寻址**的：`:latest` 这类可变 tag 被重新推送时，digest 变化，缓存条目自动失效。如果 `HEAD` 探测因任何原因失败，diving 会静默回退到完整分析 —— 缓存绝不会阻塞请求。

> 由于分层数据需从镜像源（如 Docker Hub）下载，首次分析大镜像可能较慢。下载中断会自动续传。对于私有化部署的 registry，请将 diving（或其 web 镜像）运行在可访问该 registry 的主机上。

## 许可证

基于 [Apache License 2.0](./LICENSE) 开源。
