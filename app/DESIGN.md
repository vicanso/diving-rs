# Diving Desktop 设计系统与交互规范（NSH 0.7.0）

> 本文档是 `Diving Desktop`（位于仓库 `app/` 子目录）在进入界面与组件开发前确立的规范性设计文档（Design System、信息架构、Motion Inventory 与验收门禁）。所有 `app/src/ui/` 基础样式控件与 `app/src/features/` 业务屏幕必须严格遵守本文档与 Native Shell（NSH `0.7.0`）`nsh-design-guides` 规范。

---

## 1. 文档定位、目标平台与 NSH 0.6.1 工程前提

### 1.1 产品定位与架构前提

- **产品名称**：`Diving Desktop`（容器镜像层分析与诊断原生桌面客户端）
- **工程形态**：位于仓库独立子目录 `app/`（根 `Cargo.toml` 中通过 `[workspace] exclude = ["app"]` 隔离，通过 `path` 依赖 `../crates/diving-core` 与 `../crates/diving-app`）
- **开发模式**：纯 Rust 模式（`nsh init app --template rust`），无 JS VM、无内嵌浏览器，零独立服务端，直接在进程内复用 `diving-core` 分析引擎与 `~/.diving/{layers,analysis}` 持久化缓存及凭证，支持远程仓库、本地 `docker://`、`podman://` 与本地压缩包/目录 `file://`
- **设计宣言（Design Thesis）**：v2 采用 **OrbStack / Linear 风格的原生 macOS 专业工具台**——以纯净苹果浅灰白（Light）与中性炭黑 + 克制钴蓝（Dark）为基底，全高可折叠左侧边栏与 48pt 一体化内容顶栏统一承接导航、最近记录、工作区 Tab 与 Diff 操作；**去掉网页营销感、去掉重边界线"框套框"堆积、用微弱面层色差（6.5%–7% 发丝线）建立层次，并将键盘与辅助功能作为一等公民**。

### 1.2 目标运行平台与窗口约束

| 维度 | 规范值 | 说明 |
| --- | --- | --- |
| 目标平台族 | Desktop（`macOS 14+` 主力、Linux `GTK 4.14+`、Windows `10 1809+`） | 仅针对桌面键鼠工作流与紧凑密度设计，不包含移动端或 TUI 适配 |
| 窗口默认尺寸 | `1280 × 820 pt` | 兼顾三栏主检视工作台与 Findings 双列展示 |
| 窗口最小尺寸 | `960 × 640 pt` | 在最小尺寸下仍须保持主任务完整可用，禁止横向溢出与文字截断死锁 |
| 响应式断点 | `1100 pt` | `>= 1100 pt` 为标准工作台模式；`960–1099 pt` 进入紧凑工作台模式 |

### 1.3 NSH 0.6.1 强制约束清单

在 `0.6.1` 版本的工具链与运行机制下，设计阶段必须纳入以下硬性工程约束：

1. **桌面自绘融合标题栏（#2348）**：隐藏操作系统默认标题栏（`TitleBarStyle::Hidden`），应用首行直接作为融合标题栏；通过 `title_bar()` 动态读取标题栏高度与原生窗口按钮占位（macOS 左侧红绿灯、Windows/Linux 右侧窗口按钮），禁止写死左侧 padding。
2. **Capabilities 显式授权（0.6.0 #1499）**：`app/nsh.json` 若声明 `capabilities`，必须显式包含 `application: true`（主窗口、应用菜单、全局快捷键、退出与重启控制）与 `homeDir: true`（读取 `~/.diving/` 配置与缓存目录），并按需声明 `picker: true`（本地文件选择 `pick_paths` / `pick_save_path`）、`clipboard: true` 与 `openUrl`。
3. **SVG 静态资源特性门控（0.6.1 #2615）**：通过 `asset!` / `assets!` 打包静态 SVG 图标时，必须在各桌面平台依赖（`nsh-platform-macos`、`nsh-platform-linux`、`nsh-platform-windows`）显式启用 `svg` feature，避免构建期报错。
4. **右键上下文菜单事件类型（0.6.0 #2202）**：Rust 侧 `ContextMenu` 监听签名使用 `ContextMenuEvents`（而非旧版 `MenuEvent`）。
5. **原生 Spring 过渡动画（0.6.0 #2259）**：`Transition` 为 `#[non_exhaustive]` 且原生支持 `spring` 曲线，用于侧抽屉、面板展开与物理复位。
6. **Updater 命名通道与退出安装（0.6.0/0.6.1 #2249 / #2263）**：`app/nsh.json` 的 `updater` 必须声明专属桌面发布通道（例如 `desktop`）并启用 `installOnQuit`，防止仅发布 `diving` CLI 版本时误触发桌面客户端更新提示。
7. **指针光标礼仪（0.6.0 #2224）**：按钮、标签页、下拉选择器、列表行等应用内交互控件一律使用系统默认箭头光标；**仅外链（`link()`，`role="link"`）使用 `cursor_pointer()`**；拖拽分割条按实际方向使用 resize 光标。

---

## 2. 语义主题 Token 体系（`app/theme.json` 规范）

### 2.1 Token 使用铁律

- **单一真相源**：所有色值仅存在于 `app/theme.json`（shadcn `cssVars` 格式）。
- **零字面量色值**：业务界面代码（`features/*`）与样式控件代码（`ui/*`）中**严禁出现**裸 hex、RGB/RGBA、HSL/HSLA 或调色板类名（如 `rgb(..)`）。
- **禁止 `.unwrap()` 读取自定义 Token**：`app/src/ui/theme.rs` 对 `theme().color("...")` 与 `theme().number("...")` 封装访问函数，并使用在视觉审查时一眼可见错位的 fallback（如回退至反差明显的 `theme().destructive` 或 `theme().foreground`），避免运行期 panic 或隐形失效。
- **状态色不得用于装饰**：`success`、`warning`、`danger`、`info` 仅用于表达真实系统/分析状态，且必须配合图标或文字符号，**绝不单独依赖颜色传递语义**。

### 2.2 Light / Dark 双套语义色值对照表

#### A. 核心表面、文字与边框

| Token 名称 | 用途角色 | Light 值 | Dark 值 | 备注 |
| --- | --- | --- | --- | --- |
| `background` | 主窗口右区画布底色 | `#ffffff` | `#111214` | 中性炭黑（Dark）/ 纯白（Light） |
| `foreground` | 主正文与主标题文字 | `#111318` | `#eceef2` | 与主表面保持 AAA/AA 高对比 |
| `card` | 卡片/信息面板/代码框表面 | `#ffffff` | `#191b1f` | 微弱提亮台阶，极细边界分隔 |
| `card-foreground` | 卡片内主要文字 | `#111318` | `#eceef2` | — |
| `sidebar` | 顶通可折叠左侧 App Sidebar 底色 | `#f5f5f7` | `#16171a` | 自绘标题栏下顶通延伸至窗口顶边 |
| `sidebar-foreground` | 侧边栏正文文字 | `#1c1f26` | `#d4d7de` | — |
| `popover` | `Menu` / `Select` / `Popover` / `Tooltip` 浮层表面 | `#ffffff` | `#1e2025` | 所有锚定浮层共享同一处理 |
| `popover-foreground` | 浮层内主文字 | `#111318` | `#eceef2` | — |
| `muted` | 次级静音容器、滑块轨底、代码块内嵌区 | `#f2f3f6` | `#202227` | 无重外框的色调层级 |
| `muted-foreground` | 次级辅助信息、字段标签、占位说明 | `#636874` | `#8b909a` | 次要信息始终用颜色层级而非单纯缩小字号 |
| `text-tertiary` | 三级元数据（快捷键标牌、版本次标） | `#9298a4` | `#5e636e` | 自定义 Token |
| `border` | 发丝线边界、表格横线、分割线 | `rgba(0, 0, 0, 0.07)` | `rgba(255, 255, 255, 0.065)` | 极低噪仅单侧绘制一次 |
| `border-strong` | 强调边框、弹窗描边、悬浮加强边 | `rgba(0, 0, 0, 0.15)` | `rgba(255, 255, 255, 0.14)` | 自定义 Token |
| `input` | 输入框与统一命令栏静态边界 | `rgba(0, 0, 0, 0.11)` | `rgba(255, 255, 255, 0.10)` | — |
| `ring` | 键盘焦点外环（`focus_visible`） | `rgba(37, 99, 235, 0.45)` | `rgba(59, 130, 246, 0.55)` | 必须绘制在控件边框之外 |

#### B. 交互动作与状态变体（Per-variant Normal / Hover / Active）

