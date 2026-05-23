//! Minimal, dependency-free localization for generated text.
//!
//! Scope (phase 1): the optimization-recommendation strings produced by
//! [`crate::recommend`]. Localization happens at *generation* time — the
//! language is resolved once and the recommendation text is produced already
//! translated, so every render surface (CLI, Markdown, terminal, web) stays
//! unchanged.
//!
//! The English catalog is byte-identical to the original hardcoded literals,
//! so `Lang::En` reproduces the previous output exactly.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    /// Map a locale tag / language name to a supported language.
    /// Anything that looks Chinese (`zh`, `cn`, `中文`) → Zh, else En.
    fn from_tag(tag: &str) -> Lang {
        let t = tag.to_lowercase();
        if t.contains("zh") || t.contains("cn") || tag.contains('中') {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    /// Resolve the language: an explicit value wins, otherwise the standard
    /// locale env vars are consulted, otherwise English.
    pub fn resolve(explicit: Option<&str>) -> Lang {
        if let Some(v) = explicit {
            if !v.trim().is_empty() {
                return Lang::from_tag(v);
            }
        }
        for key in ["DIVING_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(key) {
                if !v.is_empty() {
                    return Lang::from_tag(&v);
                }
            }
        }
        Lang::En
    }
}

/// Substitute `{0}`, `{1}`, … placeholders. The catalog templates contain no
/// literal braces, so a plain replace is sufficient and dependency-free.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut s = template.to_string();
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), a);
    }
    s
}

