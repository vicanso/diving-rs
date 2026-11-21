# diving-rs 优化与迭代路线图

> 更新时间：2026-11-21。
> 定位：**Diving Desktop 体验优先 + `diving-core` / `diving-web` 高性价比补齐**。
> 边界：聚焦**镜像体积、层结构、效率与轻量安全卫生**，不做重型 SBOM / 漏洞库 / 供应链合规。

---

## Phase 1：止血与基础体验（已完成 ✅）

### Diving Desktop (`app/`)
- [x] **修复重新分析时镜像名与旧结果错位**（[state.rs:367-368](../app/src/domain/state.rs#L367-L368)、[title_bar.rs:276-288](../app/src/features/title_bar.rs#L276-L288)）：引入 `pending_image`，将结果关联的 `image_ref` 独立保存，文件预览与导出始终从当前结果取 ref；取消或失败不污染已有结果。
- [x] **全局进度条与取消能力**（[landing.rs:64-71](../app/src/features/landing.rs#L64-L71)、[service.rs:226-238](../app/src/domain/service.rs#L226-L238)、[state.rs:310-314](../app/src/domain/state.rs#L310-L314)）：Inspector 重新分析时展示进度条与取消按钮；Diff 接入双侧 `on_progress` 与取消按钮；`navigate_home` 同时中止 `active_diff_task`。
- [x] **避免窗口缩放与错误状态触发整树重建**（[inspector/mod.rs:66](../app/src/features/inspector/mod.rs#L66)、[findings.rs:40](../app/src/features/inspector/findings.rs#L40)、[treemap.rs:241-249](../app/src/features/inspector/treemap.rs#L241-L249)）：避免在顶层 `dynamic` 中直接读取 `window().width` 或 `analyze_error` 导致整个 Inspector / Findings / Treemap 重建。
- [x] **修复 Diff `Mod+R`、Stale Retry 与配置回退**（[state.rs:817-831](../app/src/domain/state.rs#L817-L831)、[inspector/mod.rs:94](../app/src/features/inspector/mod.rs#L94)、[service.rs:85-118](../app/src/domain/service.rs#L85-L118)）：Diff 刷新跳过内存缓存；Stale Retry 针对当前结果镜像重试且不回退旧缓存；配置加载失败时保留 `use_docker_config(true)` 并展示提示横幅。

### `diving-web` & `diving` TUI
- [x] **Web 安全加固**（[analyzer.rs:381](../crates/diving-core/src/analyzer.rs#L381)、[registry.rs:184](../crates/diving-core/src/image/registry.rs#L184)、[controller.rs:23-29](../crates/diving-web/src/controller.rs#L23-L29)）：
  - Web 模式默认拒绝私网/回环 IP（防 SSRF），并校验 `WWW-Authenticate` realm 与重定向主机。
  - 增加 manifest/config 大小、层数、解压体积、tar 条目数及并发分析/webhook 任务上限。
  - `/api/*` 增加可选 API token 鉴权；`/api/file` 改为流式响应。
- [x] **TUI 健壮性修复**（[ui/mod.rs:160-176](../crates/diving/src/ui/mod.rs#L160-L176)、[ui/mod.rs:259](../crates/diving/src/ui/mod.rs#L259)、[ui/layer_detail.rs:26](../crates/diving/src/ui/layer_detail.rs#L26)）：SIGCONT 改用 `try_send`；0 层镜像防御；本地时间转换避免 `.single().unwrap()` 在夏令时 panic。

---

## Phase 2：Desktop 顺手度与基础功能对齐（已完成 ✅）

### 性能与交互
- [x] **文件树搜索提速**（[file_tree.rs:36-45](../app/src/features/inspector/file_tree.rs#L36-L45)、[model.rs:342-359](../app/src/domain/model.rs#L342-L359)）：关键字输入防抖，每层预计算 lowercase 路径与子树标记，避免每键 O(n·depth) 分配与全量克隆 key。
- [x] **Treemap 缓存与异步导出**（[treemap.rs:28](../app/src/features/inspector/treemap.rs#L28)、[service.rs:316-322](../app/src/domain/service.rs#L316-L322)、[state.rs:749](../app/src/domain/state.rs#L749)）：每个结果只构建一次 final tree（放到 Tokio 算）；Markdown 导出异步化、层文件导出流式写入；统一错误/成功 toast 通知。
- [x] **真实进度事件**（[state.rs:461-490](../app/src/domain/state.rs#L461-L490)）：`diving-core` 增加层完成/字节进度事件，解决并发下载时进度条瞬间跳到 ~95%。
- [x] **键盘导航与 i18n 补全**（[file_tree.rs:389-415](../app/src/features/inspector/file_tree.rs#L389-L415)、[layer_timeline.rs:132-150](../app/src/features/inspector/layer_timeline.rs#L132-L150)）：文件树与层列表支持方向键、Enter/Space 展开与预览；清理硬编码英文与未翻译文案。

### 基础功能对齐
- [x] **真实多平台选择**（[title_bar.rs:404](../app/src/features/title_bar.rs#L404)）：Inspector 平台选择器基于 `res.supported_archs` 动态生成。
- [x] **展示已有分析数据**：展示 `envs`、`labels`、`unstripped_binaries` 与推荐项 `heuristic` 标记；Findings 取消 `.take(40)` 静默截断，提供"显示全部"。
- [x] **Recents 管理**：支持单条删除、清空列表，并标出 stale 记录；修复 `analyzed_at` 时区显示。
- [x] **JSON 报告导出**：与 Markdown 导出并列，输出标准 `AnalyzeReport` JSON。

> 工程原则：`state.rs`、`diff.rs`、`model.rs` 随改随拆，不单独停下来做大重构。

---

## Phase 3：核心高价值能力 + Desktop 三件套（已完成 ✅）

### `diving-core` 性能与能力
- [x] **单次扫描合并（Base-OS 与小文件 Secret）**（[layer.rs](../crates/diving-core/src/image/layer.rs)、[scan.rs](../crates/diving-core/src/image/scan.rs)、[pipeline.rs](../crates/diving-core/src/image/pipeline.rs)）：在 tar 单次解压流中直接捕获 `os-release`（对本地与远程镜像均生效，跳过额外 `probe_base_os` 解压）并完成小文件内容 Secret 扫描。
- [x] **小文件内容级 Secret 扫描**（[sensitive.rs](../crates/diving-core/src/sensitive.rs)）：对 ≤1 MiB 非二进制文本文件无正则检测 PEM 私钥、AWS Access Key、GitHub/Slack/Stripe/Google Token 及 `.env`/配置文件硬编码秘钥（自动忽略占位符）。
- [x] **核心性能优化**：`Analyzer::diff` 改为 `tokio::try_join!` 并行（[analyzer.rs](../crates/diving-core/src/analyzer.rs)）；分析缓存改用 zstd 压缩写入并兼容读取历史纯 JSON 缓存（[analysis_cache.rs](../crates/diving-core/src/store/analysis_cache.rs)）。
- [x] **纯 HTTP 私有/本地仓库支持**（[image_ref.rs](../crates/diving-core/src/image/image_ref.rs)）：保留显式 `http://` 协议（如 `http://localhost:5000/app:v1`）。
- [x] **ImageConfig 与 Dockerfile 规则增强**（[oci_image.rs](../crates/diving-core/src/image/oci_image.rs)、[recommend.rs](../crates/diving-core/src/recommend.rs)、[i18n.rs](../crates/diving-core/src/i18n.rs)）：解析 `ExposedPorts`、`Healthcheck`、`WorkingDir`、`Volumes`、`StopSignal`；新增 `nohealthcheck` 与 `cacheorder`（`COPY .` 位于依赖安装前）中英文规则。

### Diving Desktop 新功能三件套
- [x] **拖放导入（Drag & Drop）**（[lib.rs](../app/src/lib.rs)、[state.rs](../app/src/domain/state.rs)）：根窗口接入 `on_file_drop`，拖入 `.tar` 或 OCI 目录自动解码 URI 并按 `file://` 启动分析。
- [x] **文件跨层溯源 / 全局搜索**（[provenance_dialog.rs](../app/src/features/provenance_dialog.rs)、[model.rs](../app/src/domain/model.rs)、[file_tree.rs](../app/src/features/inspector/file_tree.rs)）：支持跨所有层搜索文件路径，展示每层的增删改操作与 Dockerfile 指令，点击直达对应层与文件。
- [x] **优化建议节省估算（What-if）**（[model.rs](../app/src/domain/model.rs)、[findings.rs](../app/src/features/inspector/findings.rs)）：基于去重净可回收空间，在 Findings 顶部展示"预计节省 → 预计镜像体积与效率"横幅。

---

## 延后（Later）

- **发 0.13 / 1.0 前**：`diving-core` 公开 API 规范化（`#[non_exhaustive]`、Cargo features 拆分 `goblin`/`zstd`/凭证 helper、解耦 `reqwest::Client`、完善 rustdoc、补齐 registry/401/Range/429 集成测试与 fuzz 测试）。
- **按需再看**：
  - Desktop：CI Gate 结论卡片、内置 AI 报告、私有仓库凭证管理界面、Diff 文本逐行对比、仓库 tag 列表浏览、同一 tag 历史时间线、深度 VoiceOver/a11y 审查。
  - CI/Web：SARIF / JUnit / GitLab Code Quality 导出、兼容 `.dive-ci`、GitHub Action PR 评论机器人、Web `/metrics` 与持久化 webhook 队列。

---

## 明确不做（Won't-Do）

- 重型 SBOM（CycloneDX / SPDX）、多语言包数据库解析与漏洞扫描对接（交给 Trivy / Syft / Grype）。
- 供应链签名 / SLSA attestation（cosign / OCI Referrers）与 License 合规审计。
- 低价值周边：系统通知/菜单栏图标、Deep link（`diving://`）、单文件 HTML 报告导出、Web 多镜像历史看板。