| Token 名称 | 用途角色 | Light 值 | Dark 值 | 备注 |
| --- | --- | --- | --- | --- |
| `primary` | 决策区唯一默认主动作背景（克制钴蓝） | `#2563eb` | `#3b82f6` | 每块决策区最多 1 个 |
| `primary-hover` | 主动作悬停态 | `#1d4ed8` | `#4f8ff7` | 自定义 Token |
| `primary-active` | 主动作按压态 | `#1e40af` | `#2563eb` | 自定义 Token |
| `primary-foreground` | 主动作前景色 | `#ffffff` | `#ffffff` | 纯白文字确保对比可读 |
| `secondary` | 次要默认按钮静态背景 | `#f0f1f4` | `#23252b` | 无外框纯色调按钮 |
| `secondary-hover` | 次要动作悬停态 | `#e5e7ec` | `#2b2e36` | 自定义 Token |
| `secondary-active` | 次要动作按压态 | `#d9dce3` | `#333741` | 自定义 Token |
| `secondary-foreground` | 次要动作前景色 | `#111318` | `#eceef2` | — |
| `accent` | 列表行、菜单项、幽灵按钮悬停洗色 | `#ecedf1` | `#262930` | — |
| `accent-foreground` | 悬停态文字前景色 | `#111318` | `#eceef2` | — |
| `surface-active` | 侧栏、层列表与文件树当前选中行填充背景 | `rgba(37, 99, 235, 0.10)` | `rgba(59, 130, 246, 0.15)` | 自定义 Token |
| `destructive` | 危险/破坏性确认操作背景 | `#dc2626` | `#ef4444` | — |
| `destructive-hover` | 危险动作悬停态 | `#b91c1c` | `#dc2626` | 自定义 Token |
| `destructive-active` | 危险动作按压态 | `#991b1b` | `#b91c1c` | 自定义 Token |
| `destructive-foreground` | 危险动作前景色 | `#ffffff` | `#ffffff` | — |

#### C. 语义状态与 Diving 领域专属 Token

| Token 组 | Token 名称 | Light 值 | Dark 值 | 强制配对约束 |
| --- | --- | --- | --- | --- |
| **系统状态** | `success` / `success-soft` | `#059669` / `rgba(5, 150, 105, 0.10)` | `#34d399` / `rgba(52, 211, 153, 0.14)` | 高效率评分、已解决项 |
|  | `warning` / `warning-soft` | `#d97706` / `rgba(217, 119, 6, 0.10)` | `#fbbf24` / `rgba(251, 191, 36, 0.14)` | 警告、陈旧缓存（Stale）、浪费提醒 |
|  | `danger` / `danger-soft` | `#dc2626` / `rgba(220, 38, 38, 0.10)` | `#f87171` / `rgba(248, 113, 113, 0.14)` | 失败、严重诊断、glibc 不兼容 |
|  | `info` / `info-soft` | `#6366f1` / `rgba(99, 102, 241, 0.10)` | `#818cf8` / `rgba(129, 140, 248, 0.14)` | 信息提示与特征标签 |
| **文件变更** | `op-added` / `op-added-bg` | `#059669` / `rgba(5, 150, 105, 0.10)` | `#34d399` / `rgba(52, 211, 153, 0.14)` | **必须**与前缀符号 `+` 成对渲染 |
|  | `op-modified` / `op-modified-bg` | `#d97706` / `rgba(217, 119, 6, 0.10)` | `#fbbf24` / `rgba(251, 191, 36, 0.14)` | **必须**与前缀符号 `~` 成对渲染 |
|  | `op-removed` / `op-removed-bg` | `#dc2626` / `rgba(220, 38, 38, 0.10)` | `#f87171` / `rgba(248, 113, 113, 0.14)` | **必须**与前缀符号 `−` 成对渲染 |
| **Dockerfile 指令** | `opcode-from` / `opcode-from-bg` | `#4f46e5` / `rgba(99, 102, 241, 0.10)` | `#a5b4fc` / `rgba(129, 140, 248, 0.14)` | `FROM` 指令胶囊 |
|  | `opcode-run` / `opcode-run-bg` | `#0284c7` / `rgba(14, 165, 233, 0.10)` | `#38bdf8` / `rgba(56, 189, 248, 0.14)` | `RUN` 指令胶囊 |
|  | `opcode-copy` / `opcode-copy-bg` | `#059669` / `rgba(5, 150, 105, 0.10)` | `#34d399` / `rgba(52, 211, 153, 0.14)` | `COPY` / `ADD` 指令胶囊 |
|  | `opcode-cmd` / `opcode-cmd-bg` | `#d97706` / `rgba(217, 119, 6, 0.10)` | `#fbbf24` / `rgba(251, 191, 36, 0.14)` | `CMD` / `ENTRYPOINT` 指令胶囊 |
|  | `opcode-meta` / `opcode-meta-bg` | `#475569` / `rgba(100, 116, 139, 0.10)` | `#94a3b8` / `rgba(148, 163, 184, 0.14)` | `ENV` / `LABEL` / `WORKDIR` 等元指令 |
| **体积微条** | `size-bar-track` | `rgba(15, 23, 42, 0.06)` | `rgba(255, 255, 255, 0.08)` | 尺寸比例底轨 |
|  | `size-bar-fill` | `#0ea5e9` | `#38bdf8` | 常规占比填充 |
|  | `size-bar-heavy` | `#f59e0b` | `#fbbf24` | 中高占比预警 |
|  | `size-bar-extreme` | `#ef4444` | `#f87171` | 极大体积占比 |
| **Treemap 色板** | `treemap-1` / `treemap-2` / `treemap-3` | `#2a78d6` / `#eb6834` / `#1baf7a` | `#3987e5` / `#d95926` / `#199e70` | 对应占比前三的镜像层 |
|  | `treemap-other` | `#c3c2b7` | `#4a4a46` | 其余层聚合 |
|  | `treemap-ink` / `treemap-ink-other` | `#0b0b0b` / `#0b0b0b` | `#0b0b0b` / `#ffffff` | 经过对比度校验的区块内文字色 |
| **Diff 发散色对** | `diverge-up` | `#eb6834` | `#d95926` | 体积/数量增长，**必须**配 `+` |
|  | `diverge-down` | `#2a78d6` | `#3987e5` | 体积/数量缩减，**必须**配 `−` |

---

## 3. 字体、图标、圆角、间距与桌面密度标尺

### 3.1 字体与离线打包策略

- **原生 UI 字体 + 离线等宽字体**：
  - UI 正文字体：不在 `app/theme.json` 设置 `font-sans`，直接复用平台原生 UI 字体栈（macOS 为 **SF Pro + 苹方 PingFang SC**，消除中英混排割裂感；Windows/Linux 分别回退系统无衬线字体）。
  - 等宽字体：`IBM Plex Mono`（权重 `400`, `500`, `600`，离线打包于 `app/assets/fonts/`，零外部 CDN 请求），专用于镜像引用、文件路径、层摘要、SHA256、字节大小数字、Docker 指令与 Dockerfile、Unix 权限/UID:GID、快捷键标牌以及文件代码/Hex 预览。
- **等宽数字对齐**：所有可比数值（效率评分、体积、占比、文件计数、Diff 增减值）必须调用 `.tabular_nums()` 并右对齐（`text_right`），避免切换或过滤时数字列抖动。

### 3.2 字阶规范（Type Scale）

单屏最多使用 **3–4 个字号** 与 **2 个字重（Normal 400 / Semibold 600，仅局部强调使用 Medium 500）**，正文字号下限为 **`12pt`**：

| 级别 | NSH 类/尺寸 | 用途场景 | 字重 | 行高策略 |
| --- | --- | --- | --- | --- |
| 展示标题 | `text-2xl` (24pt) | 仅用于 Landing 页主标题 | `BOLD` (700) | 紧凑标题行高，配置 `max_font_size_multiplier` 上限防止过大字号下撑破首屏 |
| 区块标题 | `text-base` (16pt) | 弹窗标题、抽屉标题、Finding 面板标题 | `SEMIBOLD` (600) | 单行截断或允许自然折行，`role="header"` |
| 标准正文 | `text-sm` (14pt) | 按钮、输入框、Tab 标签、KPI 主值、命令摘要正文 | `NORMAL` / `MEDIUM` | 控件基于内容与 `min_h` 撑开，适应系统字体缩放 |
| 密集数据 | `text-xs` (12pt) | 虚拟化文件树行、层时间线行、表格单元格、辅助标签、状态栏提示 | `NORMAL` / `MEDIUM` (Mono) | 与固定高密度行轨道协调，长文本配置可控截断与 Tooltip/完整预览 |

### 3.3 图标规范