/// Look up a catalog entry. Unknown keys return a visible marker (loud but
/// safe to render).
pub fn tr(lang: Lang, key: &str) -> &'static str {
    match (lang, key) {
        // ---- recommendation titles ----------------------------------------
        (Lang::En, "rec.wasted.title") => "Reclaim wasted space",
        (Lang::Zh, "rec.wasted.title") => "回收浪费的空间",
        (Lang::En, "rec.pkgcache.title") => "Remove package manager cache",
        (Lang::Zh, "rec.pkgcache.title") => "清理包管理器缓存",
        (Lang::En, "rec.devart.title") => "Exclude development artifacts",
        (Lang::Zh, "rec.devart.title") => "排除开发/构建产物",
        (Lang::En, "rec.layercount.title") => "Reduce layer count",
        (Lang::Zh, "rec.layercount.title") => "减少镜像层数",
        (Lang::En, "rec.bigfiles.title") => "Large files added in recent layers",
        (Lang::Zh, "rec.bigfiles.title") => "近期层中新增的大文件",
        (Lang::En, "rec.junk.title") => "Editor/OS junk files",
        (Lang::Zh, "rec.junk.title") => "编辑器/系统垃圾文件",
        (Lang::En, "rec.slimbase.title") => "Consider a slimmer base image",
        (Lang::Zh, "rec.slimbase.title") => "考虑更精简的基础镜像",
        (Lang::En, "rec.oversized.title") => "Oversized layer(s)",
        (Lang::Zh, "rec.oversized.title") => "超大镜像层",
        (Lang::En, "rec.dflint.title") => "Dockerfile anti-patterns",
        (Lang::Zh, "rec.dflint.title") => "Dockerfile 反模式",
        (Lang::En, "rec.buildonly.title") => "Build-only files in runtime image",
        (Lang::Zh, "rec.buildonly.title") => "运行时镜像中的纯构建期文件",
        (Lang::En, "rec.doclocale.title") => "Documentation / man / locale data",
        (Lang::Zh, "rec.doclocale.title") => "文档 / man / locale 数据",
        (Lang::En, "rec.logtemp.title") => "Log / temp files baked into image",
        (Lang::Zh, "rec.logtemp.title") => "打进镜像的日志 / 临时文件",
        (Lang::En, "rec.toolchain.title") => "Build toolchain present in final image",
        (Lang::Zh, "rec.toolchain.title") => "最终镜像中存在构建工具链",
        (Lang::En, "rec.crossdup.title") => "Files duplicated across layers",
        (Lang::Zh, "rec.crossdup.title") => "跨层重复文件",
        (Lang::En, "rec.secfiles.title") => "Potential secrets in image",
        (Lang::Zh, "rec.secfiles.title") => "镜像中疑似存在密钥",
        (Lang::En, "rec.secmeta.title") => "Secrets in image metadata",
        (Lang::Zh, "rec.secmeta.title") => "镜像元数据中的密钥",
        (Lang::En, "rec.worldread.title") => "World-readable secret files",
        (Lang::Zh, "rec.worldread.title") => "所有人可读的密钥文件",
        (Lang::En, "rec.runasroot.title") => "Container runs as root",
        (Lang::Zh, "rec.runasroot.title") => "容器以 root 运行",
        (Lang::En, "rec.setuid.title") => "setuid/setgid binaries",
        (Lang::Zh, "rec.setuid.title") => "setuid/setgid 二进制文件",
        (Lang::En, "rec.worldwrite.title") => "World-writable files",
        (Lang::Zh, "rec.worldwrite.title") => "所有人可写的文件",
        (Lang::En, "rec.netreclaim.title") => "Net reclaimable estimate (de-duplicated)",
        (Lang::Zh, "rec.netreclaim.title") => "净可回收空间估算（已去重）",

        // ---- recommendation details (use {0}, {1}, … placeholders) --------
        (Lang::En, "rec.wasted.detail") => "{0} ({1}% of the image) is occupied by files that a later layer overwrites or deletes. Because each layer is immutable, those bytes still ship.",
        (Lang::Zh, "rec.wasted.detail") => "{0}（占镜像 {1}%）被后续层覆盖或删除的文件占用。由于每一层都是不可变的，这些字节仍会随镜像分发。",
        (Lang::En, "rec.pkgcache.detail") => "{0} of package-manager cache (OS: apt/apk/yum/dnf/pacman; language: pip/npm/yarn/cargo/go/composer/gem) is baked into the image across {1} file(s){2}.",
        (Lang::Zh, "rec.pkgcache.detail") => "{0} 的包管理器缓存（系统：apt/apk/yum/dnf/pacman；语言生态：pip/npm/yarn/cargo/go/composer/gem）被打进镜像，共 {1} 个文件{2}。",
        (Lang::En, "rec.devart.detail") => "{0} of build/SCM artifacts ({1} file(s){2}) such as .git, build caches or debug output is shipped.",
        (Lang::Zh, "rec.devart.detail") => "{0} 的构建/版本控制产物（{1} 个文件{2}），如 .git、构建缓存或调试输出被打进镜像。",
        (Lang::En, "rec.layercount.detail") => "The image has {0} layers (limit is 127). Many small layers add metadata overhead and slow pulls.",
        (Lang::Zh, "rec.layercount.detail") => "镜像有 {0} 层（上限 127）。过多的小层会增加元数据开销并拖慢拉取。",
        (Lang::En, "rec.bigfiles.detail") => "{0} across {1} file(s) was added in the most recent layers — review whether each is required at runtime.",
        (Lang::Zh, "rec.bigfiles.detail") => "最近的层中新增了 {0}，共 {1} 个文件——请逐一确认运行时是否真的需要。",
        (Lang::En, "rec.junk.detail") => "{0} of editor/OS junk ({1} file(s){2}) — .DS_Store, Thumbs.db, .vscode/.idea, vim swap files — is shipped.",
        (Lang::Zh, "rec.junk.detail") => "{0} 的编辑器/系统垃圾（{1} 个文件{2}）——.DS_Store、Thumbs.db、.vscode/.idea、vim swap——被打进镜像。",
        (Lang::En, "rec.slimbase.detail") => "Base OS is `{0}`, a full distribution. A slimmer base cuts both image size and CVE surface substantially.",
        (Lang::Zh, "rec.slimbase.detail") => "基础系统为 `{0}`，是完整发行版。改用更精简的基础镜像可大幅降低体积与 CVE 暴露面。",
        (Lang::En, "rec.slimbase.extra") => " Detected base ≈ {0} across {1} layer(s).",
        (Lang::Zh, "rec.slimbase.extra") => " 推算基础镜像约 {0}，共 {1} 层。",
        (Lang::En, "rec.oversized.detail") => "{0} layer(s) each exceed {1} (or 30% of the image). Slimming the dominant instruction has the biggest size impact.",
        (Lang::Zh, "rec.oversized.detail") => "{0} 个层各自超过 {1}（或镜像的 30%）。精简其主导指令对体积的影响最大。",
        (Lang::En, "rec.dflint.detail") => "{0} build-instruction issue(s){1} that inflate image size or break layer caching (reconstructed from history — verify against the real Dockerfile).",
        (Lang::Zh, "rec.dflint.detail") => "{0} 处构建指令问题{1}会增大镜像体积或破坏层缓存（基于历史反推，请对照真实 Dockerfile 核对）。",
        (Lang::En, "rec.buildonly.detail") => "{0} of likely build-time-only files ({1} file(s){2}) — static libs (.a/.o), headers (.h), .pyc, JS source maps. Verify they are not loaded at runtime before stripping.",
        (Lang::Zh, "rec.buildonly.detail") => "{0} 的疑似纯构建期文件（{1} 个文件{2}）——静态库(.a/.o)、头文件(.h)、.pyc、JS source map。移除前请确认运行时不会加载它们。",
        (Lang::En, "rec.doclocale.detail") => "{0} of docs, man pages, info and locale files ({1} file(s){2}) is rarely needed in a container.",
        (Lang::Zh, "rec.doclocale.detail") => "{0} 的文档、man、info 与 locale 文件（{1} 个文件{2}），容器中通常并不需要。",
        (Lang::En, "rec.logtemp.detail") => "{0} of log or temporary files ({1} file(s){2}) is shipped; these belong to runtime, not the image.",
        (Lang::Zh, "rec.logtemp.detail") => "{0} 的日志或临时文件（{1} 个文件{2}）被打进镜像；它们属于运行时，不应进镜像。",
        (Lang::En, "rec.toolchain.detail") => "{0} of compiler/build tools ({1} found{2}) are present. They enlarge the image and widen the attack surface if the app does not compile at runtime.",
        (Lang::Zh, "rec.toolchain.detail") => "存在 {0} 的编译器/构建工具（发现 {1} 个{2}）。若应用运行时不需编译，它们会增大体积并扩大攻击面。",
        (Lang::En, "rec.crossdup.detail") => "{0} of byte-identical content is duplicated across layers ({1} group(s)). Typical cause: a multi-stage build that copies node_modules, .so files, or model weights from the builder into the runtime stage instead of referencing them from a shared base.",
        (Lang::Zh, "rec.crossdup.detail") => "共 {0} 的相同内容在多个 layer 中重复存在（{1} 组）。常见原因：多阶段构建把 node_modules、.so 文件或模型权重从 builder 阶段又拷贝进 runtime 阶段，而不是从共享基础镜像引用。",
        (Lang::En, "rec.secfiles.detail") => "{0} suspected secret/credential file(s) detected. Deleting them in a later layer does NOT help — the earlier layer still contains the bytes and is recoverable.",
        (Lang::Zh, "rec.secfiles.detail") => "检测到 {0} 个疑似密钥/凭证文件。在后续层删除无济于事——较早的层仍含有这些字节且可被还原。",
        (Lang::En, "rec.secmeta.detail") => "{0} environment variable/label/Dockerfile {1} look like hardcoded credentials. Image metadata is world-readable via `docker inspect`.",
        (Lang::Zh, "rec.secmeta.detail") => "{0} {1}环境变量/标签/Dockerfile 项疑似硬编码凭证。镜像元数据可经 `docker inspect` 被任何人读取。",
        (Lang::En, "rec.worldread.detail") => "{0} suspected secret file(s) are readable by every user and process in the container (others-read bit set).",
        (Lang::Zh, "rec.worldread.detail") => "{0} 个疑似密钥文件对容器内所有用户和进程可读（others 读位已置位）。",
        (Lang::En, "rec.runasroot.detail") => "No non-root USER is set, so the process runs as root by default — a privilege-escalation risk if compromised.",
        (Lang::Zh, "rec.runasroot.detail") => "未设置非 root 的 USER，进程默认以 root 运行——一旦被攻破存在提权风险。",
        (Lang::En, "rec.setuid.detail") => "{0} setuid/setgid {1} present — common privilege-escalation targets.",
        (Lang::Zh, "rec.setuid.detail") => "存在 {0} 个 setuid/setgid 二进制文件——常见的提权目标。",
        (Lang::En, "rec.worldwrite.detail") => "{0} world-writable file(s) — any process/user in the container can tamper with them.",
        (Lang::Zh, "rec.worldwrite.detail") => "{0} 个所有人可写的文件——容器内任意进程/用户都能篡改它们。",
        (Lang::En, "rec.netreclaim.detail") => "Roughly {0} is reclaimable in total once overlap between the recommendations above is removed. This is the de-duplicated upper bound — do NOT add up the individual cards (the same file is often counted by several rules).",
        (Lang::Zh, "rec.netreclaim.detail") => "去除上述建议之间的重叠后，总计约 {0} 可回收。这是去重后的上界——请勿把各条建议简单相加（同一文件常被多条规则重复计数）。",

        // ---- recommendation fix hints -------------------------------------
        (Lang::En, "rec.wasted.hint") => "Create and clean up the data in the *same* RUN instruction so the bytes never enter a layer.",
        (Lang::Zh, "rec.wasted.hint") => "在*同一条* RUN 指令内创建并清理数据，使这些字节根本不进入任何层。",
        (Lang::En, "rec.pkgcache.hint") => "apt: `rm -rf /var/lib/apt/lists/*` in the same RUN; apk: `apk add --no-cache`; yum/dnf: `yum clean all`. For language tools: `pip install --no-cache-dir`, `npm ci && npm cache clean --force`, `go clean -cache -modcache`, `cargo --locked` + clean target; or use BuildKit `--mount=type=cache` for transient caches.",
        (Lang::Zh, "rec.pkgcache.hint") => "apt：同一 RUN 内 `rm -rf /var/lib/apt/lists/*`；apk：`apk add --no-cache`；yum/dnf：`yum clean all`。语言生态用 `pip install --no-cache-dir`、`npm ci && npm cache clean --force`、`go clean -cache -modcache`，或在 BuildKit 中用 `--mount=type=cache` 挂载临时缓存。",
        (Lang::En, "rec.devart.hint") => "Add a .dockerignore and/or use a multi-stage build so these never reach the final stage.",
        (Lang::Zh, "rec.devart.hint") => "添加 .dockerignore 并/或使用多阶段构建，使这些产物不进入最终阶段。",
        (Lang::En, "rec.layercount.hint") => "Merge consecutive RUN instructions with `&&`.",
        (Lang::Zh, "rec.layercount.hint") => "用 `&&` 合并相邻的 RUN 指令。",
        (Lang::En, "rec.junk.hint") => "Add these patterns to .dockerignore.",
        (Lang::Zh, "rec.junk.hint") => "把这些模式加入 .dockerignore。",
        (Lang::En, "rec.slimbase.hint") => "Switch to a `-slim`, `alpine`, or distroless base (verify glibc / runtime needs first).",
        (Lang::Zh, "rec.slimbase.hint") => "切换到 `-slim`、`alpine` 或 distroless 基础镜像（先确认 glibc / 运行时依赖）。",
        (Lang::En, "rec.oversized.hint") => "Audit the command that builds this layer; remove caches/intermediate files within the same RUN.",
        (Lang::Zh, "rec.oversized.hint") => "审查构建该层的命令；在同一 RUN 内移除缓存/中间文件。",
        (Lang::En, "rec.dflint.hint") => "Apply the fix noted inline on each item below.",
        (Lang::Zh, "rec.dflint.hint") => "按下方每一条内联标注的修复方式处理。",
        (Lang::En, "rec.buildonly.hint") => "Move compilation to a builder stage and copy only runtime outputs into the final stage.",
        (Lang::Zh, "rec.buildonly.hint") => "把编译放到 builder 阶段，最终阶段只拷贝运行时产物。",
        (Lang::En, "rec.doclocale.hint") => "Use distro minimization (e.g. dpkg `path-exclude`, `apk --no-cache`, or a -slim/distroless base).",
        (Lang::Zh, "rec.doclocale.hint") => "使用发行版精简手段（如 dpkg `path-exclude`、`apk --no-cache`，或 -slim/distroless 基础镜像）。",
        (Lang::En, "rec.logtemp.hint") => "Clean /var/log and /tmp at the end of the RUN that creates them.",
        (Lang::Zh, "rec.logtemp.hint") => "在生成它们的那条 RUN 末尾清理 /var/log 和 /tmp。",
        (Lang::En, "rec.toolchain.hint") => "Install build deps in a builder stage; keep only runtime packages in the final stage.",
        (Lang::Zh, "rec.toolchain.hint") => "在 builder 阶段安装构建依赖；最终阶段只保留运行时包。",
        (Lang::En, "rec.crossdup.hint") => "Use a multi-stage build with a single COPY of only the runtime-needed paths (`COPY --from=builder /app/dist /app/dist`); avoid `COPY --from=builder /app /app`. Share base layers between stages where possible.",
        (Lang::Zh, "rec.crossdup.hint") => "多阶段构建只 COPY 运行时真正需要的路径（`COPY --from=builder /app/dist /app/dist`），避免整目录 `COPY --from=builder /app /app`；尽可能让阶段间共享基础层。",
        (Lang::En, "rec.secfiles.hint") => "Never COPY secrets in; use BuildKit `--mount=type=secret` or runtime env/secret managers, then rebuild (squashing alone is not enough if the secret was committed upstream).",
        (Lang::Zh, "rec.secfiles.hint") => "切勿 COPY 密钥进镜像；用 BuildKit `--mount=type=secret` 或运行时环境/密钥管理器，并重建（若密钥已在上游层提交，仅 squash 不够）。",
        (Lang::En, "rec.secmeta.hint") => "Pass secrets at runtime (env/secret manager); do not bake them into ENV/ARG/LABEL.",
        (Lang::Zh, "rec.secmeta.hint") => "在运行时传入密钥（环境变量/密钥管理器）；不要写进 ENV/ARG/LABEL。",
        (Lang::En, "rec.worldread.hint") => "Restrict permissions (`chmod 600`) or, better, don't ship the secret at all.",
        (Lang::Zh, "rec.worldread.hint") => "收紧权限（`chmod 600`），更好的做法是根本不打进镜像。",
        (Lang::En, "rec.runasroot.hint") => "Add a dedicated user and `USER nonroot` before CMD.",
        (Lang::Zh, "rec.runasroot.hint") => "新建专用用户并在 CMD 前设置 `USER nonroot`。",
        (Lang::En, "rec.setuid.hint") => "Strip the bits you don't need: `RUN find / -perm /6000 -type f -exec chmod a-s {} +`.",
        (Lang::Zh, "rec.setuid.hint") => "去掉不需要的位：`RUN find / -perm /6000 -type f -exec chmod a-s {} +`。",
        (Lang::En, "rec.worldwrite.hint") => "Tighten permissions (e.g. `chmod o-w`).",
        (Lang::Zh, "rec.worldwrite.hint") => "收紧权限（如 `chmod o-w`）。",

        // ---- small fragments / words --------------------------------------
        (Lang::En, "frag.more") => " (+{0} more)",
        (Lang::Zh, "frag.more") => "（还有 {0} 条）",
        (Lang::En, "word.entry_sg") => "entry",
        (Lang::Zh, "word.entry_sg") => "",
        (Lang::En, "word.entry_pl") => "entries",
        (Lang::Zh, "word.entry_pl") => "",
        (Lang::En, "word.binary_sg") => "binary",
        (Lang::Zh, "word.binary_sg") => "个",
        (Lang::En, "word.binary_pl") => "binaries",
        (Lang::Zh, "word.binary_pl") => "个",

        // ---- dockerfile linter messages ({0} = instruction snippet) -------
        (Lang::En, "lint.add") => "ADD with URL/tarball — prefer COPY or explicit download+extract: {0}",
        (Lang::Zh, "lint.add") => "ADD 带 URL/压缩包——应改用 COPY 或显式下载+解压：{0}",
        (Lang::En, "lint.apt") => "apt install without `rm -rf /var/lib/apt/lists/*` in the same RUN: {0}",
        (Lang::Zh, "lint.apt") => "apt install 未在同一 RUN 内 `rm -rf /var/lib/apt/lists/*`：{0}",
        (Lang::En, "lint.upgrade") => "apt-get upgrade in a layer (non-reproducible, bloats image): {0}",
        (Lang::Zh, "lint.upgrade") => "层内执行 apt-get upgrade（不可复现且增大镜像）：{0}",
        (Lang::En, "lint.pip") => "pip install without `--no-cache-dir`: {0}",
        (Lang::Zh, "lint.pip") => "pip install 未加 `--no-cache-dir`：{0}",
        (Lang::En, "lint.npm") => "npm/yarn install without cache cleanup: {0}",
        (Lang::Zh, "lint.npm") => "npm/yarn install 未清理缓存：{0}",
        (Lang::En, "lint.chown") => "recursive chown/chmod in RUN duplicates the tree — use `COPY --chown`: {0}",
        (Lang::Zh, "lint.chown") => "RUN 内递归 chown/chmod 会复制整棵树——应用 `COPY --chown`：{0}",
        (Lang::En, "lint.runstreak") => "{0} consecutive RUN instructions — merge with `&&` to cut layers",
        (Lang::Zh, "lint.runstreak") => "连续 {0} 条 RUN 指令——用 `&&` 合并以减少层数",

        // ---- CLI output (main.rs) -----------------------------------------
        (Lang::En, "cli.analyzing") => "Analyzing {0}...",
        (Lang::Zh, "cli.analyzing") => "正在分析 {0}…",
        (Lang::En, "cli.result") => "Analyze result:",
        (Lang::Zh, "cli.result") => "分析结果：",
        (Lang::En, "cli.efficiency") => "  efficiency: {0} %",
        (Lang::Zh, "cli.efficiency") => "  效率：{0} %",
        (Lang::En, "cli.wasted") => "  wasted bytes: {0} bytes ({1})",
        (Lang::Zh, "cli.wasted") => "  浪费字节：{0} 字节（{1}）",
        (Lang::En, "cli.recs") => "Optimization recommendations:",
        (Lang::Zh, "cli.recs") => "优化建议：",
        (Lang::En, "cli.saved") => " (~{0} saved)",
        (Lang::Zh, "cli.saved") => "（约可省 {0}）",
        (Lang::En, "cli.fail") => "FAIL",
        (Lang::Zh, "cli.fail") => "失败",
        (Lang::En, "cli.check.eff") => "{0}: lowest efficiency check, lowest: {1}",
        (Lang::Zh, "cli.check.eff") => "{0}：效率下限检查，下限：{1}",
        (Lang::En, "cli.check.bytes") => "{0}: highest wasted bytes check, highest: {1}",
        (Lang::Zh, "cli.check.bytes") => "{0}：浪费字节上限检查，上限：{1}",
        (Lang::En, "cli.check.pct") => "{0}: highest user wasted percent check, highest: {1}",
        (Lang::Zh, "cli.check.pct") => "{0}：浪费比例上限检查，上限：{1}",
        (Lang::En, "cli.cifail") => "CI check fail",
        (Lang::Zh, "cli.cifail") => "CI 检查失败",

        // severity / category words (En identical to the machine codes)
        (Lang::En, "sev.high") => "high",
        (Lang::Zh, "sev.high") => "高",
        (Lang::En, "sev.medium") => "medium",
        (Lang::Zh, "sev.medium") => "中",
        (Lang::En, "sev.low") => "low",
        (Lang::Zh, "sev.low") => "低",
        (Lang::En, "sev.info") => "info",
        (Lang::Zh, "sev.info") => "提示",
        (Lang::En, "cat.size") => "size",
        (Lang::Zh, "cat.size") => "体积",
        (Lang::En, "cat.necessity") => "necessity",
        (Lang::Zh, "cat.necessity") => "必要性",
        (Lang::En, "cat.security") => "security",
        (Lang::Zh, "cat.security") => "安全",

        // ---- download/auth progress (docker.rs, stderr) -------------------
        (Lang::En, "prog.auth") => "  > Authenticating with {0}...",
        (Lang::Zh, "prog.auth") => "  > 正在向 {0} 认证…",
        (Lang::En, "prog.manifest") => "  > Fetching manifest...",
        (Lang::Zh, "prog.manifest") => "  > 正在获取 manifest…",
        (Lang::En, "prog.layers") => "  > {0} layer(s) | {1} compressed",
        (Lang::Zh, "prog.layers") => "  > {0} 层 | {1}（压缩后）",
        (Lang::En, "prog.download") => "  > Downloading {0} ({1}, {2})...",
        (Lang::Zh, "prog.download") => "  > 正在下载 {0}（{1}，{2}）…",
        (Lang::En, "prog.cached") => "  > Cached   {0} ({1}, {2})",
        (Lang::Zh, "prog.cached") => "  > 已缓存 {0}（{1}，{2}）",
        (Lang::En, "prog.cache.hit") => "  > Loaded analysis from cache ({0})",
        (Lang::Zh, "prog.cache.hit") => "  > 命中分析缓存（{0}）",

        // ---- Markdown report skeleton (markdown.rs) -----------------------
        (Lang::En, "md.title") => "Image Analysis",
        (Lang::Zh, "md.title") => "镜像分析",
        (Lang::En, "md.risktags") => "Risk Tags",
        (Lang::Zh, "md.risktags") => "风险标签",
        (Lang::En, "md.imginfo") => "Image Info",
        (Lang::Zh, "md.imginfo") => "镜像信息",
        (Lang::En, "md.col.field") => "Field",
        (Lang::Zh, "md.col.field") => "字段",
        (Lang::En, "md.col.value") => "Value",
        (Lang::Zh, "md.col.value") => "值",
        (Lang::En, "md.f.arch") => "Architecture",
        (Lang::Zh, "md.f.arch") => "架构",
        (Lang::En, "md.f.os") => "OS",
        (Lang::Zh, "md.f.os") => "系统",
        (Lang::En, "md.f.baseos") => "Base OS",
        (Lang::Zh, "md.f.baseos") => "基础系统",
        (Lang::En, "md.f.user") => "User",
        (Lang::Zh, "md.f.user") => "运行用户",
        (Lang::En, "md.f.csize") => "Compressed size",
        (Lang::Zh, "md.f.csize") => "压缩后大小",
        (Lang::En, "md.f.usize") => "Uncompressed size",
        (Lang::Zh, "md.f.usize") => "解压后大小",
        (Lang::En, "md.f.layers") => "Total Layers",
        (Lang::Zh, "md.f.layers") => "总层数",
        (Lang::En, "md.f.eff") => "Efficiency",
        (Lang::Zh, "md.f.eff") => "效率",
        (Lang::En, "md.f.wasted") => "Wasted space",
        (Lang::Zh, "md.f.wasted") => "浪费空间",
        (Lang::En, "md.f.baseimg") => "Base image",
        (Lang::Zh, "md.f.baseimg") => "基础镜像",
        (Lang::En, "md.baseimg.val") => "{0} layers, {1} compressed / {2} uncompressed",
        (Lang::Zh, "md.baseimg.val") => "{0} 层，{1}（压缩）/ {2}（解压）",
        (Lang::En, "md.dockerfile") => "Dockerfile (reconstructed)",
        (Lang::Zh, "md.dockerfile") => "Dockerfile（反推）",
        (Lang::En, "md.envs") => "Environment Variables",
        (Lang::Zh, "md.envs") => "环境变量",
        (Lang::En, "md.labels") => "Labels",
        (Lang::Zh, "md.labels") => "标签",
        (Lang::En, "md.wasted") => "Wasted Space",
        (Lang::Zh, "md.wasted") => "浪费的空间",
        (Lang::En, "md.wasted.desc") => "Files overwritten or deleted in a later layer (top 20 by size):",
        (Lang::Zh, "md.wasted.desc") => "被后续层覆盖或删除的文件（按大小取前 20）：",
        (Lang::En, "md.col.path") => "Path",
        (Lang::Zh, "md.col.path") => "路径",
        (Lang::En, "md.col.totwasted") => "Total Wasted",
        (Lang::Zh, "md.col.totwasted") => "浪费总量",
        (Lang::En, "md.col.occ") => "Occurrences",
        (Lang::Zh, "md.col.occ") => "次数",
        (Lang::En, "md.bigfiles") => "Large Files Added in Recent Layers",
        (Lang::Zh, "md.bigfiles") => "近期层中新增的大文件",
        (Lang::En, "md.col.size") => "Size",
        (Lang::Zh, "md.col.size") => "大小",
        (Lang::En, "md.col.mode") => "Mode",
        (Lang::Zh, "md.col.mode") => "权限",
        (Lang::En, "md.col.owner") => "Owner",
        (Lang::Zh, "md.col.owner") => "属主",
        (Lang::En, "md.secwarn") => "⚠️ Security Warnings (Potential Secrets)",
        (Lang::Zh, "md.secwarn") => "⚠️ 安全警告（疑似密钥）",
        (Lang::En, "md.col.layer") => "Layer",
        (Lang::Zh, "md.col.layer") => "层",
        (Lang::En, "md.col.risk") => "Risk",
        (Lang::Zh, "md.col.risk") => "风险",
        (Lang::En, "md.cell.layer") => "Layer {0}",
        (Lang::Zh, "md.cell.layer") => "第 {0} 层",
        (Lang::En, "md.secmore") => "*… and {0} more — see JSON output for the full list.*",
        (Lang::Zh, "md.secmore") => "*……还有 {0} 条——完整列表见 JSON 输出。*",
        (Lang::En, "md.recs") => "Optimization Recommendations",
        (Lang::Zh, "md.recs") => "优化建议",
        // Title-cased category words for the Markdown report (the CLI uses the
        // lowercase `cat.*` machine codes; Markdown historically title-cased).
        (Lang::En, "md.cat.size") => "Size",
        (Lang::Zh, "md.cat.size") => "体积",
        (Lang::En, "md.cat.necessity") => "Necessity",
        (Lang::Zh, "md.cat.necessity") => "必要性",
        (Lang::En, "md.cat.security") => "Security",
        (Lang::Zh, "md.cat.security") => "安全",
        (Lang::En, "md.heuristic") => " · heuristic",
        (Lang::Zh, "md.heuristic") => " · 启发式",
        (Lang::En, "md.potsavings") => "Potential savings",
        (Lang::Zh, "md.potsavings") => "预计可省",
        (Lang::En, "md.fix") => "Fix",
        (Lang::Zh, "md.fix") => "建议",
        (Lang::En, "md.affected") => "Affected",
        (Lang::Zh, "md.affected") => "受影响",
        (Lang::En, "md.layers") => "Layers",
        (Lang::Zh, "md.layers") => "分层",
        (Lang::En, "md.total") => "total",
        (Lang::Zh, "md.total") => "层",
        (Lang::En, "md.skipnote") => " — {0} base layers auto-detected and hidden",
        (Lang::Zh, "md.skipnote") => " —— 已自动识别并隐藏 {0} 个基础层",
        (Lang::En, "md.layerhead") => "Layer {0} — {1} compressed / {2} uncompressed",
        (Lang::Zh, "md.layerhead") => "第 {0} 层 — {1}（压缩）/ {2}（解压）",
        (Lang::En, "md.command") => "Command",
        (Lang::Zh, "md.command") => "命令",
        (Lang::En, "md.emptylayer") => "*Empty layer — no file changes.*",
        (Lang::Zh, "md.emptylayer") => "*空层——无文件变更。*",
        (Lang::En, "md.emptyline") => "- **Layer {0}** · empty layer",
        (Lang::Zh, "md.emptyline") => "- **第 {0} 层** · 空层",
        (Lang::En, "md.nochanges") => "*No file changes recorded for this layer.*",
        (Lang::Zh, "md.nochanges") => "*该层未记录文件变更。*",
        (Lang::En, "md.col.change") => "Change",
        (Lang::Zh, "md.col.change") => "变更",
        (Lang::En, "md.op.removed") => "Removed",
        (Lang::Zh, "md.op.removed") => "删除",
        (Lang::En, "md.op.modified") => "Modified",
        (Lang::Zh, "md.op.modified") => "修改",
        (Lang::En, "md.op.added") => "Added",
        (Lang::Zh, "md.op.added") => "新增",
        (Lang::En, "md.moremfiles") => "*{0} more files not shown*",
        (Lang::Zh, "md.moremfiles") => "*另有 {0} 个文件未显示*",

        // ---- Terminal UI (src/ui/*) ---------------------------------------
        (Lang::En, "tui.layers.title") => " Layers ",
        (Lang::Zh, "tui.layers.title") => " 分层 ",
        (Lang::En, "tui.layers.title_active") => " ● Layers ",
        (Lang::Zh, "tui.layers.title_active") => " ● 分层 ",
        (Lang::En, "tui.col.index") => "Index",
        (Lang::Zh, "tui.col.index") => "序号",
        (Lang::En, "tui.col.size") => "Size",
        (Lang::Zh, "tui.col.size") => "大小",
        (Lang::En, "tui.col.command") => "Command",
        (Lang::Zh, "tui.col.command") => "命令",
        (Lang::En, "tui.created") => "Created:",
        (Lang::Zh, "tui.created") => "创建于：",
        (Lang::En, "tui.command") => "Command:",
        (Lang::Zh, "tui.command") => "命令：",
        (Lang::En, "tui.layerdetails.title") => " Layer Details ",
        (Lang::Zh, "tui.layerdetails.title") => " 层详情 ",
        (Lang::En, "tui.files.title") => " Current Layer Contents ",
        (Lang::Zh, "tui.files.title") => " 当前层内容 ",
        (Lang::En, "tui.files.title_active") => " ● Current Layer Contents ",
        (Lang::Zh, "tui.files.title_active") => " ● 当前层内容 ",
        (Lang::En, "tui.col.perm") => "Permission",
        (Lang::Zh, "tui.col.perm") => "权限",
        (Lang::En, "tui.col.uidgid") => " UID:GID ",
        (Lang::Zh, "tui.col.uidgid") => " UID:GID ",
        (Lang::En, "tui.col.fsize") => "     Size",
        (Lang::Zh, "tui.col.fsize") => "   大小",
        (Lang::En, "tui.col.filetree") => "FileTree",
        (Lang::Zh, "tui.col.filetree") => "目录树",
        (Lang::En, "tui.modetips") => "Esc|0: All   1: Modified/Removed   2: File >= 1MB   |  Current: {0}",
        (Lang::Zh, "tui.modetips") => "Esc|0: 全部   1: 修改/删除   2: 文件 >= 1MB   |  当前: {0}",
        (Lang::En, "tui.col.count") => "Count",
        (Lang::Zh, "tui.col.count") => "次数",
        (Lang::En, "tui.col.totspace") => "Total Space",
        (Lang::Zh, "tui.col.totspace") => "总空间",
        (Lang::En, "tui.col.path") => "Path",
        (Lang::Zh, "tui.col.path") => "路径",
        (Lang::En, "tui.imgname") => "Image name: ",
        (Lang::Zh, "tui.imgname") => "镜像名称：",
        (Lang::En, "tui.totsize") => "Total Image size: ",
        (Lang::Zh, "tui.totsize") => "镜像总大小：",
        (Lang::En, "tui.potwasted") => "Potential wasted space: ",
        (Lang::Zh, "tui.potwasted") => "潜在浪费空间：",
        (Lang::En, "tui.effscore") => "Image efficiency score: ",
        (Lang::Zh, "tui.effscore") => "镜像效率分数：",
        (Lang::En, "tui.imgdetails.title") => " Image Details ",
        (Lang::Zh, "tui.imgdetails.title") => " 镜像详情 ",
        (Lang::En, "tui.recs") => "Optimization Recommendations",
        (Lang::Zh, "tui.recs") => "优化建议",
        (Lang::En, "tui.heur") => " [heuristic]",
        (Lang::Zh, "tui.heur") => " [启发式]",

        // ---- AI analysis (ai.rs, CLI stderr/stdout) -----------------------
        (Lang::En, "ai.analyzing") => "  > Asking AI for optimization insights...",
        (Lang::Zh, "ai.analyzing") => "  > 正在请求 AI 分析优化要点…",
        (Lang::En, "ai.compare") => "  > Comparing against previous snapshot...",
        (Lang::Zh, "ai.compare") => "  > 正在与上次分析快照对比…",
        (Lang::En, "ai.report") => "AI optimization report:",
        (Lang::Zh, "ai.report") => "AI 优化分析报告：",
        (Lang::En, "ai.fail") => "AI analysis failed: {0}",
        (Lang::Zh, "ai.fail") => "AI 分析失败：{0}",
        (Lang::En, "ai.script.title") => "Entrypoint / CMD scripts",
        (Lang::Zh, "ai.script.title") => "入口/启动脚本（ENTRYPOINT/CMD）",
        (Lang::En, "ai.script.truncated") => "… (script truncated)",
        (Lang::Zh, "ai.script.truncated") => "……（脚本已截断）",

        // ---- WeCom push (wecom.rs, CLI stderr/stdout) ---------------------
        (Lang::En, "wecom.title") => "diving analysis report",
        (Lang::Zh, "wecom.title") => "diving 分析报告",
        (Lang::En, "wecom.sending") => "  > Pushing result to WeCom bot...",
        (Lang::Zh, "wecom.sending") => "  > 正在推送结果到企微机器人…",
        (Lang::En, "wecom.sent") => "Result pushed to WeCom bot.",
        (Lang::Zh, "wecom.sent") => "结果已推送到企微机器人。",
        (Lang::En, "wecom.fail") => "WeCom push failed: {0}",
        (Lang::Zh, "wecom.fail") => "企微推送失败：{0}",

        // Unknown key: visible, safe marker.
        (_, _) => "<missing i18n key>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_resolution() {
        assert_eq!(Lang::resolve(Some("zh_CN.UTF-8")), Lang::Zh);
        assert_eq!(Lang::resolve(Some("en_US.UTF-8")), Lang::En);
        assert_eq!(Lang::resolve(Some("")), Lang::En);
        assert_eq!(Lang::resolve(Some("中文")), Lang::Zh);
    }

    #[test]
    fn fill_substitutes_in_order() {
        assert_eq!(
            fill("{0} of {1} files", &["1.2 MB", "30"]),
            "1.2 MB of 30 files"
        );
        assert_eq!(fill("no args", &[]), "no args");
    }

    #[test]
    fn english_catalog_is_unchanged() {
        // Guard: En must reproduce the original literals exactly.
        assert_eq!(
            tr(Lang::En, "rec.runasroot.title"),
            "Container runs as root"
        );
        assert_eq!(
            tr(Lang::En, "rec.netreclaim.title"),
            "Net reclaimable estimate (de-duplicated)"
        );
    }

    #[test]
    fn zh_catalog_has_entries() {
        assert_ne!(tr(Lang::Zh, "rec.runasroot.title"), "<missing i18n key>");
        assert_ne!(tr(Lang::Zh, "rec.wasted.detail"), "<missing i18n key>");
    }
}
