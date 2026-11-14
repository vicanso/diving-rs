# Diving Web UI/UX Analysis & Optimization Proposal

> **Publication-Grade Evaluation, Structural Redesign, and Implementation Roadmap for the diving-rs Web Interface**  
> **Target Reference:** [https://diving.vicanso.dev/?image=redis%3Aalpine&arch=amd64](https://diving.vicanso.dev/?image=redis%3Aalpine&arch=amd64)  
> **Repository:** `vicanso/diving-rs`  
> **Status:** Final Proposed Architecture (Milestone M1)  
> **Date:** November 2026  

---

## Table of Contents

1. [Executive Summary & Core Diagnosis](#1-executive-summary--core-diagnosis)
   - [1.1 Context & Purpose](#11-context--purpose)
   - [1.2 Live Web Inspection: `redis:alpine&arch=amd64`](#12-live-web-inspection-redisalpinearchamd64)
   - [1.3 Screen Real Estate & Layout Inefficiencies](#13-screen-real-estate--layout-inefficiencies)
   - [1.4 The Fundamental Architectural Flaw: The Collapsed Layer Dropdown](#14-the-fundamental-architectural-flaw-the-collapsed-layer-dropdown)
   - [1.5 Industry Benchmarking & Comparison Matrix](#15-industry-benchmarking--comparison-matrix)
2. [Comprehensive Panel-by-Panel Audit](#2-comprehensive-panel-by-panel-audit)
   - [2.1 Header & Global Navigation](#21-header--global-navigation)
   - [2.2 Image Layers & Layer Selection Architecture](#22-image-layers--layer-selection-architecture)
   - [2.3 Command History & Layer Metadata Inspection](#23-command-history--layer-metadata-inspection)
   - [2.4 File Tree Browsing & Usability Bottlenecks](#24-file-tree-browsing--usability-bottlenecks)
   - [2.5 File Change Status & Visual Size Contrast](#25-file-change-status--visual-size-contrast)
3. [Structural Proposals & ASCII Wireframes](#3-structural-proposals--ascii-wireframes)
   - [3.1 Full-Page Macro Architecture: Single Column Stack vs. 2-Pane Workspace](#31-full-page-macro-architecture-single-column-stack-vs-2-pane-workspace)
   - [3.2 Component Wireframe: Streamlined Application Header & Omnibar](#32-component-wireframe-streamlined-application-header--omnibar)
   - [3.3 Component Wireframe: Persistent Master Layer Timeline](#33-component-wireframe-persistent-master-layer-timeline)
   - [3.4 Component Wireframe: Beautified Command Inspector & Metadata Strip](#34-component-wireframe-beautified-command-inspector--metadata-strip)
   - [3.5 Component Wireframe: Modernized File Tree with Status Badges & Micro-Bars](#35-component-wireframe-modernized-file-tree-with-status-badges--micro-bars)
   - [3.6 Responsive Behavior & Adaptive Breakpoints](#36-responsive-behavior--adaptive-breakpoints)
4. [Proposed Design System & Style Guide](#4-proposed-design-system--style-guide)
   - [4.1 Semantic Color Token Architecture](#41-semantic-color-token-architecture)
   - [4.2 Disciplined Typographic Scale (Eliminating Fractional Pixels)](#42-disciplined-typographic-scale-eliminating-fractional-pixels)
   - [4.3 Surface Elevation & Hairline Borders (Eliminating the "Border Onion")](#43-surface-elevation--hairline-borders-eliminating-the-border-onion)
   - [4.4 Status Badges, Relative Micro-Bars & Accessibility (WCAG Compliance)](#44-status-badges-relative-micro-bars--accessibility-wcag-compliance)
5. [Prioritized Implementation Roadmap & Technical Guide](#5-prioritized-implementation-roadmap--technical-guide)
   - [5.1 Action Priority Matrix (P0 / P1 / P2)](#51-action-priority-matrix-p0--p1--p2)
   - [5.2 Phase 1: High-Impact Usability & Status Bug Fixes (P0)](#52-phase-1-high-impact-usability--status-bug-fixes-p0)
   - [5.3 Phase 2: Core 2-Pane Master-Detail Workspace Architecture (P1)](#53-phase-2-core-2-pane-master-detail-workspace-architecture-p1)
   - [5.4 Phase 3: File Tree Modernization & Design Tokens (P1)](#54-phase-3-file-tree-modernization--design-tokens-p1)
   - [5.5 Phase 4: Ergonomics, Command Beautifier & Theming (P2)](#55-phase-4-ergonomics-command-beautifier--theming-p2)
   - [5.6 File-by-File Engineering Guide](#56-file-by-file-engineering-guide)
6. [Verification, Smoke Testing & Acceptance Criteria](#6-verification-smoke-testing--acceptance-criteria)

---

## 1. Executive Summary & Core Diagnosis

### 1.1 Context & Purpose
`diving-rs` is a high-performance Docker image layer analysis tool implemented in Rust. In its terminal mode (`TUI`), built with `ratatui`, it delivers an exceptional developer experience: an intuitive side-by-side 50/50 split between layer history on the left and filesystem modifications on the right.

The web edition (`web/`, React 19 + TypeScript + Ant Design 6) expands accessibility by allowing developers, CI pipelines, and cloud environments to inspect images without local Docker daemon dependencies. However, in translating the terminal interface to the browser, the web application deviated significantly from the master-detail paradigm that makes container inspection effective.

This document synthesizes deep-dive forensic audits across the live production deployment and the frontend codebase. It establishes an actionable blueprint to transform the web UI into a publication-grade, minimalist developer workspace inspired by Linear, GitHub, and Raycast.

---

### 1.2 Live Web Inspection: `redis:alpine&arch=amd64`
Analysis of the live reference endpoint `https://diving.vicanso.dev/?image=redis%3Aalpine&arch=amd64` (and its backing JSON API `/api/analyze?image=redis%3Aalpine%3Farch%3Damd64`) reveals exact production characteristics:

- **Target Container:** `library/redis:alpine` (Architecture: `amd64`).
- **Layers:** 14 distinct OCI layers.
  - Compressed transfer payload: **39.5 MB**.
  - Uncompressed filesystem footprint: **120.4 MB**.
  - Layer composition: 7 file-bearing layers (Layers 0, 2, 3, 7, 8, 9, 10) and 7 metadata-only layers (Layers 1, 4, 5, 6, 11, 12, 13) containing `ENV`, `ARG`, `WORKDIR`, `ENTRYPOINT`, `EXPOSE`, and `CMD` instructions.
- **Largest Contributor:** Layer 7 (index 7, 0-indexed; Layer 8 in 1-indexed UI) contributes **35.5 MB compressed** (**110.7 MB unpacked**), introducing 128 modified/added filesystem nodes.
- **Observed User Experience Breakdown:**
  1. *Transitional Disorientation:* During initial fetch (~800ms), an `ImagePreviewCard` displays an ordered `<ol>` list of all 14 layers. Once the full analysis completes, this overview is discarded.
  2. *Layer Obfuscation:* The full layer timeline vanishes, replaced by a single Ant Design `<Select>` dropdown. Half of the entries in this dropdown display the cryptic text `"NONE"` (e.g., `2: NONE`, `5: NONE`, `6: NONE`). Non-empty entries display truncated 8-character hex digests (`8: AB7525B6 (35.5 MB)`) rather than human-readable Dockerfile instructions.
  3. *Command Overwhelm:* Selecting Layer 8 outputs a raw, unformatted 2,528-character wall of Bash script inside a generic `<span>` with `word-break: break-all;`, forcing the file tree table down past the fold.
  4. *Scroll Trap:* The file tree is capped inside a 480px fixed-height inner container. Browsing 646 files in Layer 3 forces the user into an inner scroll trap embedded inside a page that stretches to >3,200 vertical pixels.
  5. *Monochrome File Hierarchy:* File change status relies strictly on text color without badges. Added files receive zero styling (`Op::None`). A 45.1 MB shared library (`rejson.so`) looks visually indistinguishable from a 38 B text file (`shells`).

---

### 1.3 Screen Real Estate & Layout Inefficiencies
The web layout is constrained by hardcoded CSS container widths and linear component stacking:

```
+----------------------------------------------------------------------------------------------------+
| 64px Fixed Header (Wasted center search space; collides with floating GitHub ribbon)               |
+----------------------------------------------------------------------------------------------------+
|                                                                                                    |
|    <-- Wasted Margin (30-40%) -->    +--------------------------------+    <-- Wasted Margin -->   |
|                                      | Content Container              |                            |
|                                      | max-width: 1120px              |                            |
|                                      | (App.css:16 & index.css:40)    |                            |
|                                      |                                |                            |
|                                      | 1. Image Summary Card          |                            |
|                                      | 2. Treemap ("Bytes Are", 360px)|                            |
|                                      | 3. Layer Content (Toolbar,     |                            |
|                                      |    Command, 480px File Tree)   |                            |
|                                      | 4. Findings Grid (6 Cards)     |                            |
|                                      | 5. Reconstructed Dockerfile    |                            |
|                                      | 6. Latest Images List          |                            |
|                                      |                                |                            |
|                                      | Total Depth: > 3,200px         |                            |
|                                      +--------------------------------+                            |
|                                                                                                    |
+----------------------------------------------------------------------------------------------------+
```

#### Diagnostic Metrics:
- **Horizontal Real Estate Loss:** On modern 1440p (2560×1440) and 1080p (1920×1080) monitors, `--content-max: 1120px` leaves over **56% of total screen pixels completely unused**.
- **Vertical Scroll Depth:** Users must scroll through **over 3,200 vertical pixels** to review analysis findings.
- **Inner Scroll Friction:** In `components.tsx:63`, `FILE_TREE_MAX_HEIGHT = 480` creates a cramped nested viewport displaying only 13 table rows at a time (`FILE_TREE_ROW_HEIGHT = 36`). Wheel-scrolling through the file tree routinely hijacks page scrolling, causing abrupt viewport jumps.

---

### 1.4 The Fundamental Architectural Flaw: The Collapsed Layer Dropdown
In container analysis, **the image layer is the primary navigational axis**. A developer inspects a container image to understand *what each layer did*, *why it is large*, and *what files were created, modified, or overwritten*.

In `diving-rs` web (`web/src/App.tsx:444-450`), layer selection was collapsed into a secondary form input:

```tsx
// Current Implementation in App.tsx
<Col xs={24} sm={12} md={8}>
  <Form.Item label={i18nGet("layerLabel")}>
    <Select value={currentLayer} onChange={setCurrentLayer} options={layerOptions} />
  </Form.Item>
</Col>
```

#### Why This Breaks the User Experience:
1. **Loss of Build Sequence Context:** When closed, the dropdown reveals only the active layer. The developer cannot see prior or subsequent layers, total layer count, or relative size distributions.
2. **High Interaction Cost:** Stepping through 14 layers to investigate filesystem mutations requires 28 discrete clicks (open dropdown, locate next layer, select, repeat). In contrast, the terminal TUI requires a single press of `↓`.
3. **Loss of Size Contrast:** The developer cannot visually spot bloat layers at a glance because layer sizes are represented as plain text numbers hidden inside closed dropdown options.

---

### 1.5 Industry Benchmarking & Comparison Matrix

| Evaluation Dimension | `diving-rs` Current Web | `diving-rs` Terminal TUI | Docker Desktop (Inspector) | GitHub File Explorer | Modern Developer Standard (Linear/Raycast) |
|---|---|---|---|---|---|
| **Layout Paradigm** | Single-column 1120px stack | 50/50 horizontal split | Master-detail split + contextual tabs | Sidebar tree + file viewport | Full-viewport master-detail workspace |
| **Layer Browsing** | Hidden `<Select>` dropdown | Dedicated table with index, size & command | Proportional visual bar stack | N/A | Persistent visual timeline with key navigation |
| **Command Readability** | Unformatted 2,528-char wall | Truncated single line | Instruction list with copy | N/A | Formatted, folded Bash inspector with 1-click copy |
| **File Tree Viewport** | 480px inner scroll trap | Full terminal height | Fluid height | Fluid viewport height | Adaptive fluid height with fullscreen toggle |
| **Change Status** | Color-only text; Added status missing | Text tags | Badges (`Added`, `Modified`) | Git status pills (`M`, `A`, `D`) | High-contrast accessible badges `[+ ADD]`, `[~ MOD]` |
| **Size Contrast** | Flat text (45 MB == 38 B) | Monospace size column | Bar indicators | Byte sizes | Relative visual micro-bars & heat typography |
| **Visual Styling** | 4-5 nested borders ("onion") | Clean ANSI borders | Clean borderless sections | Hairline dividers | Flat elevation, 1px subtle hairline borders |
| **Theming** | OS matchMedia only | ANSI colors | Dark / Light toggle | System / Dark / Light | Persistent Dark / Light / System header toggle |

---

## 2. Comprehensive Panel-by-Panel Audit

### 2.1 Header & Global Navigation

#### 1. 64px Sticky Real Estate & Missing Persistent Breadcrumbs
- **Location:** `web/src/App.css:97-150`, `web/src/App.tsx:556-577`.
- **Finding:** The header remains fixed at the top of the viewport (`height: 64px`, `background: color-mix(in srgb, var(--bg-elevated) 88%, transparent)`). However, 75% of its width is dedicated to an oversized search input that redundantly displays the query string `redis:alpine`.
- **Usability Defect:** As the user scrolls down to inspect Layer 8 or the Findings Grid, the primary context—the image repository, active tag, digest, and architecture—scrolls off-screen. The sticky header fails to provide a persistent context breadcrumb.

#### 2. Floating GitHub Corner Banner Layout Collisions
- **Location:** `web/src/icons.tsx:49-65`, `web/src/App.css:108-114`.
- **Finding:** The GitHub corner SVG ribbon is positioned with `position: fixed; top: 0; right: 0; z-index: 120;`.
- **Usability Defect:** Because it floats independently of the flex header container, it collides with header buttons on viewports between 1024px and 1280px. In `App.css:112`, a brittle responsive hack was applied:
  ```css
  @media (max-width: 1248px) {
    .headerInner.withCorner { padding-right: 52px; }
  }
  ```
  This creates asymmetrical whitespace and clips header actions on smaller screens.

#### 3. Fixed-Width Architecture Dropdown (`96px/108px`)
- **Location:** `web/src/components.tsx:307-319`.
- **Finding:** The architecture selector in `SearchBar` is hardcoded:
  ```tsx
  style={{ width: size === "large" ? 108 : 96 }}
  ```
- **Usability Defect:** Multi-arch platform names like `linux/arm64/v8` or `linux/riscv64` are severely truncated with ellipsis (`ARM64...`), preventing unambiguous architecture verification.

#### 4. Lack of Manual Dark/Light Theme Switching
- **Location:** `web/src/App.tsx:78-92`.
- **Finding:** Theme detection relies strictly on `window.matchMedia("(prefers-color-scheme: dark)")`.
- **Usability Defect:** Developers working in bright ambient conditions or presenting in conference rooms cannot toggle between dark and light themes manually. No toggle control exists in the interface.

---

### 2.2 Image Layers & Layer Selection Architecture

#### 1. Vanishing Layer Timeline Post-Analysis
- **Location:** `web/src/App.tsx:580-586` vs `web/src/App.tsx:612-653`.
- **Finding:** During loading, `ImagePreviewCard` displays an ordered list (`<ol className="previewLayers">`) with Index, Size, and Command for all 14 layers. Upon completion, this element is completely unmounted and replaced by the single `<Select>` dropdown in `getLayerContentView`.
- **Cognitive Impact:** The user is stripped of their global mental map of the image build process exactly at the moment they begin their analysis.

#### 2. The "NONE" Metadata Dropdown Issue
- **Location:** `web/src/App.tsx:367-386`.
- **Code:**
  ```tsx
  let { digest } = item;
  if (digest) digest = digest.replace("sha256:", "").substring(0, 8);
  if (!digest) digest = "none";
  const size = item.size || 0;
  let sizeDesc = size > 0 ? ` (${prettyBytes(size)})` : "";
  return {
    value: index,
    label: `${index + 1}: ${digest.toUpperCase()}${sizeDesc}`,
  };
  ```
- **Finding:** In OCI images, metadata instructions (`ENV`, `ARG`, `EXPOSE`, `CMD`, `ENTRYPOINT`) do not produce filesystem layer blobs; their `digest` is empty and `size` is 0.
- **Usability Defect:** In `redis:alpine`, 7 out of 14 layers are rendered as:
  `2: NONE`, `5: NONE`, `6: NONE`, `7: NONE`, `12: NONE`, `13: NONE`, `14: NONE`.
  Developers cannot tell which layer sets the environment variables, which layer configures ports, or which layer defines the entrypoint without clicking each one blindly.

#### 3. Hex Hash Obfuscation vs Dockerfile Instructions
- **Finding:** For layers that contain files, the dropdown labels them with 8-character hex hashes (e.g., `8: AB7525B6 (35.5 MB)`).
- **Usability Defect:** Container developers identify build steps by their Dockerfile instructions (`RUN apk add...`, `COPY redis.conf...`), not by arbitrary tarball SHA-256 prefixes.

#### 4. Absence of Proportional Visual Size Weight Bars
- **Finding:** Layer weights are listed purely as formatted text (`35.5 MB`, `3.85 MB`, `201 kB`).
- **Usability Defect:** The eye cannot scan the timeline to immediately locate the heaviest steps. In `redis:alpine`, Layer 8 accounts for 90% of the compressed image weight, yet it occupies the same vertical visual weight as a 949 B step.

---

### 2.3 Command History & Layer Metadata Inspection

#### 1. The 2,528-Character Unformatted Bash Wall in Layer 8
- **Location:** `web/src/App.tsx:505-514`, `web/src/App.css:651-665`.
- **Finding:** Inspecting Layer 8 reveals a single `<span>` containing 2,528 characters:
  ```
  RUN |2 REDIS_DOWNLOAD_URL=https://github.com/redis/redis/releases/download/8.10.3/redis-full.tar.gz REDIS_DOWNLOAD_SHA=5e0c00ed77f789fb1a4a30f1fbb58a793cc88c41c0f11f0e38e1b4f192991188 /bin/sh -c set -eux; 	apk add --no-cache --virtual .build-deps 		coreutils 		dpkg-dev dpkg 		gcc 		linux-headers 		make 		musl-dev 		openssl-dev 		zstd-dev 		g++ 	; 	wget -O redis.tar.gz "$REDIS_DOWNLOAD_URL"; 	echo "$REDIS_DOWNLOAD_SHA *redis.tar.gz" | sha256sum -c -; 	mkdir -p /usr/src/redis; 	tar -xzf redis.tar.gz -C /usr/src/redis --strip-components=1; 	rm redis.tar.gz; 	...
  ```
- **Usability Defect:**
  1. *Unescaped whitespace:* Literal tab characters (`\t`) are rendered as raw spacing gaps.
  2. *Zero syntax highlighting:* Shell commands, arguments, and environment variables blend together.
  3. *Zero formatting:* Semicolons (`;`) and chained statements (`&&`) are not broken across lines.
  4. *Missing copy trigger:* There is no 1-click button to copy the command string to reproduce the build locally.

#### 2. BuildKit Metadata Pollution (`RUN |2 ... /bin/sh -c`)
- **Finding:** Modern BuildKit engines prefix commands with internal argument counters (e.g. `|2 REDIS_DOWNLOAD_URL=... /bin/sh -c`).
- **Usability Defect:** The actual shell command (`set -eux; apk add...`) is pushed 180 characters to the right, obscured by build engine boilerplate.

#### 3. Hidden Unpacked Footprints, Compression Ratios, and Digests
- **Location:** `web/src/types.ts:52-60` vs `web/src/App.tsx:505-514`.
- **Finding:** The backend provides:
  ```ts
  export interface Layer {
    created: string;
    digest: string;
    mediaType: string;
    cmd: string;
    size: number;
    unpackSize: number;
    empty: boolean;
  }
  ```
- **Usability Defect:** `unpackSize` (110.7 MB for Layer 8 vs 35.5 MB compressed) is completely ignored in the layer view. The expansion ratio (3.12×) and full content digest are hidden from the developer.

#### 4. Layout Pushing: Forcing the File Tree Below the Viewport
- **Finding:** Because `.layerCmd` expands dynamically to accommodate whatever text length the command has, Layer 8's 2,528-char command pushes the entire File Tree table down by more than 350px.
- **Usability Defect:** When a developer clicks between Layer 7 (0 B, 1-line command) and Layer 8 (35.5 MB, 2,528-char command), the file tree jumps up and down violently.

---

### 2.4 File Tree Browsing & Usability Bottlenecks

#### 1. The 480px Viewport Cap & Nested Scroll Trap
- **Location:** `web/src/components.tsx:62-66`, `web/src/App.css:688-696`.
- **Code:**
  ```ts
  const FILE_TREE_ROW_HEIGHT = 36;
  const FILE_TREE_MAX_HEIGHT = 480;
  ```
- **Finding:** Capping the file tree at 480px permits only ~13 visible rows simultaneously.
- **Usability Defect:** When exploring a layer like Layer 3 (646 files), the user is confined to a tiny peephole inside an already massive web page. Scrolling through files triggers scroll-chaining bugs where outer page scrolling takes over when the inner container boundary is hit.

#### 2. Wasteful Left-Hand Columns: Mode (88px) & UID:GID (64px) Consuming ~25% Width
- **Location:** `web/src/App.css:669-686`, `723-746`.
- **Code:**
  ```css
  .fileTreeHeaderOnly li span:nth-child(1), .virtualFileTreeRow span:nth-child(1) {
    width: 88px; flex-shrink: 0; /* Permissions (Mode) */
  }
  .fileTreeHeaderOnly li span:nth-child(2), .virtualFileTreeRow span:nth-child(2) {
    width: 64px; flex-shrink: 0; text-align: right; /* UID:GID */
  }
  .fileTreeHeaderOnly li span:nth-child(3), .virtualFileTreeRow span:nth-child(3) {
    width: 72px; flex-shrink: 0; text-align: right; margin-right: 12px; /* Size */
  }
  .fileTreeHeaderOnly li span:nth-child(4), .virtualFileTreeRow span:nth-child(4) {
    flex: 1; min-width: 0; /* File Path */
  }
  ```
- **Finding:**
  1. *Mode Void:* For directories, permissions are blank (`""`), leaving an 88px empty gap on every directory row.
  2. *Low-Entropy UID:GID:* In 99% of container images, `UID:GID` is `0:0` for all files. Dedicating 64px of high-priority left-hand real estate to repetitive `0:0` text wastes horizontal space.
  3. *Path Truncation:* After reserving 224px for Mode, UID:GID, and Size, nested paths at depth 4+ (`usr/local/lib/redis/modules/rejson.so`) suffer severe text truncation (`rejson...`), forcing users to hover to read file names.

#### 3. Lack of Tree Guide Lines and Folder Compaction
- **Location:** `web/src/components.tsx:237`.
- **Finding:** Indentation is rendered via plain CSS:
  ```tsx
  <span className={opClass} style={{ paddingLeft: row.depth * 16 }}>
  ```
- **Usability Defect:**
  1. *No Visual Connection:* When scrolling through 50 files inside `/usr/local/lib`, the parent directory scrolls off-screen. Without vertical tree connection lines (`│`, `├──`, `└──`), users cannot identify folder boundaries.
  2. *No Directory Compaction:* Single-child directory hierarchies require tedious individual clicks. For example:
     `usr` -> expand -> `local` -> expand -> `bin` -> expand
     In modern developer interfaces (GitHub, VS Code), this is automatically compacted into a single clickable row: `usr/local/bin/`.

#### 4. The Broken Substring Path Search in `isMatchKeyword`
- **Location:** `web/src/analysis.ts:168-179`.
- **Code:**
  ```ts
  const isMatchKeyword = (item: FileTreeList, keyword: string): boolean => {
    if (item.name.includes(keyword)) {
      return true;
    }
    for (let i = 0; i < item.children.length; i++) {
      if (isMatchKeyword(item.children[i], keyword)) {
        return true;
      }
    }
    return false;
  };
  ```
- **Finding:** `item.name` stores only the local node basename (e.g. `"redis-server"` or `"bin"`). The full path is stored in `item.key` (`"usr/local/bin/redis-server"`).
- **Usability Defect:** Searching for path segments with slashes (e.g., `usr/local`, `etc/apk`, `/bin/`) **yields zero results**. The search input fails on natural path queries.

#### 5. Defective Expand/Collapse Override Logic
- **Location:** `web/src/analysis.ts:190-192`.
- **Code:**
  ```ts
  const expandAll = !!(opt.expandAll || opt.keyword);
  const shouldExpand = (key: string) =>
    expandAll || !!(opt.expandItems && opt.expandItems.includes(key));
  ```
- **Finding:** When the user checks the "Expand" checkbox (`opt.expandAll`), `expandAll` evaluates to `true`.
- **Usability Defect:** Clicking the collapse icon `[-]` on any individual directory calls `onToggleExpand`, removing the key from `opt.expandItems`. However, `shouldExpand(row.key)` still returns `true` because `expandAll` overrides it. **The user cannot collapse any folder while the Expand checkbox is active.**

---

### 2.5 File Change Status & Size Contrast

#### 1. The Missing "Added" Status (`Op::Added` Rendered as `Op::None`)
- **Location:** `src/image/oci_image.rs:233-241`, `web/src/analysis.ts:13-15`, `web/src/components.tsx:210-215`.
- **Code in `analysis.ts`:**
  ```ts
  export const opRemoved = 1;
  export const opModified = 2;
  // opAdded is completely absent!
  ```
- **Code in `oci_image.rs`:**
  ```rust
  pub enum Op { None, Removed, Modified, Added }
  // ...
  let mut op = Op::None;
  if file.is_whiteout.is_some() {
      op = Op::Removed;
  } else if modified_paths.contains(file.path.as_str()) {
      op = Op::Modified;
  }
  ```
- **Finding:** In the Rust backend, new files introduced in a layer are not added to `modified_paths`, leaving them as `Op::None` (`0`). In the TypeScript frontend, `opAdded` does not exist.
- **Usability Defect:** Newly added files receive **no CSS class and no visual indicator whatsoever**. In Layer 7, the 21 brand new binaries (including `redis-server`, `redis-cli`, `rejson.so`) look identical to unchanged baseline files.

#### 2. Color-Only Status Indication & WCAG Non-Compliance
- **Location:** `web/src/App.css:756-761`.
- **Code:**
  ```css
  .fileTree .modified { color: var(--warn); }
  .fileTree .removed { color: var(--danger); }
  ```
- **Finding:** Modified and removed items are styled purely through text color (`#d97706` amber and `#dc2626` red).
- **Usability Defect:**
  1. *Accessibility Failure:* This violates WCAG 2.1 Success Criterion 1.4.1 (Use of Color). Color-blind developers (deuteranopia/protanopia) cannot distinguish modified files from base files.
  2. *No Textual Badges:* There are no structured badges (`[+ ADD]`, `[~ MOD]`, `[- DEL]`).
  3. *Uninformative Whiteouts:* Deleted files (whiteout markers) have no strikethrough styling and fail to display their previous size or original origin layer.

#### 3. Zero Size Contrast (45.1 MB Binary vs 38 B Config File)
- **Location:** `web/src/components.tsx:236`, `web/src/App.css:738-745`.
- **Code:**
  ```tsx
  <span>{prettyBytes(row.size)}</span>
  ```
- **Finding:** In Layer 7, `rejson.so` is **45.1 MB** (45,058,232 bytes), while `shells` is **38 B** (38 bytes) and symlinks are **0 B**.
- **Usability Defect:** Both are displayed in identical 12.5px monospace font with 400 weight and muted gray color. The user cannot spot huge bloat files without manually reading every single digit.

---

## 3. Structural Proposals & ASCII Wireframes

### 3.1 Full-Page Macro Architecture: Single Column Stack vs. 2-Pane Workspace

#### CURRENT ARCHITECTURE: Single-Column Linear Stack (>3,200px Scroll Depth)
```
+----------------------------------------------------------------------------------------------------+
| HEADER: Logo | Fixed-width Arch [AMD64] | Search Input: "redis:alpine" | MCP Button | GitHub Ribbon|
+----------------------------------------------------------------------------------------------------+
| CONTENT WRAPPER (max-width: 1120px, ~50% screen wasted on desktop)                                 |
|                                                                                                    |
|  +----------------------------------------------------------------------------------------------+  |
|  | IMAGE SUMMARY CARD: Efficiency Score Gauge (99%) | Size Metrics | Base OS Tags              |  |
|  +----------------------------------------------------------------------------------------------+  |
|                                                                                                    |
|  +----------------------------------------------------------------------------------------------+  |
|  | WHERE THE BYTES ARE (Treemap Canvas: 360px height)                                          |  |
|  +----------------------------------------------------------------------------------------------+  |
|                                                                                                    |
|  +----------------------------------------------------------------------------------------------+  |
|  | LAYER CONTENT (Primary Analysis Silo):                                                       |  |
|  |   [Layer Dropdown: "8: AB7525B6 (35.5 MB)"]  [Size Filter]  [ ] Mod  [ ] Expand  [Keywords]  |  |
|  |   Created: 11/14/2026, 9:30:15 AM                                                            |  |
|  |   Command: RUN |2 REDIS_DOWNLOAD_URL=... (2,528-char unformatted bash wall)                  |  |
|  |   +----------------------------------------------------------------------------------------+ |  |
|  |   | MODE         UID:GID     SIZE      FILETREE                                             | |  |
|  |   | -rwxr-xr-x    0:0       45.1 MB    v usr                                                | |  |
|  |   |                           0 B        v local                                            | |  |
|  |   |                                        v lib (trapped in 480px scroller)                | |  |
|  |   +----------------------------------------------------------------------------------------+ |  |
|  +----------------------------------------------------------------------------------------------+  |
|                                                                                                    |
|  +----------------------------------------------------------------------------------------------+  |
|  | FINDINGS GRID (2-Column Grid: Wasted, Sensitive, Duplicates, Recommendations, Dockerfile)    |  |
|  +----------------------------------------------------------------------------------------------+  |
+----------------------------------------------------------------------------------------------------+
```

#### PROPOSED ARCHITECTURE: Modern 2-Pane Master-Detail Workspace
```
+-----------------------------------------------------------------------------------------------------------------------+
| COMPACT APP BAR (48px): [Logo] Diving · redis:alpine [amd64 v] | [Q Quick Search (/)] | [Compare] [Theme] [GitHub]    |
+-----------------------------------------------------------------------------------------------------------------------+
| KPI STRIP (40px): 99% Efficiency | 120.4 MB Unpacked | 39.5 MB Compressed | 0 B Wasted | Alpine Linux 3.21            |
+----------------------------------------------------+------------------------------------------------------------------+
| MASTER: LAYER TIMELINE (35% Width)                 | DETAIL: INSPECTION WORKSPACE (65% Width)                         |
| Filter: [ All (14) ] [ Files Only (7) ] [ >1MB (2) ]| Tabs: [ File Tree (128) ] [ Treemap ] [ Findings (7) ] [ Docker ]|
|----------------------------------------------------+------------------------------------------------------------------|
| #   TAG    SIZE       PROPORTION    INSTRUCTION    | LAYER 8 DETAILS & COMMAND INSPECTOR                              |
|----------------------------------------------------+ Compressed: 35.5 MB  Unpacked: 110.7 MB  Ratio: 3.12x  Files: 128 |
| 01  FROM   3.85 MB    [===       ]  alpine:3.21    | sha256:ab7525b6c4... [Copy]  Created: 2026-11-05 19:17           |
| 02  CMD    0 B        [          ]  /bin/sh        |------------------------------------------------------------------|
| 03  RUN    949 B      [          ]  addgroup redis | [v] Command: RUN redis download & build    [Wrap] [Format] [Copy]|
| 04  RUN    201 kB     [          ]  apk add tzdata | +--------------------------------------------------------------+ |
| 05  ENV    0 B        [          ]  REDIS_VERSION  | | 1  set -eux;                                                 | |
| 06  ARG    0 B        [          ]  DOWNLOAD_URL   | | 2  apk add --no-cache --virtual .build-deps gcc make ...     | |
| 07  ARG    0 B        [          ]  DOWNLOAD_SHA   | | 3  wget -O redis.tar.gz "$REDIS_DOWNLOAD_URL";               | |
| 08> RUN    35.5 MB    [==========]  redis build... | | 4  make -C /usr/src/redis -j"$(nproc)" all install;          | |
| 09  RUN    101 B      [          ]  mkdir /data    | +--------------------------------------------------------------+ |
| 10  DIR    32 B       [          ]  WORKDIR /data  |------------------------------------------------------------------|
| 11  COPY   2.1 kB     [          ]  entrypoint.sh  | FILE TREE EXPLORER (Layer 8 Modifications)                       |
| 12  ENTRY  0 B        [          ]  docker-entry...| [ Search path (e.g. usr/bin, *.so) ] [Filter: Changes v] [Reset] |
| 13  EXPOSE 0 B        [          ]  6379/tcp       |------------------------------------------------------------------|
| 14  CMD    0 B        [          ]  redis-server   | STATUS   SIZE        RELATIVE    FILE PATH               ACTIONS |
|                                                    | [+ ADD]  45.1 MB   [██████████]  usr/local/lib/rejson.so [Prev]  |
|                                                    | [+ ADD]   7.2 MB   [██░░░░░░░░]  usr/local/bin/redis-cli [Prev]  |
|                                                    | [~ MOD]   38 B     [          ]  etc/shells                      |
+----------------------------------------------------+------------------------------------------------------------------+
| STATUS BAR (24px): [↑/↓] Navigate Layers  [Tab] Switch Pane  [/] Filter Files  [Esc] Clear Filters  [?] Shortcut Help |
+-----------------------------------------------------------------------------------------------------------------------+
```

---

### 3.2 Component Wireframe: Streamlined Application Header & Omnibar

```
+---------------------------------------------------------------------------------------------------------------+
| [Diving Logo]  |  redis:alpine [amd64 v]  |  [ Q Search image or press / ...         ]  | [Diff] [MCP] [Theme] [GH]|
+---------------------------------------------------------------------------------------------------------------+
```

#### Detailed Element Specifications:
1. **Brand & Version (Left):**
   - Compact vector logo with version chip (`v0.12.0`). Height: 32px.
2. **Context Breadcrumb (Persistent):**
   - Active image repository (`redis:alpine`) in bold font weight (`600`).
   - Clickable Architecture Dropdown Pill (`[amd64 v]`): Clicking switches architecture without navigating back or retyping.
3. **Refined Omnibar (Center, Flexible Width 380px–600px):**
   - Minimalist input field with search icon (`Q`) and keyboard shortcut badge (`/`).
   - Integrated recent search history dropdown.
4. **Utility Actions (Right):**
   - `[Diff Mode]`: Direct modal/page trigger for image comparison.
   - `[MCP Server]`: Dialog with connection strings and configuration options.
   - `[Theme Toggle]`: 3-state icon button (`Sun` = Light, `Moon` = Dark, `Monitor` = System).
   - `[GitHub Icon]`: Clean, non-overlapping SVG icon link in standard flex alignment.

---

### 3.3 Component Wireframe: Persistent Master Layer Timeline

```
+------------------------------------------------------------------------------+
| LAYERS (14 total · 39.5 MB compressed · 120.4 MB uncompressed)               |
| Filter: [x] All (14)   [ ] Has Files (7)   [ ] Large >1MB (2)                |
+----+--------+-----------+--------------------+-------------------------------+
| #  | OP     | SIZE      | PROPORTIONAL WEIGHT| INSTRUCTION SUMMARY           |
+----+--------+-----------+--------------------+-------------------------------+
| 01 | FROM   |   3.85 MB | [██░░░░░░░░] 10.8% | alpine:3.21                   |
| 02 | CMD    |       0 B | [          ]  0.0% | /bin/sh                       |
| 03 | RUN    |     949 B | [          ]  0.0% | addgroup -S -g 1000 redis     |
| 04 | RUN    |    201 kB | [          ]  0.5% | apk add --no-cache tzdata...  |
| 05 | ENV    |       0 B | [          ]  0.0% | REDIS_VERSION=8.10.3          |
| 06 | ARG    |       0 B | [          ]  0.0% | REDIS_DOWNLOAD_URL            |
| 07 | ARG    |       0 B | [          ]  0.0% | REDIS_DOWNLOAD_SHA            |
| 08>| RUN    |  35.47 MB | [██████████] 89.7% | make -C /usr/src/redis all    |
| 09 | RUN    |     101 B | [          ]  0.0% | mkdir /data && chown redis    |
| 10 | DIR    |      32 B | [          ]  0.0% | WORKDIR /data                 |
| 11 | COPY   |   2.10 kB | [          ]  0.0% | docker-entrypoint.sh /usr/... |
| 12 | ENTRY  |       0 B | [          ]  0.0% | ["docker-entrypoint.sh"]      |
| 13 | EXPOSE |       0 B | [          ]  0.0% | 6379/tcp                      |
| 14 | CMD    |       0 B | [          ]  0.0% | ["redis-server"]              |
+----+--------+-----------+--------------------+-------------------------------+
```

#### Layer Card Ergonomics:
- **Active State (`>08`):** High-contrast left accent border (`3px solid var(--accent-base)`), elevated background surface (`var(--surface-sunken)`), and bold index counter.
- **Opcode Pills:** Standardized semantic chips:
  - `FROM`: Sky Blue (`#0ea5e9`).
  - `RUN`: Amber (`#f59e0b`).
  - `COPY` / `ADD`: Emerald Green (`#10b981`).
  - `ENV` / `ARG` / `WORKDIR`: Muted Slate (`#64748b`).
  - `ENTRYPOINT` / `CMD`: Violet (`#8b5cf6`).
- **Proportional Size Bar:** Horizontal SVG or CSS flex bar illustrating `layer.size / total_image_size`. Bloat layers immediately draw the eye.
- **Empty State Indicator:** Renders `0 B` or `[meta]` instead of `"NONE"`.

---

### 3.4 Component Wireframe: Beautified Command Inspector & Metadata Strip

```
+------------------------------------------------------------------------------------------------------+
| SELECTED: Layer #8 — RUN (make -C /usr/src/redis all)                                                |
+------------------------------------------------------------------------------------------------------+
| METRICS STRIP:                                                                                       |
| [ 35.5 MB Compressed ]  [ 110.7 MB Unpacked ]  [ 3.12x Expansion ]  [ 128 Files (21 New, 107 Mod) ]  |
| Digest: sha256:ab7525b6c48d9c57803f27118... [Copy]  ·  Created: 2026-11-05 19:17:30 UTC            |
+------------------------------------------------------------------------------------------------------+
| BUILD INSTRUCTION & SHELL SCRIPT:                           [Wrap Lines] [Format] [Copy Shell Box]  |
| +--------------------------------------------------------------------------------------------------+ |
| | 1  # BuildKit Arguments: REDIS_DOWNLOAD_URL, REDIS_DOWNLOAD_SHA                                  | |
| | 2  set -eux;                                                                                     | |
| | 3  apk add --no-cache --virtual .build-deps \                                                    | |
| | 4      coreutils dpkg-dev dpkg gcc linux-headers make musl-dev openssl-dev zstd-dev g++;         | |
| | 5  wget -O redis.tar.gz "$REDIS_DOWNLOAD_URL";                                                   | |
| | 6  echo "$REDIS_DOWNLOAD_SHA *redis.tar.gz" | sha256sum -c -;                                    | |
| | 7  mkdir -p /usr/src/redis;                                                                      | |
| | 8  tar -xzf redis.tar.gz -C /usr/src/redis --strip-components=1;                                 | |
| | ... [Show 16 more lines ▼]                                                                       | |
| +--------------------------------------------------------------------------------------------------+ |
+------------------------------------------------------------------------------------------------------+
```

#### Key Innovations in Command Inspector:
1. **BuildKit Boilerplate Stripping:** `|2 ARG1=... /bin/sh -c` is parsed into dedicated environment comments at the top of the box.
2. **Auto-Formatting:** Statements chained with `;`, `&&`, and `\` are split into indented, numbered lines.
3. **Collapsible Frame:** Clamped to a default maximum height of 160px (approx 6-8 lines) with an inline accordion toggle (`Show 16 more lines ▼`), preventing the command box from pushing the file tree off-screen.
4. **Copy Action:** 1-click clipboard button with visual feedback tooltip (`Copied!`).

---

### 3.5 Component Wireframe: Modernized File Tree with Status Badges & Micro-Bars

```
+------------------------------------------------------------------------------------------------------+
| FILE TREE EXPLORER (Layer #8 Modifications)                                                          |
| [ Search path or name (e.g. usr/bin, *.so)      x ]  [Show: All Changes (128) v] [Compact Paths: [x]]|
+------------------------------------------------------------------------------------------------------+
| STATUS   SIZE         RELATIVE   FILE TREE HIERARCHY                              ACTIONS            |
|------------------------------------------------------------------------------------------------------|
| [~ MOD]  110.5 MB  [██████████]  ▼ usr/                                                              |
|          106.9 MB  [█████████░]  │   ▼ local/lib/redis/modules/ (compacted 4x)                       |
| [+ ADD]   45.1 MB  [█████░░░░░]  │   │   ├──  rejson.so                      [Preview] [Download]   |
| [+ ADD]   20.5 MB  [██░░░░░░░░]  │   │   ├──  redisearch.so                  [Preview] [Download]   |
| [+ ADD]    3.4 MB  [█░░░░░░░░░]  │   │   ├──  redistimeseries.so             [Preview] [Download]   |
| [+ ADD]  610.6 kB  [░░░░░░░░░░]  │   │   └──  redisbloom.so                  [Preview] [Download]   |
| [~ MOD]   37.3 MB  [████░░░░░░]  │   ▼ local/bin/                                                    |
| [+ ADD]    7.8 MB  [█░░░░░░░░░]  │   │   ├──  redis-cli                      [Preview] [Download]   |
| [+ ADD]    6.7 MB  [█░░░░░░░░░]  │   │   ├──  redis-benchmark                [Preview] [Download]   |
| [~ MOD]        0 B  [          ]  │   │   └──  redis-sentinel → redis-server                         |
| [~ MOD]       38 B  [          ]  ▼ etc/                                                             |
| [~ MOD]       38 B  [          ]      └──  shells                                 [Preview] [Download]   |
| [- DEL]    1.2 kB   [~~~~~~~~~~]      └──  apk/world.old (whiteout)                                  |
+------------------------------------------------------------------------------------------------------+
| Showing 128 matching files (21 added, 107 modified, 1 deleted)  ·  [Expand All]  [Collapse All]     |
+------------------------------------------------------------------------------------------------------+
```

#### File Tree Visual Features:
1. **Explicit Status Badges:**
   - `[+ ADD]`: Emerald pill (`#10b981`). Identifies newly introduced files in this layer.
   - `[~ MOD]`: Amber pill (`#f59e0b`). Identifies modified/overwritten files.
   - `[- DEL]`: Red pill (`#ef4444`). Identifies deleted/whiteout files with strikethrough styling.
2. **Relative Size Micro-Bars:**
   - Visual micro-track (`[█████░░░░░]`) calculated as `file.size / max_layer_file_size`.
   - Giant binaries (`rejson.so`, 45.1 MB) immediately dominate the visual hierarchy.
3. **Directory Path Compaction:**
   - Single-child chains like `local/lib/redis/modules/` are collapsed into a single row, saving 4 redundant rows and expansion clicks.
4. **Tree Connection Guide Lines:**
   - Monospace tree characters (`│`, `├──`, `└──`) provide visual hierarchy without requiring horizontal scrolling.
5. **Fluid Dynamic Height:**
   - Instead of hardcoding `max-height: 480px`, the container uses `height: calc(100vh - 360px)` (min 560px), eliminating the nested scroll trap on desktop screens.

---

### 3.6 Responsive Behavior & Adaptive Breakpoints

The workspace layout adapts smoothly across three standard responsive tiers:

```
+-----------------------------------+-----------------------------------+
| Large Desktop (>= 1200px)         | 2-Pane Side-by-Side (35% / 65%)   |
+-----------------------------------+-----------------------------------+
| Tablet / Small Laptop (768–1199px)| 2-Pane Compact (30% / 70%) or     |
|                                   | Collapsible Left Drawer           |
+-----------------------------------+-----------------------------------+
| Mobile (< 768px)                  | Segmented Control [Layers | Files]|
|                                   | Single Full-Width Card Stack      |
+-----------------------------------+-----------------------------------+
```

- **Mobile Viewport (< 768px):** A clean Ant Design `Segmented` control toggles between the **Layer Timeline** and the **File Inspector**, preserving full mobile touch usability without horizontal truncation.

---

## 4. Proposed Design System & Style Guide

### 4.1 Semantic Color Token Architecture

The design system eliminates hardcoded hex values in favor of a 2-tier CSS variable architecture (`tokens.css`), supporting seamless transitions between light, dark, and high-contrast modes.

```css
/* ==========================================================================
   Diving Modern Design System - Semantic Tokens (tokens.css)
   ========================================================================== */

:root {
  /* Surfaces & Elevation */
  --surface-canvas: #f8fafc;        /* Slate-50: Crisp, clean application background */
  --surface-raised: #ffffff;        /* Pure white: Elevated cards & timeline panels */
  --surface-sunken: #f1f5f9;        /* Slate-100: Table headers, code blocks, inputs */
  --surface-overlay: #ffffff;       /* Modals, popovers, dropdown menus */
  --surface-hover: #e2e8f0;         /* Slate-200: Interactive list item hover */
  --surface-active: #e0f2fe;        /* Sky-100: Active layer highlight */

  /* Hairline Borders */
  --border-subtle: rgba(15, 23, 42, 0.06);   /* Structural panel dividers */
  --border-default: rgba(15, 23, 42, 0.12);  /* Form inputs, cards */
  --border-strong: rgba(15, 23, 42, 0.22);   /* Focus states, active pills */

  /* Typography Inks */
  --text-primary: #0f172a;          /* Slate-900: High-legibility headers & file names */
  --text-secondary: #475569;        /* Slate-600: Descriptions, metadata, paths */
  --text-tertiary: #94a3b8;         /* Slate-400: Placeholders, timestamps, disabled */
  --text-inverse: #ffffff;

  /* Accent & Focus */
  --accent-base: #0ea5e9;           /* Sky-500: Developer tool cyan/blue accent */
  --accent-hover: #0284c7;          /* Sky-600 */
  --accent-subtle: rgba(14, 165, 233, 0.08);

  /* Semantic Status Indicators */
  --status-added: #059669;          /* Emerald-600: Added files */
  --status-added-bg: rgba(5, 150, 105, 0.10);
  --status-modified: #d97706;       /* Amber-600: Modified files */
  --status-modified-bg: rgba(217, 119, 6, 0.10);
  --status-removed: #dc2626;        /* Red-600: Whiteouts / Deleted files */
  --status-removed-bg: rgba(220, 38, 38, 0.10);
  --status-info: #6366f1;           /* Indigo-500: Informational chips */
  --status-info-bg: rgba(99, 102, 241, 0.10);

  /* Visual Size Heat Scale */
  --size-bar-track: rgba(15, 23, 42, 0.06);
  --size-bar-fill: #0ea5e9;
  --size-bar-heavy: #f59e0b;        /* Files > 10 MB */
  --size-bar-extreme: #ef4444;      /* Files > 50 MB */
}

/* Dark Mode Tokens */
html.dark {
  --surface-canvas: #090d12;        /* Deep Obsidian neutral background */
  --surface-raised: #111720;        /* Elevated dark panels */
  --surface-sunken: #18202b;        /* Table heads, code containers */
  --surface-overlay: #1e2735;
  --surface-hover: #222d3d;
  --surface-active: #0c2d48;

  --border-subtle: rgba(255, 255, 255, 0.06);
  --border-default: rgba(255, 255, 255, 0.12);
  --border-strong: rgba(255, 255, 255, 0.24);

  --text-primary: #f1f5f9;          /* Slate-100 */
  --text-secondary: #94a3b8;        /* Slate-400 */
  --text-tertiary: #64748b;         /* Slate-500 */
  --text-inverse: #090d12;

  --accent-base: #38bdf8;           /* Sky-400 */
  --accent-hover: #7dd3fc;
  --accent-subtle: rgba(56, 189, 248, 0.12);

  --status-added: #34d399;          /* Emerald-400 */
  --status-added-bg: rgba(52, 211, 153, 0.14);
  --status-modified: #fbbf24;       /* Amber-400 */
  --status-modified-bg: rgba(251, 191, 36, 0.14);
  --status-removed: #f87171;        /* Red-400 */
  --status-removed-bg: rgba(248, 113, 113, 0.14);

  --size-bar-track: rgba(255, 255, 255, 0.08);
  --size-bar-fill: #38bdf8;
  --size-bar-heavy: #fbbf24;
  --size-bar-extreme: #f87171;
}
```

---

### 4.2 Disciplined Typographic Scale (Eliminating Fractional Pixels)

The current codebase contains 11 distinct font sizes, including fractional sizes like `12.5px` (`components.tsx:236`, `App.css:738`) and `13.5px` (`components.tsx:556`), causing subpixel rendering blur on standard 1× DPI displays. 

The proposed scale standardizes all typography onto an integer 4px grid:

| Token Name | Font Family | Size | Line Height | Weight | Application Area |
|---|---|---|---|---|---|
| `--font-mono-xs` | `IBM Plex Mono` | **11px** | 14px | 500 | Mode flags, UID:GID tags, opcode chips |
| `--font-mono-sm` | `IBM Plex Mono` | **12px** | 16px | 400 / 500 | File tree rows, sizes, byte counts, digests |
| `--font-mono-base` | `IBM Plex Mono` | **13px** | 20px | 400 | Command inspector, shell scripts, Dockerfile |
| `--font-sans-xs` | `IBM Plex Sans` | **11px** | 14px | 600 (Caps) | Table headers, column labels, status badges |
| `--font-sans-sm` | `IBM Plex Sans` | **12px** | 16px | 400 / 500 | Metadata descriptions, hints, tooltips |
| `--font-sans-base` | `IBM Plex Sans` | **14px** | 20px | 400 / 500 | General body text, form inputs, button labels |
| `--font-sans-md` | `IBM Plex Sans` | **16px** | 24px | 600 | Card titles, panel headings, tab labels |
| `--font-sans-lg` | `IBM Plex Sans` | **20px** | 28px | 600 | Modal titles, section hero text |
| `--font-metric-xl`| `IBM Plex Sans` | **28px** | 32px | 700 | Summary KPI score, total image size numbers |

---

### 4.3 Surface Elevation & Hairline Borders (Eliminating the "Border Onion")

In the current web implementation, viewing a single file requires looking through up to **5 nested borders**:
1. Page `.contentWrapper` card border.
2. `.panel` card container border (`border: 1px solid var(--border)`).
3. Inner `.layerToolbar` form box border.
4. `.fileTreeHeaderOnly` table header border.
5. `.virtualFileTreeScroller` inner scroll border.

To counter Ant Design's nested card padding, the existing code resorts to negative margin hacks (`style={{ margin: "-16px -18px" }}`).

#### Proposed Structural Simplification:
- **Flat Elevation Model:** Eliminate nested card containers. The workspace is divided into two adjacent panels using clean `1px solid var(--border-subtle)` hairline dividers.
- **Negative Margin Purge:** Remove all `-16px -18px` margin overrides in `components.tsx`. Tables cleanly align with panel edges naturally.
- **Shadow Elimination:** Drop heavy blurred box shadows (`box-shadow: var(--shadow)`) on static content containers. Reserve shadows strictly for elevated floating overlays (modals, dropdown menus).

---

### 4.4 Status Badges, Relative Micro-Bars & Accessibility (WCAG Compliance)

#### 1. Accessible Change Status Badges
To comply with WCAG 2.1 SC 1.4.1 (Color Independence) and SC 1.4.3 (Contrast Minimum ≥ 4.5:1), all file change indicators must combine **iconography, textual abbreviations, and background tinting**:

```css
/* Accessible Status Badges */
.statusBadge {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 4px;
  line-height: 14px;
  letter-spacing: 0.02em;
}

.statusBadge.add {
  color: var(--status-added);
  background: var(--status-added-bg);
  border: 1px solid color-mix(in srgb, var(--status-added) 30%, transparent);
}

.statusBadge.mod {
  color: var(--status-modified);
  background: var(--status-modified-bg);
  border: 1px solid color-mix(in srgb, var(--status-modified) 30%, transparent);
}

.statusBadge.del {
  color: var(--status-removed);
  background: var(--status-removed-bg);
  border: 1px solid color-mix(in srgb, var(--status-removed) 30%, transparent);
}
```

#### 2. Relative Size Micro-Bars
Every file row renders a subtle 48px micro-bar track immediately following the byte count:

```tsx
// Micro-Bar Component
const SizeMicroBar: React.FC<{ size: number; maxLayerSize: number }> = ({ size, maxLayerSize }) => {
  const percent = maxLayerSize > 0 ? Math.min(100, Math.round((size / maxLayerSize) * 100)) : 0;
  const isExtreme = size > 50 * 1024 * 1024; // > 50 MB
  const isHeavy = size > 5 * 1024 * 1024;    // > 5 MB

  return (
    <div className="sizeBarTrack" title={`${prettyBytes(size)} (${percent}% of largest file)`}>
      <div 
        className={`sizeBarFill ${isExtreme ? "extreme" : isHeavy ? "heavy" : ""}`} 
        style={{ width: `${percent}%` }} 
      />
    </div>
  );
};
```

---

## 5. Prioritized Implementation Roadmap & Technical Guide

### 5.1 Action Priority Matrix (P0 / P1 / P2)

| Priority | Feature / Fix | Target Module | Visual & UX Impact | Feasibility | Effort |
|---|---|---|:---:|:---:|:---:|
| **P0** | **Fix Broken Path Search (`isMatchKeyword`)** | `web/src/analysis.ts` | Critical (Bug fix) | High | 1 hr |
| **P0** | **Fix Expand/Collapse Override Defect** | `web/src/analysis.ts` | High (Bug fix) | High | 1 hr |
| **P0** | **Add `Op::Added` & Accessible Status Badges** | `web/src/analysis.ts`, `components.tsx`, `oci_image.rs` | Critical | High | 3 hrs |
| **P0** | **Fix Dropdown "NONE" Labels & 8-Hex Hashes** | `web/src/App.tsx` | Critical | High | 2 hrs |
| **P1** | **Master-Detail 2-Pane Workspace Layout** | `web/src/App.tsx`, `App.css` | Transformational | High | 6 hrs |
| **P1** | **Persistent Interactive Layer Timeline** | `web/src/components.tsx`, `App.tsx` | Transformational | High | 5 hrs |
| **P1** | **Fluid File Tree Height & Eliminate 480px Trap** | `web/src/components.tsx`, `App.css` | High | High | 2 hrs |
| **P1** | **Relative Size Micro-Bars & Contrast Weight** | `web/src/components.tsx`, `App.css` | High | High | 2 hrs |
| **P1** | **Column Streamlining (Hide blank mode & 0:0 UID)**| `web/src/components.tsx`, `App.css` | High | High | 2 hrs |
| **P1** | **Widescreen Container (`--content-max` relaxation)** | `web/src/App.css`, `index.css` | High | High | 1 hr |
| **P2** | **Beautified Command Inspector with BuildKit Filter** | `web/src/components.tsx` | High | High | 4 hrs |
| **P2** | **Directory Path Compaction (`usr/local/bin`)** | `web/src/analysis.ts`, `components.tsx` | Medium | Medium | 4 hrs |
| **P2** | **Tree Connection Guide Lines (`│`, `├──`, `└──`)** | `web/src/components.tsx`, `App.css` | Medium | Medium | 3 hrs |
| **P2** | **Manual Theme Toggle (System / Dark / Light)** | `web/src/App.tsx`, `App.css` | Medium | High | 2 hrs |
| **P2** | **Eliminate GitHub Corner & Clean Header Actions** | `web/src/icons.tsx`, `App.tsx` | Medium | High | 2 hrs |
| **P2** | **Keyboard Navigation Shortcuts (`↑/↓`, `/`, `Tab`)** | `web/src/App.tsx` | Medium | High | 3 hrs |

---

### 5.2 Phase 1: High-Impact Usability & Status Bug Fixes (P0)

#### 1. Fix Path Search Bug in `web/src/analysis.ts`
Modify `isMatchKeyword` to test the full path key (`item.key`) rather than only the local node name (`item.name`):

```ts
// In web/src/analysis.ts:168-179
const isMatchKeyword = (item: FileTreeList, keyword: string): boolean => {
  const normalizedKeyword = keyword.trim().toLowerCase();
  if (!normalizedKeyword) return true;
  
  // Test both full path (key) and base segment (name)
  const keyMatch = item.key.toLowerCase().includes(normalizedKeyword);
  const nameMatch = item.name.toLowerCase().includes(normalizedKeyword);
  
  if (keyMatch || nameMatch) {
    return true;
  }
  for (let i = 0; i < item.children.length; i++) {
    if (isMatchKeyword(item.children[i], keyword)) {
      return true;
    }
  }
  return false;
};
```

#### 2. Fix Expand/Collapse Override Defect in `web/src/analysis.ts`
Decouple individual folder expansion tracking from the global `expandAll` flag so manual collapses are honored:

```ts
// In web/src/analysis.ts:188-195
const collapsedItems = opt.collapsedItems || [];
const shouldExpand = (key: string) => {
  if (collapsedItems.includes(key)) return false;
  if (opt.keyword) return true;
  if (opt.expandAll) return true;
  return !!(opt.expandItems && opt.expandItems.includes(key));
};
```

#### 3. Implement `Op::Added` and Status Badges
1. In `src/image/oci_image.rs:393-398`, when an analyzed file is not in `modified_paths` and layer index > 0, set `op = Op::Added`.
2. In `web/src/analysis.ts`, export `opAdded = 3`.
3. In `web/src/components.tsx`, render `<StatusBadge op={row.op} />` with `[+ ADD]`, `[~ MOD]`, and `[- DEL]`.

---

### 5.3 Phase 2: Core 2-Pane Master-Detail Workspace Architecture (P1)

#### 1. Layout Transformation in `web/src/App.tsx`
Replace the single-column linear stack with a Master-Detail grid:

```tsx
<div className="workspaceGrid">
  <aside className="workspaceMaster">
    <LayerTimeline
      layers={report.layers}
      currentLayer={currentLayer}
      onSelectLayer={setCurrentLayer}
    />
  </aside>
  <main className="workspaceDetail">
    <Tabs
      defaultActiveKey="files"
      items={[
        {
          key: "files",
          label: `File Tree (${activeLayerFileCount})`,
          children: <LayerDetailAndFileTree layer={report.layers[currentLayer]} ... />,
        },
        {
          key: "treemap",
          label: "Treemap View",
          children: <ImageTreemap fileSummaryList={report.fileSummaryList} />,
        },
        {
          key: "findings",
          label: `Findings & Security (${findingsCount})`,
          children: <FindingsWorkspace report={report} />,
        },
        {
          key: "dockerfile",
          label: "Reconstructed Dockerfile",
          children: <DockerfileCard commands={report.dockerfile} />,
        },
      ]}
    />
  </main>
</div>
```

#### 2. CSS Grid Definition in `web/src/App.css`
```css
.workspaceGrid {
  display: grid;
  grid-template-columns: 360px 1fr;
  gap: 16px;
  align-items: start;
  min-height: calc(100vh - 120px);
}

@media (max-width: 1024px) {
  .workspaceGrid {
    grid-template-columns: 1fr;
  }
}
```

---

### 5.4 Phase 3: File Tree Modernization & Design Tokens (P1)

#### 1. Fluid File Tree Height
In `web/src/App.css`, remove `max-height: 480px`:
```css
.virtualFileTreeScroller {
  overflow: auto;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--surface-raised);
  height: calc(100vh - 340px);
  min-height: 520px;
}
```

#### 2. Column Optimization
Hide Mode and UID:GID by default on desktop, allocating 85%+ of row width to the hierarchical file path and micro-bar. Add a toggle in the tree toolbar (`[Columns v]`) for power users who require permission diagnostics.

---

### 5.5 Phase 4: Ergonomics, Command Beautifier & Theming (P2)

#### 1. Command Beautifier & BuildKit Cleaner
Create a formatting utility `formatDockerCommand(rawCmd: string)`:
- Strips BuildKit header prefixes (`|2 ARG=... /bin/sh -c`).
- Replaces `;\t` and `&&` with newline delimiters and 4-space indentation.
- Renders an inline syntax-styled code container with line numbers and a 1-click clipboard button.

#### 2. Manual Theme Switcher in Sticky Header
Add a theme state hook persisted in `localStorage` (`"system" | "dark" | "light"`) and bind an icon button in the header to toggle the `dark` class on `document.documentElement`.

---

### 5.6 File-by-File Engineering Guide

| Source File | Key Changes Required |
|---|---|
| `web/src/analysis.ts` | 1. Fix `isMatchKeyword` to test `item.key.includes(keyword)`.<br>2. Add `export const opAdded = 3;`.<br>3. Fix `shouldExpand` to respect manual collapse state.<br>4. Implement path compaction helper `compactSingleChildDirs(tree)`. |
| `web/src/components.tsx` | 1. Replace hardcoded `480px` max height with dynamic calculation.<br>2. Add `StatusBadge` component (`[+ ADD]`, `[~ MOD]`, `[- DEL]`).<br>3. Add `SizeMicroBar` proportional track component.<br>4. Purge negative margin overrides (`margin: "-16px -18px"`).<br>5. Implement `CommandInspector` with formatting and copy action. |
| `web/src/App.tsx` | 1. Replace `<Select>` layer dropdown with persistent master `LayerTimeline`.<br>2. Re-architect view into 2-Pane Master-Detail grid with secondary tabs.<br>3. Add theme toggle state (`light`, `dark`, `system`) with `localStorage`.<br>4. Add sticky image context breadcrumb and keyboard listeners (`↑/↓`). |
| `web/src/App.css` | 1. Add `.workspaceGrid`, `.workspaceMaster`, and `.workspaceDetail`.<br>2. Purge nested border onion classes.<br>3. Standardize typography classes to integer pixel scale.<br>4. Remove floating GitHub corner collision hacks. |
| `web/src/index.css` | 1. Relax `--content-max: 1120px` to fluid `min(100% - 48px, 1680px)`.<br>2. Define complete semantic token palette (`--surface-*`, `--border-*`, `--status-*`). |
| `src/image/oci_image.rs` | Ensure backend classifies newly introduced layer files as `Op::Added` instead of falling through to `Op::None`. |

---

## 6. Verification, Smoke Testing & Acceptance Criteria

### 6.1 Automated Verification Commands & Smoke Tests

#### 1. Live API Contract & Layer Extraction Verification:
```bash
curl -sL "https://diving.vicanso.dev/api/analyze?image=redis%3Aalpine%3Farch%3Damd64" | python3 -c '
import sys, json
data = json.load(sys.stdin)
layers = data.get("layers", [])
print(f"Total layers: {len(layers)}")
assert len(layers) == 14, "Expected 14 layers in redis:alpine"
for i, l in enumerate(layers):
    print(f"[{i:02d}] size={l[\"size\"]:10d} | unpack={l[\"unpackSize\"]:10d} | empty={str(l[\"empty\"]):5s} | cmd={l[\"cmd\"][:45]}")
'
```

#### 2. Codebase Typecheck & Production Build:
```bash
cd /Users/tree/github/diving-rs/web
npm run build
```
*Acceptance:* TypeScript compilation and Vite bundling complete with **zero errors**.

#### 3. Rust Backend Test Suite:
```bash
cd /Users/tree/github/diving-rs
cargo test --lib
```
*Acceptance:* All internal layer parsing, diffing, and image analysis unit tests pass cleanly.

---

### 6.2 Manual Acceptance Checklist & Test Scenarios

- [ ] **Master-Detail Workspace:** On viewports ≥ 1024px, the Layer Timeline is permanently visible on the left, while the File Tree and Detail Workspace occupy the right.
- [ ] **No "NONE" Dropdown Artifacts:** Metadata layers display semantic instruction tags (`ENV`, `ARG`, `CMD`, `EXPOSE`) and `0 B` rather than `NONE`.
- [ ] **Accurate Status Indicators:** Newly introduced files display `[+ ADD]` in emerald; modified files display `[~ MOD]` in amber; whiteouts display `[- DEL]` in red with strikethrough.
- [ ] **Visual Size Contrast:** Large binaries (>10 MB) display prominent micro-bars and bold weight, standing out immediately from small configuration files.
- [ ] **Path Search:** Searching `usr/local` or `*.so` immediately filters files across directory levels without failing on slashes.
- [ ] **Command Formatting:** Layer 8's 2,528-character script is parsed into indented lines, clamped to 160px height with an expander and a 1-click copy button.
- [ ] **Clean Theming & Header:** The sticky header provides a persistent image breadcrumb and an accessible Light/Dark/System theme toggle with zero layout collisions.