- **统一图元家族**：全部界面图标使用同一套线性 SVG 家族（统一 `1.5px` 描边、圆角端帽），放入 `app/assets/icons/`，通过 `app/src/ui/icons.rs` 导出枚举与固定尺寸渲染。
- **图标对齐槽位**：列表、按钮与菜单中的图标必须居中放入固定宽度正方形槽位（`14pt` 对应 `Sm`/`Xs`，`16pt` 对应 `Md`/`Lg`），防止不同图标内在宽高差引起相邻文字错位。
- **无障碍标注**：纯图标按钮必须具备简洁准确的英文 `accessibility_label`（如 `"Switch color theme"`, `"Open local image file"`）并在桌面端配置 `Tooltip` 说明含义与快捷键。

### 3.4 圆角与同心几何（Radius Scale）

仅允许以下 4 个圆角层级，严禁一处一值：

| 层级 | Token / 方法 | 数值 | 适用对象 |
| --- | --- | --- | --- |
| 容器圆角 | `radius-lg` | `12pt` | Findings 面板卡片、Landing 一体化命令框、Dialog/Sheet 容器、Treemap 外框 |
| 控件圆角 | `radius` | `7pt` | 按钮、输入框、`Select`、`SegmentedControl` 容器、Popup/Menu/Popover 浮层 |
| 内嵌圆角 | `radius-inset` | `5pt` | 分段控制内部滑块、Tab 选中底块、Sidebar 导航项、Opcode 胶囊、状态徽章（遵守 `inner ≈ max(0, outer − gap)`） |
| 全圆角 | `rounded_full` | `9999pt` | **仅限**圆形指示点、Switch 拇指；**严禁**用于普通按钮或 Tooltip |

**裁切约束**：带圆角的列表容器、选中高亮填充与内部滚动区域必须设置 `.overflow_hidden()` 或同步圆角，确保背景不溢出圆角；同时外置 `focus_visible` 光环的父级必须保留至少 `3pt` 外延留白，避免光环被裁切。

### 3.5 空间语法（Spacing Scale）与对齐原则

仅从语义步进序列中取值：**`2 / 4 / 8 / 12 / 16 / 24 / 32 pt`**（对应 `.gap_0_5()` 到 `.gap_8()`）：

- `gap-0.5` (2pt)：像素级微调、紧密符号与数字间隔
- `gap-1` (4pt)：微标签内边距、控件标签与图标紧邻间隙
- `gap-2` (8pt)：按钮内图标与文字、表单/工具栏同行相邻控件、弹窗底部按钮组
- `gap-3` (12pt)：面板内相关子组、KPI 指标块内元素间距、紧致行内边距
- `gap-4` (16pt)：面板内边距（`p_4`）、分栏内部区块间隔、表单控件组之间
- `gap-6` (24pt)：Findings 网格间距、主要页面大模块之间
- `gap-8` (32pt)：空状态呼吸留白、Landing 上下留白

**结构性铁律**：
1. **内边距归组件，外间距归父级**：`Panel` / `Button` / `Input` 自己负责内部 padding，父容器只管 `gap_*`；面板内部不再额外套一层全距 Card，禁止双重内边距。
2. **逻辑起止边**：水平间距与对齐优先采用 start/end 语义方法（`ps_*` / `pe_*`），保证文本方向镜像一致性。
3. **发丝线单一归属**：`1pt`（1px）边界线仅允许作为经过审计的字面量点值例外，由边界拥有者单侧绘制一次，相邻面板禁止重复描边导致 `2px` 粗线。

### 3.6 桌面密度与统一 `Size` 枚举

`app/src/ui/` 中所有基础交互控件统一接受一个 `Size` 枚举，一次锁定高度、水平内边距、字号、图标尺寸、内间距与圆角，禁止在页面端局部改高：

| `Size` 变体 | 控件高度 | 图标尺寸 | 文本字号 | 水平内边距 | 典型适用位置 |
| --- | --- | --- | --- | --- | --- |
| `Size::Xs` | `22pt` | `12pt` | `12pt` | `7pt` | 侧栏底栏微调切换、命令框内联微按钮 |
| `Size::Sm` | `26pt` | `14pt` | `12pt` | `10pt` | 48pt 工具栏动作、过滤下拉、侧栏过滤分段按钮 |
| `Size::Md`（默认） | `30pt` | `15pt` | `13pt` | `12pt` | 常规按钮、弹窗表单控件、Landing 命令栏提交按钮 |
| `Size::Lg` | `36pt` | `15pt` | `13pt` | `14pt` | Landing 一体化命令框输入、局部大控件 |

- **图标按钮**：宽高严格等于当前 `Size` 的高度正方形（`22 / 26 / 30 / 36 pt`）。
- **数据行高**：Sidebar 导航行 `28pt`；`LayerTimeline` 行高 `30pt`；VirtualFileTree 数据行高 `28pt`；Diff 表格行高 `32pt`。

---

## 4. 桌面应用壳层与自绘标题栏（NSH 0.6.1 v2 工作台）

### 4.1 全高可折叠侧边栏 + 一体化 48pt 内容顶栏

依据 NSH `0.6.1` 设计规范（#2348）与 macOS 原生多栏工具范式（对标 OrbStack / Linear），桌面窗口启用 `TitleBarStyle::Hidden`，由全高可折叠 `App Sidebar`（`208pt`）与右侧 `Content Header Bar`（`48pt`）共同构成原生顶通窗口：

```text
+-----------------------+---------------------------------------------------------------------+
| [● ● ●]    [侧栏] [+] | [切换或标题]    [ 文件树 | 空间分布 | 分析发现 | Dockerfile ]   [操作] |
|-----------------------|---------------------------------------------------------------------|
| Workbench             | Landing：无顶栏底线，居中单框一体化命令栏 + 三列快捷卡片 + 示例          |
|  • New Analysis   ⌘N  |---------------------------------------------------------------------|
|  • Local Images  ⇧⌘O  | Inspector：36pt 单行指标带 → 可拖拽 [镜像层 | 详情工作区] + 底部状态条      |
|  • Compare        ⌘D  |---------------------------------------------------------------------|
|  • Open File…     ⌘O  | Diff：右侧直接渲染对比报告（所有操作收口于 48pt 内容顶栏）                |
|                       |                                                                     |
| Recent (Scroll)       |                                                                     |
|  • redis:alpine  99%  |                                                                     |
|-----------------------|                                                                     |
| Diving v0.12.3    ⋯   |                                                                     |
| [⌘K] [☀]     [EN|中]  |                                                                     |
+-----------------------+---------------------------------------------------------------------+
```

- **动态安全占位与折叠联动**：`App Sidebar` 展开时，红绿灯安全区归左侧栏顶部 `48pt` 拖拽区（右边紧跟侧栏折叠 `[PanelLeft]` 与新建 `[+]` 按钮）；`App Sidebar` 收起时，红绿灯安全区由 `Content Header Bar` 左侧接管，紧接着展示侧栏展开 `[PanelLeft]` 与紧凑实用按钮（保证在收起侧栏与窄窗口下仍可随时切换主题、语言与命令面板）。
- **拖拽与点击契约**：侧栏顶栏与内容顶栏背景均标记 `.window_drag(true)`，保留标准桌面窗体拖动与双击缩放；内部一切按钮、输入框、Tab 与胶囊控件显式标记 `.window_drag(false)`。
- **路由态职责映射**：
  - **Landing 态**：内容顶栏无底部分隔线，主画布光学偏上居中呈现「图标+单标题 → 无内边框嵌套的一体化命令框 → 3 列快捷动作卡片 → 无边框示例条」，近期分析由左侧边栏统一承载。
  - **Inspector 态**：内容顶栏左侧展示当前镜像身份胶囊（点击或 `⌘L` 原地切换为内联搜索栏），中间居中放置 `File Tree` / `Treemap` / `Findings` / `Dockerfile` 工作台 Tabs，右侧置对比/刷新/导出，下方紧接 `36pt` 单行指标摘要条，最大限度释放纵向阅读空间。
  - **Diff 态**：内容顶栏直接容纳 `Swap`、`Open Base`、`Open Target`、`Compare with…`、`Export Markdown…`，消除主区重复标题卡片。

### 4.2 响应式窗口策略（`960px` – 无上限）

