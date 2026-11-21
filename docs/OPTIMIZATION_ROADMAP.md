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

## Phase 2：Desktop 顺手度与基础功能对齐

### 性能与交互
- [ ] **文件树搜索提速**（[file_tree.rs:36-45](../app/src/features/inspector/file_tree.rs#L36-L45)、[model.rs:342-359](../app/src/domain/model.rs#L342-L359)）：关键字输入防抖，每层预计算 lowercase 路径与子树标记，避免每键 O(n·depth) 分配与全量克隆 key。
- [ ] **Treemap 缓存与异步导出**（[treemap.rs:28](../app/src/features/inspector/treemap.rs#L28)、[service.rs:316-322](../app/src/domain/service.rs#L316-L322)、[state.rs:749](../app/src/domain/state.rs#L749)）：每个结果只构建一次 final tree（放到 Tokio 算）；Markdown 导出异步化、层文件导出流式写入；统一错误/成功 toast 通知。
- [ ] **真实进度事件**（[state.rs:461-490](../app/src/domain/state.rs#L461-L490)）：`diving-core` 增加层完成/字节进度事件，解决并发下载时进度条瞬间跳到 ~95%。
- [ ] **键盘导航与 i18n 补全**（[file_tree.rs:389-415](../app/src/features/inspector/file_tree.rs#L389-L415)、[layer_timeline.rs:132-150](../app/src/features/inspector/layer_timeline.rs#L132-L150)）：文件树与层列表支持方向键、Enter/Space 展开与预览；清理硬编码英文与未翻译文案。

### 基础功能对齐
- [ ] **真实多平台选择**（[title_bar.rs:404](../app/src/features/title_bar.rs#L404)）：Inspector 平台选择器基于 `res.supported_archs` 动态生成。
- [ ] **展示已有分析数据**：展示 `envs`、`labels`、`unstripped_binaries` 与推荐项 `heuristic` 标记；Findings 取消 `.take(40)` 静默截断，提供"显示全部"。
- [ ] **Recents 管理**：支持单条删除、清空列表，并标出 stale 记录；修复 `analyzed_at` 时区显示。
- [ ] **JSON 报告导出**：与 Markdown 导出并列，输出标准 `AnalyzeReport` JSON。

> 工程原则：`state.rs`、`diff.rs`、`model.rs` 随改随拆，不单独停下来做大重构。

---

## Phase 3：核心高价值能力 + Desktop 三件套

### `diving-core` 性能与能力
- [ ] **单次扫描合并（Fused Pass）**（[oci_image.rs:467](../crates/diving-core/src/image/oci_image.rs#L467)、[elf.rs:107](../crates/diving-core/src/image/elf.rs#L107)、[pipeline.rs:168](../crates/diving-core/src/image/pipeline.rs#L168)）：将文件树扫描、去重哈希、ELF 探测、base-OS 探测合并为单次解压流；同步验证并修复 `file://` / `docker://` / `podman://` 的去重与 ELF 探测问题。
- [ ] **小文件内容级 Secret 扫描**：在单次扫描流中对小文本文件（≤1MB）检测 AWS/GCP key、私钥、JWT、`.env` 等敏感凭据（补齐 [sensitive.rs](../crates/diving-core/src/sensitive.rs) 目前仅匹配路径的缺口）。
- [ ] **核心性能优化**：`Analyzer::diff` 改为 `try_join!` 并行（[analyzer.rs:560-561](../crates/diving-core/src/analyzer.rs#L560-L561)）；分析缓存使用 zstd 压缩；`add_file` 宽目录（如 `node_modules`）增加 HashMap 索引；`FileTreeItem.mode` 优化存储。
- [ ] **私有/本地仓库支持**：支持纯 HTTP / insecure registry（如 `localhost:5000`）、registry mirror 与显式代理配置。
- [ ] **ImageConfig 与 Dockerfile 规则增强**：解析 `ExposedPorts`、`Healthcheck`、`WorkingDir`、`Volumes`、`StopSignal`；补充层缓存友好度、`.git` 残留、`ADD <url>`、`latest` 标签、多阶段构建建议等规则。

### Diving Desktop 新功能三件套
- [ ] **拖放导入（Drag & Drop）**：把 `.tar`（docker save / OCI）或 OCI layout 目录直接拖入窗口按 `file://` 分析。
- [ ] **文件跨层溯源 / 全局搜索**：输入任意路径，查看其在各层的添加、修改、删除历史及对应 Dockerfile 指令。
- [ ] **优化建议节省估算（What-if）**：为每条建议给出预计节省空间，汇总"全部采纳后的预计镜像体积与效率"。

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