1. **`>= 1080 pt`（标准桌面布局）**：
   - 默认展开 `208pt` 全高可折叠 `App Sidebar`（支持 `Mod+\` 或顶部按钮随时折叠并持久化）。
   - Inspector 内部采用 `ResizablePanelGroup` 横向分栏：左侧 `LayerTimeline`（最小 `260pt`、默认 `340pt`、最大 `480pt`，宽度持久化）+ 右侧 Detail 工作台（`flex_1 min_w_0`）。
   - `Findings` 页采用 2 列等宽网格布局。
2. **`960 – 1079 pt`（紧凑窗口适配）**：
   - `App Sidebar` 初始化自动收起（用户仍可按 `Mod+\` 或点左上按钮滑出），内容顶栏右侧 Compare 按钮自动收束为图标，确保 `48pt` 顶栏与 Tabs 不溢出。
   - `LayerTimeline` 宽度自动夹紧至 `260–320pt`，`File Tree` 中 `Mode & UID` 元数据列默认折起，`Findings` 退化为单列堆叠。

### 4.3 原生应用菜单（`set_application_menu`）

注册与桌面操作系统习惯一致的应用顶栏菜单，菜单名称、图标、可用状态与页面工具栏/快捷键完全同步：

- **Diving（macOS 应用菜单）/ Help**：关于 Diving Desktop、检查更新…、隐藏/退出
- **File（文件）**：聚焦搜索 (`Mod+L`)、打开本地镜像文件 (`file://`)… (`Mod+O`)、浏览本地 Docker/Podman 镜像… (`Mod+Shift+O`)、导出 Markdown 报告… (`Mod+E`)、与另一镜像对比… (`Mod+D`)
- **View（视图）**：工作台 Tab 切换（`Mod+1` 文件树、`Mod+2` 空间分布、`Mod+3` 分析发现、`Mod+4` Dockerfile）、切换深/浅主题、切换中/英语言、打开命令面板 (`Mod+K`)、刷新当前分析 (`Mod+R`)
- **Edit（编辑）**：标准剪切/复制/粘贴/全选（保障 `Input` 与只读 `Editor` 复制顺畅）
- **Window（窗口）**：最小化、缩放、关闭

---

## 5. 自有 `app/src/ui/` 样式组件库与 UI Gallery

### 5.1 强分层架构契约

严格履行 NSH 三层架构边界：

- `nsh_core`：布局（Taffy）、元素原语（`view`, `text`, `scroll`, `virtual_list`, `canvas`, `svg`）、响应式信号与异步调度。
- `nsh_base`：无外观的行为原语（焦点、键盘导航、角色与状态暴露、拖拽与虚拟化容器）。
- **`app/src/ui/`（本层）**：把 `nsh_base` 每一个组件按本文档第 2、3、7 节规范包装一次并冻结视觉规则。
- **`app/src/features/`（业务页）**：**只能**引用 `crate::ui::*` 与 `nsh_core` 布局原语；严禁直接 `use nsh_base::*`，严禁出现字面量颜色、圆角、字号，严禁用 `view().on_press(..)` 代替 `ui::Button` 或列表行组件。

### 5.2 `app/src/ui/` 模块清单与职责

| 模块文件 | 核心导出与职责边界 |
| --- | --- |
| `theme.rs` | 封装 `theme()` 访问器，提供全部 Diving 自定义语义色与尺寸的强类型访问函数（内置显著故障回退值，严禁 `.unwrap()`） |
| `motion.rs` | 集中定义 Motion Tokens（时长、缓动、距离、缩放、Spring 参数）及安全创建/降级工具（统一读取 `reduce_motion()`） |
| `icons.rs` | 绑定 `assets!` 下的线性 SVG 图标枚举与统一尺寸槽位渲染组件 `Icon` |
| `button.rs` | `Button`（`Variant::{Primary, Default, Outline, Ghost, Danger}` + `Size` + `loading` / `disabled` / `selected`）与 `IconButton`（自动强制 `accessibility_label` + `Tooltip`） |
| `input.rs` | `Input`（单行文本与搜索输入，与下拉框共享 `px-3` 水平起边）、`SearchBar`（含前置图标、右侧清除按钮、快捷键徽章 `/`） |
| `controls.rs` | `Checkbox`（支持 `indeterminate`）、`Switch`、`Select`（共享统一 `popup` 样式）、`SegmentedControl`（整组同一轮廓、无多余重边界线、内部单一滑块背景） |
| `tabs.rs` | `Tabs` 与 `TabItem`（采用完整表面填充背景或内置滑动指示块表达选中态，**严禁**使用单边竖条/下划线网页模板风标记导航项） |
| `overlay.rs` | 统一样式 helper `popup_surface()`（所有浮层共用），封装 `Menu`、`ContextMenu`（0.6.0 `ContextMenuEvents`）、`Popover`、`Tooltip`、`Dialog`、`AlertDialog`、`Sheet`、`Toast` |
| `data.rs` | `Panel`（无阴影 hairline 区块）、`Badge`（默认中性语义，克制用色）、`OpcodePill`、`StatusBadge`（强制图标/符号+颜色）、`SizeMicroBar`、`Separator`、`EmptyState`、`Skeleton`、`ProgressBar`、`Scrollbar`（置于真实滚动容器最右边缘）、`ResizableSplit` |
| `code.rs` | `CodeViewer`（基于 `nsh_base::editor()` 的只读配置与主题对齐，用于 Dockerfile 与文本文件预览）与 `PatchDiffViewer`（基于 `nsh_base::diff_view()`，需要文本对比时复用） |
| `gallery.rs` | **开发态 UI Gallery 页面**：在一个独立页面内集中展示上述全部组件的所有 `Variant`、`Size`、静态/悬停/按压/焦点/选中/禁用/加载状态，支持一键切换 Light / Dark 与字体缩放预览，作为编写业务屏前的前置门禁 |

---

## 6. 核心页面信息架构、对齐脊线与状态设计

> **对齐脊线（Alignment Spines）原则**：同一区域内所有兄弟块共享同一垂直或水平对齐线；列表行中的图标、标签、数字、动作按固定列轨道对齐，缺失某列可选徽章时，其余列绝不发生偏移；混合字号同行按文本基线（baseline）对齐，图标与文字按中线对齐。

### 视图 1：Landing（首页与来源入口）与渐进分析预览态

- **主要任务**：输入任意远程镜像、选择本地 Docker/Podman 镜像或挑选本地 `tar`/OCI 目录，快速发起第一次层分析。
- **关键对象**：镜像参考（`image_ref`）、目标平台架构（`arch`）、本地来源、历史记录与示例镜像。
- **常驻动作**：输入分析、打开本地文件… (`Mod+O`)、浏览本地 Docker/Podman… (`Mod+Shift+O`)、点选示例镜像芯片、跳转最近分析记录。
- **版式与对齐脊线**：
  - 居中工作区最大宽度 `640pt`，垂直居中偏上留白（避免空旷漂浮）。
  - 自上而下：**领域标识微标（Docker · OCI · layers）** → **标题（`text-2xl`）** → **一句话说明** → **大尺寸复合搜索框（`Size::Lg`, `40pt` 高，圆角 `radius-lg`：输入框 + 架构 `Select` + `Analyze` 主提交按钮）** → **次级本地入口行（`Open local file…` 与 `Browse Docker/Podman…` 轮廓按钮，禁止主按钮竞争）** → **示例镜像胶囊（`redis:alpine`, `nginx:alpine` 等）** → **最近分析历史（若有，采用 1 列清爽列表与时间/评分右对齐脊线）**。
- **五态设计**：
  - *空态/首次打开*：无历史记录时隐藏历史区，仅保留示例镜像与明确操作提示。
  - *加载中（< 500ms）*：搜索框提交按钮进入加载态禁用重复提交；若命中本地持久化缓存则直接进入工作台，避免闪烁。
  - *加载中（>= 500ms 渐进预览态 Progressive Preview）*：当首次远端分析超过 500ms 未返回时，平滑切换至渐进预览卡片——先基于 `Analyzer::preview` 渲染出 Manifest 层摘要表（平台、层数、总压缩体积、每层尺寸与指令骨架），顶部附带 `ProgressBar` 实时反映 `ProgressCallback` 的分层下载与解压进度，右侧提供明确可点的 `Cancel` 按钮取消在途异步任务。
  - *失败态*：Inline 错误卡片，说明精确错误分类（网络超时 / 仓库不可达 / 镜像不存在 / 需认证凭证 / 格式不支持）及恢复动作（重试、检查本地凭证或更换架构），同时保留用户输入。
  - *权限受限*：当尝试读取系统受保护目录 `file://` 或无权访问 Docker socket 时，明确提示 OS 文件授权要求或 Docker daemon 权限状态。

### 视图 2：Image Inspector Workspace（主检视工作台）

- **主要任务**：深入检查单个镜像的效率、层构成、文件变化、空间占比、安全与运行隐患。
- **核心对象**：`AnalyzeReport`（概览描述、风险标签、`Layer[]`、`FileTreeList[][]`、`wastedList`、`sensitiveFiles`、`duplicateGroups`、`bigModifiedFileList`、`recommendations`、`dockerfile`）。
- **区域拆解与对齐脊线**：
  1. **Stale 离线缓存横幅（条件出现）**：仅当 `stale_as_of` 有值时出现在 KPI 顶部，使用 `warning-soft` 底色 + 警告图标 + 说明文字 + 右侧 `Retry` 按钮；在线新鲜结果不占用垂直高度。
  2. **KPI Strip（顶层摘要条）**：单块浅底发丝线横条，内部以 `1pt` 垂直分隔线区分平等指标：`效率评分（彩色状态文本 + 图标）` | `镜像解压大小` | `浪费空间（若 >0 配警告色）` | `OS/ARCH` | `Base OS` | `运行用户` | `Runtime libc`；末尾若存在风险标签（`tags`）则克制展现（单条中性或语义小徽章，限制总数溢出提示）。
  3. **左侧 Master 栏 — `LayerTimeline`（镜像层时间线）**：
     - 顶栏标题 `Layers` + 右对齐计数与总压缩大小；下一行 3 段式过滤胶囊 `All (N)` / `Has files (M)` / `> 1 MB (K)`。
     - 列表采用严格对齐的 5 条垂直脊线，整行圆角背景填充标记选中态（**禁止**左侧装饰色条）：
       - 轨道 1（`24pt`，Mono 右对齐）：层序号 `01`, `02`…
       - 轨道 2（`56pt`，左对齐）：固定宽 `OpcodePill`（`FROM` / `RUN` / `COPY` / `CMD` / `META`）
       - 轨道 3（`64pt`，Mono 右对齐 `tabular_nums`）：压缩字节大小（`0 B` 静音显示）
       - 轨道 4（`44pt`）：`SizeMicroBar`（直观反映该层占比最大层的比例）
       - 轨道 5（`flex_1 min_w_0`，单行省略截断）：格式化指令摘要
     - 支持右键 `ContextMenu`：复制完整命令、复制层 Digest、仅筛查本层文件。
  4. **右侧 Detail 栏 — 4 个工作台 Tab**：
     顶部单一滑块指示型 `Tabs`：`File Tree (n)` | `Treemap` | `Findings (n)` | `Dockerfile`。
     - **Tab 1：`File Tree`（虚拟化文件树，最高频区）**：
       - **顶部可折叠 `CommandInspector`**：默认最多展示 6 行格式化构建指令，可点击 `Show N more lines` 展开/收起；元信息条展示 `#序号` + `Opcode` + 压缩/解压大小 + 压缩比 + 文件变更统计（`(+a / ~m / -r)`）+ 一键复制 Digest（点击后切换至 `Copied` 反馈 1.5s）+ 自动换行切换。
       - **过滤工具栏**：关键词搜索框（带 `/` 快捷键提示与清除按钮）+ `Modifications`（仅看改/删）+ 最小体积过滤下拉（All / `> 100 KB` / `> 1 MB` / `> 10 MB`）+ `Compact paths` + `Mode & UID` 勾选 + `Expand all` / `Collapse all`（工具栏控件统一 `Size::Sm` 24pt 高度与基线对齐）。
       - **固定表头 + `virtual_list` 虚拟化树表**：共享一致列轨道与内边距脊线，右侧滚动条紧贴滚动区右边缘，不被内边距拉进内部：
         - 列 1 `Status`（`44pt`）：固定符号 `+` / `~` / `−` 配状态色；
         - 列 2 `Mode`（可选，`84pt` Mono）与 列 3 `UID:GID`（可选，`72pt` Mono）；
         - 列 4 `Size`（`76pt` Mono 右对齐 `tabular_nums`）；
         - 列 5 `Bar`（`52pt` 相对本层最大文件占比微条）；
         - 列 6 `Path`（`flex_1 min_w_0`：树深度缩进 + `16pt` 固定图标槽 [折叠/展开箭头 + 文件夹/文件/链接图标] + 文件/目录名；行右侧保留 `48pt` 悬停/聚焦/右键一致可达的操作区，**不可仅在 hover 时暴露唯一预览入口**——支持选中行按 `Enter` 或双击预览、右键 `ContextMenu` 预览/复制路径/导出文件）。
       - **身份保持**：切换过滤、排序或展开父节点时，基于文件完整路径（Domain Identity）保持焦点/选中行，不按裸行索引丢失。
     - **Tab 2：`Treemap`（空间分布图）**：
       - 顶栏：面包屑路径（支持一键点击任意父目录回退、`Root` 起点）+ 右侧 `Map` / `Table` `SegmentedControl`（提供同等能力的无障碍/精确数据视图）+ Top-3 层色图例（`Layer 1` / `Layer 2` / `Layer 3` / `Other`）。
       - 图形区：基于 `nsh_base::plot` 的 `Treemap`（或经 `Canvas` 渲染并与无障碍辅助节点同步的原生双层 Treemap），面积映射解压大小、色槽映射写入层；只在大块满足几何阈值时渲染 2 级子区块与文字标题；点击目录块下钻、悬停/键盘移动时在固定信息栏或锚定详情浮层就地更新路径/体积/写入层百分比，**禁止频繁淡出淡入闪烁**。
     - **Tab 3：`Findings`（诊断发现与优化）**：
       - `>= 1100 pt` 下采用 2 列等宽网格（`gap-6`），卡片为无阴影发丝线 `Panel`；`Optimization Recommendations` 跨双列置顶：
         1. `Optimization Recommendations`（按严重度分级徽章 + 预计节省空间 + 修复命令/Dockerfile 建议代码片段）；
         2. `Wasted Space`（汇总浪费大小、重复/删除次数、写入层 → 删除/覆盖层关系说明 + 路径列表）；
         3. `Sensitive Files`（匹配路径、匹配规则原因、所属层）；
         4. `Cross-Layer Duplicates`（blake3 相同内容跨层副本数、单文件体积、额外浪费空间）；
         5. `Large Modified / Added Files`（上层新增或变更的大文件列表）。
       - 若某项卡片无问题发现（例如 0 敏感文件、0 重复），用精简 `EmptyState` 正面陈述「No sensitive files detected」，不展示空表格骨架。
     - **Tab 4：`Dockerfile`（反推重建脚本）**：
       - 顶部工具栏：`Run as user` 芯片 + `Wrap lines` 开关 + `Copy` 按钮 + `Export…` 按钮；主体为占满剩余高度的只读 `CodeViewer`（`nsh_base::editor()`，语言设为 `"dockerfile"`，带行号与语法高亮）。
  5. **底部状态栏（`24pt` 高）**：左对齐当前上下文键盘快捷键提示（随当前 Tab 与焦点所在区动态提示），右对齐缓存命中状态与当前数据源模式（Registry / Local Docker / Local File）。

### 视图 3：File Preview Sheet（文件预览侧抽屉）

- **主要任务**：在不打断当前层与文件树浏览上下文的前提下，安全检查容器镜像内任意文本、配置或二进制文件开头内容，或将其导出到本地磁盘。
- **交互与布局**：从右侧滑入的 `Sheet`（支持桌面宽 `560–720 pt` 与遮罩点击 / `Esc` 关闭，关闭后焦点自动还原至触发的文件行）。
- **内容对齐**：
  - Header：左侧文件图标 + 完整路径（可折行），右侧关闭按钮 `×`。
  - Metadata Strip：`Layer #N` + `Size` + `Mode` + `UID:GID`，右侧置 `Copy` 与 `Save as…`（调用原生 `pick_save_path` 写入文件，避免浏览器直下限制）。
  - Body：若超出预览上限显示柔和 `Truncated` 说明；文本文件直接复用只读 `CodeViewer` 并按文件后缀自动启用语法高亮；二进制文件提示「Binary file — first bytes in hex」并以等宽 16 字节/行展示 `Offset | Hex | ASCII`。
  - 空文件 / 无读权限（如设备节点/损坏 tar 项）：展示清晰占位与原因说明。

### 视图 4：Image Diff View（镜像对比视图）

- **主要任务**：量化对比两个镜像版本或同一镜像不同架构之间的体积增长、新增/失效层、目录膨胀原因及新出现的安全/优化建议。
- **发起入口（`Compare with…` Dialog）**：在 Inspector 右上角或 `Mod+D` 唤起，焦点自动落在基准镜像输入区（保留当前目标镜像名与一键多架构快捷对比按钮，如 `Compare AMD64 with ARM64`），仅一个主按钮 `Compare`。
- **对比主界面脊线**：
  - **顶栏卡片**：左右对称展示 `Base` 与 `Target` 镜像引用（若命中 Stale 缓存附时间提醒）+ `Swap`、`Open Base`、`Open Target` 三个平级动作按钮；下方 4 列表格严格右对齐指标：`Image Size` | `Compressed` | `Layers` | `Files`，增长用 `diverge-up` + `+` 号，减小用 `diverge-down` + `−` 号。
  - **分段内容区**：
    1. *Layers Diff*：提示共同前缀层数量与字节数（或「No shared layers」），下方并排对比 `Only in Base` 与 `Only in Target` 层清单；
    2. *Where the Size Changed（目录变化）*：左右对齐目录名 + 净变化字节 + 发散比例条（正红橙向右、负蓝向左）；
    3. *File Changes（文件变动）*：内部 Tabs 切分 `Added` / `Removed` / `Resized`，采用 `virtual_list` 渲染最大变更文件清单（超限显示「The largest N of M are shown」）；
    4. *Recommendations Diff*：分离展示 `New`（新增风险）与 `Resolved`（已解决优化）。
- **双镜像异步加载态与局部失败**：对比同时分析两张镜像，若任一失败，明确指出是 `Base` 还是 `Target` 拉取失败并允许直接修改或重试，不丢失用户已填的另一侧引用。

### 视图 5：Local Docker / Podman Image Picker（本地镜像选择弹窗）

- **主要任务**：弥补现有纯手工输入 `docker://` 协议的繁琐，直接枚举并筛选本机 Docker / Podman 守护进程中的镜像。
- **设计要求**：
  - Centered `Dialog`（宽 `640pt`，最大高 `520pt`）。
  - 顶部栏：`Docker` / `Podman` 运行时 `SegmentedControl` + 刷新按钮 + 实时关键词过滤框（打开自动聚焦）。
  - 中部 `virtual_list`：对齐脊线 `Repository:Tag`（加粗） | `Image ID (短 12 位 Mono)` | `Created` | `Size (右对齐 Mono)`。键盘上下移动、回车直接选择触发分析。
  - 底部：提示说明（调用本地 CLI 导出分析）+ `Cancel` + `Analyze selected`（唯一主按钮，未选中行时禁用）。
  - **异常处理**：若检测到本机未安装或未启动 Docker/Podman daemon，不显示空白列表——立即渲染清晰恢复向导（提示启动 daemon、切换另一种运行时，或提供直接跳转 `Open tar / OCI archive…` 按钮）。

### 视图 6：Command Palette（`Mod+K` 全局命令面板）

- **主要任务**：键盘流开发者在不离开键盘的前提下直达全应用命令、切换视图、检索最近镜像或切换外观设置。
- **实现基础**：封装 `nsh_base::command()` 虚拟化命令列表，按分组归类：**Recent Images（最近镜像）**、**Inspector Tabs（跳转标签）**、**Actions（本地打开 / 对比 / 导出 / 重试）**、**Appearance（主题 / 语言）**。
- **交互细节**：全局快捷键 `Mod+K` 随时唤起，输入即时过滤，`↑`/`↓` 移动，`Enter` 执行并关闭窗口（或直接执行切换类命令），`Esc` 关闭并把焦点还原至唤起前控件。

---

## 7. 动效体系与全屏 Motion Inventory（必需）

> **声音策略**：`Diving Desktop` 为专注分析与检查的开发者工具，默认并始终保持**静默（Silent）**，不添加界面交互音效。

### 7.1 Motion Tokens（定义于 `app/src/ui/motion.rs`）

全部时长、缓动与几何位移均收敛至 Token，禁止组件内部写死数字；桌面端所有高频指针响应保持在 `200ms` 以内，离场速度始终快于入场速度：

| Token 名称 | 参数值 | 语义用途 |
| --- | --- | --- |
| `duration::INSTANT` | `120ms` | 悬停洗色、边框强调色切换、微弱文字透明度变化 |
| `duration::FAST` | `160ms` | 按钮按压形变、开关拇指滑动、Tooltip 显现/收起、Popup 离场 |
| `duration::BASE` | `220ms` | Tab 滑块位移、分段控制器滑块切换、图标状态互换（Copy → Copied）、列表卡片展开 |
| `duration::SLOW` | `320ms` | Dialog / Popover / Menu 入场揭示、渐进分析预览页过渡 |
| `duration::SLOWER` | `450ms` | 仅用于初次进入工作台时分层列表前 10 项温和波浪入场（封顶延迟） |
| `easing::EASE_OUT` | `cubic-bezier(0, 0, 0.2, 1)` | 所有入场与显露 |
| `easing::EASE_IN_OUT` | `cubic-bezier(0.4, 0, 0.2, 1)` | 在两个位置之间平滑移动（Tab 滑块、Switch 拇指） |
| `easing::EASE_IN` | `cubic-bezier(0.4, 0, 1, 1)` | 浮层退出、提示收起 |
| `easing::SOFT_SETTLE` | `cubic-bezier(0.22, 1, 0.36, 1)` | 按钮按压与微弹性回落（AffectUI 风格） |
| `spring::PANEL` | 临界阻尼 `Spring`（damping_ratio ≈ `1.0`, stiffness ≈ `280`） | File Preview Sheet 侧抽屉滑入滑出，支持手势/关闭中途反向打断 |
| `distance::SM / MD / LG` | `4pt / 8pt / 12pt` | 文字微升 (`4pt`)、Menu/Popover 生长偏移 (`8pt`)、Dialog 入场位移 (`12pt`) |
| `scale::PRESS_BTN / PRESS_ICON` | `0.97 / 0.95` | 文本按钮与正方形图标按钮按压反馈（**列表行严禁缩放，仅加深背景**） |
| `scale::OVERLAY_ENTER` | `0.97` | 浮层/弹窗从 `0.97` 放大进入 |

### 7.2 逐屏 Motion Inventory（完整 9 类状态盘点）

每条在对应屏上实现并经由真机检查后才算完成；`reduce_motion()` 开启时均按最后一列执行，任何核心操作与状态均不依赖动效即可理解。

#### 7.2.1 全局壳层、标题栏与通用控件

| 状态变化类型 | 具体场景 | NSH 实现工具与 Tokens | 减弱动态效果 (`reduce_motion`) |
| --- | --- | --- | --- |
| 1. 按压与悬停 | 所有 `Button` / `IconButton` | Hover：`transition_colors` (`INSTANT`, `EASE_OUT`)；Press：`transition_transform` (`FAST`, `SOFT_SETTLE`, `scale(0.97)` / 图标 `0.95`) | 保留清晰状态切换，关闭缩放形变与过渡时长（`0ms`） |
| 1. 按压与悬停 | 标题栏图标按钮、列表/表格行 | 列表行仅变底色（`transition_colors`, `INSTANT`），**严禁缩放** | 立即切色 |
| 2. 图标与文案切换 | `Copy` ⇄ `Copied` 反馈、暗色/亮色太阳月亮图标 | `AnimatedValue` 交错缩放+淡入淡出（旧图标缩至 `0.25` 淡出，新图标放大进入，`BASE`） | 立即替换文案与图标 |
| 3. 选中项移动 | Tab 切换（Files / Treemap / Findings / Dockerfile）、分段控制（Light/Dark/Sys, EN/中） | 单一滑块背景用 `AnimatedValue` + `timing` (`BASE`, `EASE_IN_OUT`) 随当前项平移，不逐个销毁重绘 | 滑块瞬间跳至目标项 |
| 5. 浮层开关 | `Menu`、`ContextMenu`、`Select`、`Popover` | `popup_starting_style` / `popup_ending_style`：从锚点角 `scale(0.97)` + `8pt` 偏移 + 透明度淡入（开 `BASE`/`EASE_OUT`，关 `FAST`/`EASE_IN`） | 立即出现/关闭 |
| 5. 浮层开关 | `Dialog`（对比/本地镜像选择器）、`CommandPalette` | `motion()`：backdrop 淡入 + 内容区上浮 `12pt` 与 `scale(0.97)`（`SLOW`, `EASE_OUT`）；退出用 `FAST`, `EASE_IN` | 立即显示/消失 |
| 8. 加载与进度 | 顶部异步分析进度条、按钮提交 Spinner | 确定进度：宽度由 `AnimatedValue` 平滑插值 (`FAST`)；未知等待：细微脉冲透明度 | 确定进度直接跳转；静止占位图标不旋转脉冲 |
| 9. 持续或实时状态 | 未触发交互的静置工具栏与数据面板 | **无动效**（克制专业工作台，静止区域绝不做呼吸、光晕或漂浮装饰） | 同上 |

#### 7.2.2 Landing（首页）与渐进分析预览（Progressive Preview）

| 状态变化类型 | 具体场景 | NSH 实现工具与 Tokens | 减弱动态效果 (`reduce_motion`) |
| --- | --- | --- | --- |
| 4. 内容到达/离开 | 首次请求 > 500ms 切入 Progressive Preview 骨架卡片 | `motion().entering(..)` 上浮 `8pt` + 淡入 (`BASE`, `EASE_OUT`)；避免直接闪烁 | 立即呈现预览页 |
| 4. 内容到达/离开 | 本地/历史镜像列表初加载 | 仅首次挂载前 10 行做交错入场（每行 +40ms 上限 400ms，`BASE`）；重排序与复用无波浪 | 直接全部静态出现 |
| 6. 布局变化 | 提交失败后 Inline Error 横幅展开/收起 | 在状态变更前调用一次 `LayoutAnimation` (`FAST`, `EASE_OUT`) | 立即展开/收起 |
| 8. 加载与进度 | 逐层下载/解压百分比更新 | `ProgressBar` 轨道指示条基于节流后的 `ProgressCallback` 平滑过渡 (`FAST`)；**禁止**在列表数字刷新上做数字跳动动画 | 进度条直接更新宽度 |

#### 7.2.3 Image Inspector Workspace（主检视工作台）

| 状态变化类型 | 具体场景 | NSH 实现工具与 Tokens | 减弱动态效果 (`reduce_motion`) |
| --- | --- | --- | --- |
| 2. 图标与文案切换 | 文件树目录折叠/展开箭头（▸ → ▾） | 图标旋转 90° (`transition_transform`, `FAST`, `EASE_IN_OUT`) | 立即切换箭头朝向 |
| 3. 选中项移动 | `LayerTimeline` 当前层切换 | 选中层背景使用瞬时状态切色或轻量平移高亮 (`FAST`)；列表快速键盘连按可打断并直达最终位置 | 立即切换选中底色 |
| 4. 内容到达/离开 | Stale 缓存横幅出现或被关闭 | `motion()` 纵向进入/退出（`BASE`） | 立即出现/关闭 |
| 5. 浮层开关 | `CommandInspector` 长指令折叠与展开（「Show N more lines」） | 调用一次 `LayoutAnimation` (`BASE`, `EASE_OUT`) 让下方工具栏与树平滑退让 | 立即展开/收起 |
| 6. 布局变化 | 窄窗口或用户切换左侧栏显示/折叠、勾选显示 `Mode & UID` 列 | 由用户点击触发的显隐切换前调一次 `LayoutAnimation` (`BASE`)；**拖拉窗口 resize 过程中严禁启动 LayoutAnimation** | 立即调整分栏 |
| 7. 拖拽与释放 | `LayerTimeline` 与主面板中间分割条拖拽 | **拖拽过程中严格 1:1 跟随指针（0 延迟、不加缓动）**；到达最小/最大边界即止 | 1:1 跟随 |
| 9. 持续或实时状态 | `virtual_list` 数万行文件树滚动、过滤关键词即时刷新 | **显式无动效**：高频率复用单元、行内数字与文件过滤必须 0ms 瞬时响应，绝不能在回收单元挂 `motion` 或入场动画 | 无动效 |
| 9. 持续或实时状态 | Treemap 指针在各目录块间移动悬停 | **显式无动效**：悬停信息板在固定位置就地替换文本与数值（`No flicker`），严禁每过一个矩形就淡出重新入场 | 无动效 |
| 9. 持续或实时状态 | Treemap 点击目录下钻 / 面包屑回退 | 新层级区域整体微缩淡入 (`FAST`, `EASE_OUT`) | 立即切换视图层 |

#### 7.2.4 File Preview Sheet（预览侧抽屉）、Local Picker 与 Diff View

| 状态变化类型 | 具体场景 | NSH 实现工具与 Tokens | 减弱动态效果 (`reduce_motion`) |
| --- | --- | --- | --- |
| 5. 浮层开关 | `FilePreviewSheet` 右侧滑入与关闭 | NSH `0.6.0` `Transition::spring(spring::PANEL)` 从右侧边缘滑入，背景遮罩淡入 (`BASE`)；支持关闭中途再选中另一文件直接替换内容而**不闪关抽屉** | 立即显现/关闭 |
| 7. 拖拽与释放 | 若支持抽屉左边界调宽拖拽 | 拖拽跟随鼠标 1:1 变更宽度，松手吸附或保留当前宽度 | 1:1 跟随 |
| 3. 选中项移动 | Diff View 顶部 `Added / Removed / Resized` 子 Tab 切换 | 与全局 `Tabs` 共用单一 `AnimatedValue` 滑块 (`BASE`, `EASE_IN_OUT`) | 立即跳切 |
| 4. 内容到达/离开 | 点击 Diff View 的 `Swap`（交换 Base/Target） | 双侧标题栏与指标数据轻量交叉淡入 (`FAST`)，明确反馈已对调 | 立即刷新 |

---

## 8. 界面文案规范与中英双语词典（EN / ZH）

### 8.1 文案书写原则（Interface Language）

遵守 NSH `Design Guides` 接口语言准则，结合上下文做「上下文经济」裁剪：

1. **英文统一使用 Sentence case**：`Analyze image`、`Compact paths`、`Show mode & UID`、`Compare with…`，仅专有名词和缩写全大写（`OCI`、`UID:GID`、`MCP`、`Dockerfile`）。
2. **弹窗与二步指令必须带单字符省略号 `…`**：凡需要进一步输入、弹窗或系统选择器才能完成的命令（如 `Compare with…`、`Open file…`、`Local images…`、`Save as…`、`Check for updates…`），末尾必须使用单字符 `…`（禁止三个英文句点 `...`）；立即执行的动作不加省略号（`Copy`、`Swap`、`Analyze`）。
3. **短标签与按钮不带句号**：表格头、按钮、Tab、开关标签、菜单项末尾不加标点；完整解释性描述、告警或报错句子末尾必须加标准标点（英文 `.`，中文 `。`）。
4. **按母语重写而非逐词翻译**：中文保持简洁工程表达，英文去掉 `Please`、`Successfully`、`Are you sure you want to…` 等仪式废话。
5. **错误文案结构**：`发生了什么` + `明确的下一步恢复动作`（如 `Couldn't pull manifest from ghcr.io. Check network connectivity or registry credentials and try again.`）。

### 8.2 核心词库与双语对照表

| 语义标识 | 英文界面文案 (`en`) | 中文界面文案 (`zh`) | 适用上下文与规范说明 |
| --- | --- | --- | --- |
| `action_analyze` | `Analyze` | `分析` | 搜索框提交按钮（立即触发，无省略号） |
| `action_open_file` | `Open file…` | `打开本地文件…` | 打开 `file://` 系统文件选择器（二步交互带 `…`） |
| `action_local_images` | `Local images…` | `本地镜像…` | 打开 Docker/Podman 本地镜像选择器（带 `…`） |
| `action_compare` | `Compare with…` | `对比…` | 唤起基准镜像选择弹窗（带 `…`） |
| `action_save_as` | `Save as…` | `另存为…` | 预览抽屉导出单个文件到本地磁盘 |
| `action_export_md` | `Export Markdown…` | `导出 Markdown…` | 导出分析或对比 Markdown 报告 |
| `action_copy` / `copied` | `Copy` / `Copied` | `复制` / `已复制` | 立即动作，变更标签无需弹窗 |
| `action_swap` | `Swap` | `交换基准` | 对调 Diff 两侧镜像 |
| `tab_files` | `File Tree` | `文件树` | Inspector 主 Tab 1 |
| `tab_treemap` | `Treemap` | `空间分布` | Inspector 主 Tab 2 |
| `tab_findings` | `Findings` | `分析发现` | Inspector 主 Tab 3 |
| `tab_dockerfile` | `Dockerfile` | `Dockerfile` | Inspector 主 Tab 4 |
| `filter_modifications` | `Changes only` | `仅改/删` | 过滤文件树只显示新增、修改或被删除项 |
| `filter_compact_paths` | `Compact paths` | `合并单层目录` | 文件树单子目录合并展示 |
| `filter_meta_cols` | `Mode & UID` | `权限与属主` | 文件树显示权限模式和属主组列 |
| `state_stale_notice` | `Registry unavailable. Showing analysis cached at {time}.` | `无法访问镜像仓库，当前显示 {time} 的本地缓存结果。` | 完整句带句号，后跟 `Retry` |
| `err_docker_down` | `Docker daemon isn't running. Start Docker or open a tar/OCI file instead.` | `Docker 服务未运行。请先启动 Docker，或直接打开本地 tar/OCI 归档文件。` | 报错含恢复路径 |
| `err_permission` | `Permission denied reading "{path}". Grant file access in system settings or choose another file.` | `无权读取「{path}」。请在系统设置中授予文件读取权限或重新选择文件。` | 权限受限提示 |

---

## 9. 键盘交互、快捷键与无障碍规范（阻塞项）

任何无法仅凭键盘或屏幕阅读器（macOS VoiceOver / Windows Narrator / Linux Orca）完整走通核心路径的界面均视为**未完成**。

### 9.1 全局与区域快捷键矩阵

说明：`Mod` 在 macOS 自动映射为 `⌘ Command`，在 Windows 与 Linux 自动映射为 `Ctrl`。

| 快捷键 | 作用域 | 执行动作 | 说明与提示策略 |
| --- | --- | --- | --- |
| `Mod + K` | 全局 | 打开 / 关闭命令面板 (`CommandPalette`) | 最快入口；关闭后焦点返回原控件 |
| `Mod + L` | 全局 | 聚焦顶部搜索输入框并全选现有镜像名 | 与浏览器/桌面端一致的地址聚焦习惯 |
| `Mod + O` | 全局 | 打开本地文件选择器 (`file://`) | 触发系统原生 `pick_paths` |
| `Mod + Shift + O` | 全局 | 打开本地 Docker / Podman 镜像选择器 | 触发视图 5 Dialog |
| `Mod + D` | Inspector 态 | 打开 `Compare with…` 对比弹窗 | 若当前已分析镜像，默认作为 Target |
| `Mod + R` | Inspector / Diff | 强制忽略内存缓存重新分析当前镜像 | 更新陈旧分析结果 |
| `Mod + 1` ~ `Mod + 4` | Inspector 态 | 切至 `File Tree` / `Treemap` / `Findings` / `Dockerfile` | 原生应用菜单内标示 |
| `/` | Inspector (`File Tree`) | 聚焦文件树关键词过滤框（输入框未聚焦时生效） | 状态栏常驻提示 |
| `↑` / `↓` | 焦点在 `LayerTimeline` | 切换上一个 / 下一个镜像层并同步详情区 | 无需按 Enter 即可联动 |
| `↑` / `↓` / `←` / `→` | 焦点在 `VirtualFileTree` | 上下移动高亮文件行；`→` 展开目录，`←` 折叠或退回父目录 | 遵循原生树网格键盘语义 |
| `Enter` / `Space` | 焦点在树中文件行 | 对文件打开 `FilePreviewSheet`；对目录切换展开态 | 等同于鼠标双击文件 |
| `Tab` / `Shift + Tab` | 全局 | 按视觉与阅读逻辑顺序移动焦点 | 弹出层内锁定循环焦点 |
| `Esc` | 全局 / 任何浮层 | 先关闭最顶层弹窗/抽屉/菜单；若在过滤框则清空焦点与关键词 | **绝不**误退出整个应用 |

### 9.2 焦点管理与辅助功能强制契约

1. **外置高清 Focus Ring**：所有可聚焦控件统一配置 `.focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))`（或 2pt 外扩偏置环），仅在鼠标点击时不打扰视觉，键盘跳转时清晰锐利。
2. **Overlay 焦点闭环**：打开 `Dialog`、`Sheet`、`CommandPalette` 或 `Menu` 时，焦点自动移入其内部首个可交互元素；按下 `Esc` 或点击完成关闭后，**必须将焦点精准归还至原先的触发控件**。
3. **语义角色完整性**：禁止用可点击的 `view()` 冒充按钮或列表选项；`ui::Button`、`ui::Tabs`、`Menu`、`virtual_list` 行必须完整传递 `AccessibilityRole` 与选中/展开/禁用状态。
4. **纯图标必须命名**：全部仅图标控件强制在编译期 API 签名为必传 `accessibility_label: impl Into<SharedString>`；装饰性图标、分隔符与微条标注隐身（`accessibility_elements_hidden(true)`）。
5. **多模态状态编码**：
   - 文件树操作不单靠绿/黄/红着色，行首必带 `+`、`~`、`−` 等宽字符并暴露为可读标签（如 `Added, usr/bin/redis-server, 12.4 MB`）。
   - Diff 变化不单靠红/蓝微条，数值本身强制携带 `+` 或 `−` 符号。
   - 测试规范：`nsh test` / `nsh_testing::Headless` 通过 `role:button[Analyze]` 等无障碍选择器定位关键元件，顺带倒逼辅助功能标识 100% 准确。

---

## 10. 视觉与质量验收门禁（Finishing & Design Review Checklist）

### 10.1 分阶段验收机制

1. **阶段 0（进入业务页面前的 UI 基础门禁）**：在写第一行分析工作台界面代码之前，必须先完成 `app/theme.json`、`app/src/ui/` 与 `app/src/ui/gallery.rs`，在真实运行窗口中截图检查 Light / Dark 双色下每一类控件的所有尺寸与交互态，全数通过后才解锁 `features/` 开发。
2. **逐屏上线门禁**：每完成或改动任一视图，必须在运行中的 App（`nsh run`）实测 **Light + Dark 双主题**、**默认 (`1280×820`) + 最小 (`960×640`) 窗口尺寸**、**放大字体缩放**、以及 **开启系统 Reduced Motion** 下的表现，逐项核对下表并给出 `nsh run --trace` 日志确认。

### 10.2 执行级 Finishing Checklist（逐项核查，未核即未过）

#### 桌面自绘标题栏（NSH 0.6.1 新增阻塞项）
- [ ] 系统标题栏已隐藏（`TitleBarStyle::Hidden`），应用上方无系统灰/白原生标题横条。
- [ ] 原生窗口按钮安全空间（macOS 红绿灯 `left`、Win/Linux 窗口控制按钮 `right`）直接读取自 `title_bar()`，窗口首行直接承载品牌/搜索/操作而非空条。
- [ ] 标题栏空白背景支持拖拽窗口与双击缩放，交互按钮/输入框标记 `.window_drag(false)` 不抢拖拽事件。

#### 圆角与同心几何
- [ ] 全部圆角仅来自 `radius-lg` (`10pt`)、`radius` (`6pt`)、`radius-inset` (`4pt`) 与受限 `rounded-full`，无一处一值字面量。
- [ ] 嵌套圆角满足同心关系 `inner ≈ max(0, outer − gap)`，选中填充与内部滚动列表均正确裁切不溢出。

#### 排版与数字
- [ ] 仅使用 4 级字号，正文不低于 `12pt`；UI 文字使用 `IBM Plex Sans` + 系统 CJK 回退，代码/路径/数字/权限使用 `IBM Plex Mono`。
- [ ] 所有表格/时间线数值右对齐并启用 `.tabular_nums()`，在系统调大字号时按钮与输入行基于 `min_h` 自然伸展而不裁剪文字。

#### 间距与渲染像素对齐
- [ ] 所有间隙来自 `2 / 4 / 8 / 12 / 16 / 24 / 32 pt` 尺度；组件自己管理 padding，父容器只管 `gap_*`，无双倍内边距。
- [ ] 同级区段（`LayerTimeline` 各行、`VirtualFileTree` 表头与行、Diff 指标表）保持严格垂直列脊线；缺失可选列（如无 tag、无 UID）时不引发后续列错位。
- [ ] 分割线由单一边界拥有者绘制 `1pt` 发丝线（无 `2px` 叠边）；垂直滚动条贴近对应滚动区最右外缘。

#### 色彩与表面
- [ ] 代码零字面量 hex/`rgb()`；100% 源自 `app/theme.json` 并在 Light / Dark 下保持足够文字与边界对比度。
- [ ] 主工作台呈扁平表面，阴影仅用于浮层（`Menu`/`Popover`/`Select`/`Tooltip` 统一共用一组处理，`Dialog`/`Sheet` 更高层级）；每块决策区仅一个 `Primary` 按钮。

#### 悬停与按压反馈
- [ ] 所有可点按钮/标签/列表行均在桌面端响应温和悬停洗色，不可点文本/微条无虚假悬停反应。
- [ ] 所有 `Button` / `IconButton` 有显式 `active` 按压缩放（`0.97` / `0.95`）与底色加深；`LayerTimeline` 与 `VirtualFileTree` 行按压仅加深底色，**不缩放**；禁用控件无悬停与按压反应。

#### `ui` 层纯净度与组件规范
- [ ] `features/*` 内无直接导入 `nsh_base::*`，无孤立风格字面量，无用 `view` 手写的伪按钮；全部基础样式均已在 `ui-gallery` 经过验证。
- [ ] `Menu` / `Popover` / `Select` / `Dialog` / `Sheet` 职责不混用，图标按钮全部正方形、尺寸统一且含 Tooltip 与英文 `accessibility_label`。

#### 动效、性能与无障碍（阻塞项）
- [ ] 第 7 节 Motion Inventory 中每条对应动效已在真机验证到位；打开 Reduced Motion 时全部过渡瞬时、无缩放/波浪，且信息理解无损。
- [ ] `nsh run --trace` 下滚动、过滤万级文件树以及在 `LayerTimeline` 快速切换层时，确认 **仅变更节点更新，不重建整颗 `virtual_list` 树**。
- [ ] 脱离鼠标仅凭键盘（`Tab` / `方向键` / `Enter` / `Space` / `Esc` / `Mod+K`）可独立走完「搜索/选本地镜像 → 查看层与文件树 → 打开并导出预览 → 查看 Treemap & Findings → 发起镜像 Diff」全流程；焦点环清晰不被裁剪；VoiceOver 可准确朗读操作类型（`+`/`~`/`−`）与按钮角色。
- [ ] 空状态、渐进预览加载状态、超时报错、Stale 离线状态及权限拒止状态全部完备呈现且具备明确恢复操作。
