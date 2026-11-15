// Pure computation over the analyze result: summary derivation and
// file-tree flattening for the virtualized renderer. No React in here.

import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  FileTreeList,
  FileWastedSummary,
  ImageAnalyzeResult,
  ImageDescriptions,
} from "./types";

export const opRemoved = 1;
export const opModified = 2;
export const opAdded = 3;

export interface FileTreeViewOption {
  expandAll: boolean;
  expandItems: string[];
  collapsedItems?: string[];
  sizeLimit: number;
  onlyModifiedRemoved: boolean;
  keyword: string;
  compactPaths?: boolean;
}

/** Flat row used by the virtualized file-tree renderer. */
export interface FileTreeRow {
  key: string;
  mode: string;
  uid: number;
  gid: number;
  size: number;
  name: string;
  link: string;
  op: number;
  depth: number;
  isDir: boolean;
  expanded: boolean;
}

export const getImageSummary = (result: ImageAnalyzeResult) => {
  let wastedSize = 0;
  // 计算浪费的空间以及文件（按路径聚合；Map 而非逐个 find，`chmod -R`
  // 这类层会有成千上万条）
  const byPath = new Map<string, FileWastedSummary>();
  result.fileSummaryList.forEach((item) => {
    const { size, path } = item.info;
    const change = {
      writtenIn: item.writtenIn,
      layer: item.layerIndex,
      removed: item.op === opRemoved,
    };
    const found = byPath.get(path);
    if (found) {
      found.count++;
      found.totalSize += size;
      found.changes.push(change);
    } else {
      byPath.set(path, {
        path,
        count: 1,
        totalSize: size,
        changes: [change],
      });
    }
    wastedSize += size;
  });
  const wastedList = [...byPath.values()];
  wastedList.sort((a, b) => {
    return b.totalSize - a.totalSize;
  });

  // 除去第一个不为0的layer大小
  let firstNotEmptyLayerSize = 0;
  result.layers.forEach((item) => {
    if (firstNotEmptyLayerSize != 0) {
      return;
    }
    firstNotEmptyLayerSize = item.size;
  });
  const otherLayerSize = result.totalSize - firstNotEmptyLayerSize;

  // Match backend `DockerAnalyzeResult::summary` integer math exactly
  // (including the -1 penalty when any space is wasted).
  let score = "100";
  if (result.totalSize > 0) {
    let s = 100 - Math.floor((wastedSize * 100) / result.totalSize);
    if (wastedSize !== 0) {
      s = Math.max(0, s - 1);
    }
    score = String(s);
  }

  // Runtime libc string: "glibc (needs 2.34) → host 2.36 — ok". Empty when
  // the ELF probe couldn't classify the entrypoint (scratch image, dynamic
  // shell wrapper, unsupported binary). Populated server-side from goblin.
  let runtimeLibc = "";
  let runtimeLibcIssue = "";
  const rc = result.runtimeCompat;
  if (rc && rc.libc) {
    // Lead with the resolved binary path (incl. "(via wrapper)" when we
    // unwrapped a shell entrypoint) so the user sees which file was
    // actually inspected.
    let cell = rc.entrypoint ? `${rc.entrypoint}: ${rc.libc}` : rc.libc;
    if (rc.requiredGlibc) {
      cell += ` (needs ${rc.requiredGlibc})`;
    }
    if (rc.osGlibc) {
      cell += ` → host ${rc.osGlibc}`;
    }
    const statusKey = (
      {
        "": "runtimeLibcOk",
        "glibc-too-old": "runtimeLibcTooOld",
        "glibc-on-musl": "runtimeLibcGlibcOnMusl",
        "musl-on-glibc": "runtimeLibcMuslOnGlibc",
      } as Record<string, string>
    )[rc.issue];
    if (statusKey) {
      cell += ` — ${i18nGet(statusKey)}`;
    }
    runtimeLibc = cell;
    runtimeLibcIssue = rc.issue;
  }

  const imageDescriptions: ImageDescriptions = {
    score: `${score}%`,
    size: `${prettyBytes(result.totalSize)} / ${prettyBytes(result.size)}`,
    otherSize: prettyBytes(otherLayerSize),
    wastedSize: prettyBytes(wastedSize),
    osArch: `${result.os}/${result.arch}`,
    created: result.layers[result.layers.length - 1]?.created || "",
    baseOs: result.baseOs || "",
    runtimeLibc,
    runtimeLibcIssue,
    runUser: result.user || "",
  };
  return {
    wastedList,
    imageDescriptions,
  };
};

export const addKeyToFileTreeItem = (items: FileTreeList[], prefix: string) => {
  items.forEach((item) => {
    let key = item.name;
    if (prefix) {
      key = `${prefix}/${key}`;
    }
    item.key = key;
    addKeyToFileTreeItem(item.children, key);
  });
};

const isModifiedRemoved = (item: FileTreeList): boolean => {
  if (item.op === opRemoved || item.op === opModified) {
    return true;
  }
  // 递归检查子节点：目录节点的 op 标记不可靠（仅当目录由修改文件首次
  // 创建时才为 Modified），必须深入到叶子层判断，否则深层有改动的目录
  // 会被"只看修改/删除"过滤整个隐藏
  for (let i = 0; i < item.children.length; i++) {
    if (isModifiedRemoved(item.children[i])) {
      return true;
    }
  }
  return false;
};

const isMatchKeyword = (item: FileTreeList, keyword: string): boolean => {
  const normalized = keyword.trim().toLowerCase();
  if (!normalized) {
    return true;
  }
  if (
    item.key.toLowerCase().includes(normalized) ||
    item.name.toLowerCase().includes(normalized)
  ) {
    return true;
  }
  // 如果子元素符合，则也符合
  for (let i = 0; i < item.children.length; i++) {
    if (isMatchKeyword(item.children[i], normalized)) {
      return true;
    }
  }
  return false;
};

const matchesFilters = (item: FileTreeList, opt: FileTreeViewOption): boolean => {
  if (opt.sizeLimit && item.size < opt.sizeLimit) {
    return false;
  }
  if (opt.onlyModifiedRemoved && !isModifiedRemoved(item)) {
    return false;
  }
  if (opt.keyword && !isMatchKeyword(item, opt.keyword)) {
    return false;
  }
  return true;
};

export const flattenFileTree = (
  items: FileTreeList[] | undefined,
  depth: number,
  opt: FileTreeViewOption,
  out: FileTreeRow[],
) => {
  if (!items) {
    return;
  }
  const collapsedItems = opt.collapsedItems || [];
  const shouldExpand = (key: string) => {
    if (collapsedItems.includes(key)) return false;
    if (opt.keyword) return true;
    if (opt.expandAll) return true;
    return !!(opt.expandItems && opt.expandItems.includes(key));
  };

  for (const rawItem of items) {
    if (!matchesFilters(rawItem, opt)) {
      continue;
    }
    let item = rawItem;
    let displayName = rawItem.name;
    let isDir = item.children.length > 0;

    // Compact single-child directory chains (e.g. usr/local/lib/redis/modules)
    // when compactPaths is enabled, skipping root "." or symlinked dirs.
    if (opt.compactPaths !== false && isDir && displayName !== ".") {
      while (item.children.length === 1) {
        const onlyChild = item.children[0];
        if (
          onlyChild.children.length === 0 ||
          onlyChild.link ||
          !matchesFilters(onlyChild, opt)
        ) {
          break;
        }
        displayName = `${displayName}/${onlyChild.name}`;
        item = onlyChild;
      }
      isDir = item.children.length > 0;
    }

    const expanded = isDir && shouldExpand(item.key);
    out.push({
      key: item.key,
      mode: item.mode,
      uid: item.uid,
      gid: item.gid,
      size: item.size,
      name: displayName,
      link: item.link,
      op: item.op,
      depth,
      isDir,
      expanded,
    });
    if (isDir && expanded) {
      const beforeChildren = out.length;
      flattenFileTree(item.children, depth + 1, opt, out);
      // Drop empty directories when filtering produced no children
      if (out.length === beforeChildren && !opt.keyword) {
        out.pop();
      }
    }
  }
};

// ---- Treemap: the final filesystem, sized and attributed to layers ----

/** A node of the filesystem a container actually sees. */
export interface FsNode {
  name: string;
  /** Path from the image root, `/`-joined, no leading slash. */
  path: string;
  size: number;
  /** Files only: 0-based index of the layer that wrote this copy. */
  layer?: number;
  children?: Map<string, FsNode>;
}

const newDir = (name: string, path: string): FsNode => ({
  name,
  path,
  size: 0,
  children: new Map(),
});

/**
 * Merge the per-layer trees into the final filesystem: each layer's
 * removals (whiteouts, including opaque directories) apply before its
 * additions, and an upper copy replaces a lower one. Directory sizes are
 * the sum of the files below them.
 */
export const buildFinalTree = (fileTreeList: FileTreeList[][]): FsNode => {
  const root = newDir("/", "");
  fileTreeList.forEach((tree, layer) => {
    const removed: string[][] = [];
    const added: { segments: string[]; size: number }[] = [];
    const walk = (items: FileTreeList[], segments: string[]) => {
      for (const item of items) {
        // `./`-rooted tars put everything under a `.` directory.
        const next =
          segments.length === 0 && item.name === "."
            ? segments
            : [...segments, item.name];
        // A whiteout with children is an opaque directory that this same
        // layer fills again (the marker came first in the tar).
        if (item.op === opRemoved && next.length > 0 && item.name) {
          removed.push(next);
        }
        if (item.children && item.children.length > 0) {
          walk(item.children, next);
        } else if (next.length === 0 || !item.name) {
          // nameless leaf: an opaque whiteout on the root
        } else if (item.op !== opRemoved) {
          added.push({ segments: next, size: item.size });
        }
      }
    };
    walk(tree, []);
    for (const segments of removed) {
      let dir: FsNode | undefined = root;
      for (const name of segments.slice(0, -1)) {
        dir = dir.children?.get(name);
        if (!dir) break;
      }
      dir?.children?.delete(segments[segments.length - 1]);
    }
    for (const { segments, size } of added) {
      let dir = root;
      segments.slice(0, -1).forEach((name, i) => {
        let child = dir.children!.get(name);
        if (!child || !child.children) {
          child = newDir(name, segments.slice(0, i + 1).join("/"));
          dir.children!.set(name, child);
        }
        dir = child;
      });
      const name = segments[segments.length - 1];
      dir.children!.set(name, {
        name,
        path: segments.join("/"),
        size,
        layer,
      });
    }
  });
  const total = (node: FsNode): number => {
    if (!node.children) return node.size;
    let sum = 0;
    node.children.forEach((child) => {
      sum += total(child);
    });
    node.size = sum;
    return sum;
  };
  total(root);
  return root;
};

/** Bytes per layer below `node` (a file counts for the layer that wrote it). */
export const layerBytes = (
  node: FsNode,
  out: Map<number, number> = new Map(),
): Map<number, number> => {
  if (!node.children) {
    if (node.layer !== undefined) {
      out.set(node.layer, (out.get(node.layer) || 0) + node.size);
    }
    return out;
  }
  node.children.forEach((child) => layerBytes(child, out));
  return out;
};

/** The layer that wrote most of `node`'s bytes, and its share (0–1). */
export const dominantLayer = (
  node: FsNode,
): { layer: number; share: number } | null => {
  let best: { layer: number; bytes: number } | null = null;
  layerBytes(node).forEach((bytes, layer) => {
    if (!best || bytes > best.bytes) best = { layer, bytes };
  });
  if (!best || node.size === 0) return null;
  const { layer, bytes } = best;
  return { layer, share: bytes / node.size };
};

export interface TreemapRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap layout (Bruls, Huizing, van Wijk): `values` (sorted
 * largest first) tiled into the w×h box with tiles kept as close to square
 * as possible. Returns one rect per value, same order.
 */
export const squarify = (
  values: number[],
  w: number,
  h: number,
): TreemapRect[] => {
  const total = values.reduce((a, b) => a + b, 0);
  const rects: TreemapRect[] = [];
  if (total <= 0 || w <= 0 || h <= 0)
    return values.map(() => ({ x: 0, y: 0, w: 0, h: 0 }));
  const scale = (w * h) / total;
  const areas = values.map((v) => v * scale);
  let x = 0;
  let y = 0;
  let width = w;
  let height = h;
  let i = 0;
  const worst = (row: number[], side: number) => {
    const sum = row.reduce((a, b) => a + b, 0);
    const max = Math.max(...row);
    const min = Math.min(...row);
    return Math.max(
      (side * side * max) / (sum * sum),
      (sum * sum) / (side * side * min),
    );
  };
  while (i < areas.length) {
    const side = Math.min(width, height);
    const row = [areas[i]];
    let j = i + 1;
    while (
      j < areas.length &&
      worst([...row, areas[j]], side) <= worst(row, side)
    ) {
      row.push(areas[j]);
      j++;
    }
    const sum = row.reduce((a, b) => a + b, 0);
    if (width >= height) {
      // lay the row out as a column on the left
      const colW = sum / height;
      let cy = y;
      for (const area of row) {
        const rh = area / colW;
        rects.push({ x, y: cy, w: colW, h: rh });
        cy += rh;
      }
      x += colW;
      width -= colW;
    } else {
      // lay the row out across the top
      const rowH = sum / width;
      let cx = x;
      for (const area of row) {
        const rw = area / rowH;
        rects.push({ x: cx, y, w: rw, h: rowH });
        cx += rw;
      }
      y += rowH;
      height -= rowH;
    }
    i = j;
  }
  return rects;
};

// ---- Layer command parsing & file count helpers ----

export type LayerOpcode =
  | "FROM"
  | "RUN"
  | "COPY"
  | "ADD"
  | "ENV"
  | "ARG"
  | "DIR"
  | "EXPOSE"
  | "CMD"
  | "ENTRY"
  | "USER"
  | "LABEL"
  | "VOL"
  | "META";

export interface ParsedLayerCommand {
  opcode: LayerOpcode;
  summary: string;
  envArgs: string[];
  formatted: string;
}

type LexFrame =
  | { kind: "NORMAL" }
  | { kind: "SINGLE" }
  | { kind: "DOUBLE" }
  | { kind: "BACKTICK" }
  | { kind: "CMD_SUBST"; parenDepth: number };

type ShellOp = ";;" | ";" | "&&" | "||" | "";

interface ShellSegment {
  code: string;
  op: ShellOp;
}

const PIPE_OR_REDIRECT_TOKENS = new Set([
  "|",
  ">",
  ">>",
  "<",
  ">&2",
  "2>&1",
  "1>&2",
]);

const splitShellSegments = (body: string): ShellSegment[] => {
  const trimmed = body.trim();
  if (!trimmed) return [];

  const segments: ShellSegment[] = [];
  const stack: LexFrame[] = [{ kind: "NORMAL" }];
  let rootParenDepth = 0;
  let current = "";

  const pushSegment = (op: ShellOp) => {
    const code = current.trim();
    if (code) {
      segments.push({ code, op });
    } else if (op && segments.length > 0 && !segments[segments.length - 1].op) {
      segments[segments.length - 1].op = op;
    }
    current = "";
  };

  for (let idx = 0; idx < trimmed.length; idx++) {
    const ch = trimmed[idx];
    const frame = stack[stack.length - 1];

    if (frame.kind === "SINGLE") {
      current += ch;
      if (ch === "'") {
        stack.pop();
      }
      continue;
    }

    if (frame.kind === "DOUBLE") {
      if (ch === "\\") {
        if (trimmed[idx + 1] === "\n") {
          idx++;
          continue;
        }
        if (trimmed[idx + 1] === "\r" && trimmed[idx + 2] === "\n") {
          idx += 2;
          continue;
        }
        if (idx + 1 < trimmed.length) {
          current += ch + trimmed[idx + 1];
          idx++;
          continue;
        }
      }
      if (ch === "$" && trimmed[idx + 1] === "(") {
        current += "$(";
        idx++;
        stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
        continue;
      }
      if (ch === "`") {
        current += ch;
        stack.push({ kind: "BACKTICK" });
        continue;
      }
      current += ch;
      if (ch === '"') {
        stack.pop();
      }
      continue;
    }

    if (frame.kind === "BACKTICK") {
      if (ch === "\\" && idx + 1 < trimmed.length) {
        current += ch + trimmed[idx + 1];
        idx++;
        continue;
      }
      current += ch;
      if (ch === "`") {
        stack.pop();
      }
      continue;
    }

    // NORMAL or CMD_SUBST
    if (ch === "\\") {
      if (trimmed[idx + 1] === "\n") {
        idx++;
        continue;
      }
      if (trimmed[idx + 1] === "\r" && trimmed[idx + 2] === "\n") {
        idx += 2;
        continue;
      }
      if (idx + 1 < trimmed.length) {
        current += ch + trimmed[idx + 1];
        idx++;
        continue;
      }
    }

    if (ch === "'") {
      current += ch;
      stack.push({ kind: "SINGLE" });
      continue;
    }
    if (ch === '"') {
      current += ch;
      stack.push({ kind: "DOUBLE" });
      continue;
    }
    if (ch === "`") {
      current += ch;
      stack.push({ kind: "BACKTICK" });
      continue;
    }
    if (ch === "$" && trimmed[idx + 1] === "(") {
      current += "$(";
      idx++;
      stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
      continue;
    }

    if (ch === "(") {
      if (frame.kind === "CMD_SUBST") {
        frame.parenDepth++;
      } else if (stack.length === 1) {
        rootParenDepth++;
      }
      current += ch;
      continue;
    }

    if (ch === ")") {
      if (frame.kind === "CMD_SUBST") {
        if (frame.parenDepth === 0) {
          stack.pop();
        } else {
          frame.parenDepth--;
        }
      } else if (stack.length === 1) {
        rootParenDepth = Math.max(0, rootParenDepth - 1);
      }
      current += ch;
      continue;
    }

    if (ch === " " || ch === "\t" || ch === "\r" || ch === "\n") {
      if (current.length > 0 && !current.endsWith(" ")) {
        current += " ";
      }
      continue;
    }

    if (
      stack.length === 1 &&
      frame.kind === "NORMAL" &&
      rootParenDepth === 0
    ) {
      if (ch === ";" && trimmed[idx + 1] === ";") {
        pushSegment(";;");
        idx++;
        rootParenDepth = 0;
        continue;
      }
      if (ch === ";") {
        pushSegment(";");
        continue;
      }
      if (ch === "&" && trimmed[idx + 1] === "&") {
        pushSegment("&&");
        idx++;
        continue;
      }
      if (ch === "|" && trimmed[idx + 1] === "|") {
        pushSegment("||");
        idx++;
        continue;
      }
    }

    current += ch;
  }

  pushSegment("");
  return segments;
};

const splitCaseHeader = (code: string): [string, string] | null => {
  if (!/^case\s+/.test(code)) return null;
  const stack: LexFrame[] = [{ kind: "NORMAL" }];

  for (let idx = 0; idx < code.length; idx++) {
    const ch = code[idx];
    const frame = stack[stack.length - 1];
    if (frame.kind === "SINGLE") {
      if (ch === "'") stack.pop();
      continue;
    }
    if (frame.kind === "DOUBLE") {
      if (ch === "\\") {
        idx++;
        continue;
      }
      if (ch === "$" && code[idx + 1] === "(") {
        idx++;
        stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
        continue;
      }
      if (ch === "`") {
        stack.push({ kind: "BACKTICK" });
        continue;
      }
      if (ch === '"') stack.pop();
      continue;
    }
    if (frame.kind === "BACKTICK") {
      if (ch === "\\") {
        idx++;
        continue;
      }
      if (ch === "`") stack.pop();
      continue;
    }
    if (ch === "\\") {
      idx++;
      continue;
    }
    if (ch === "'") {
      stack.push({ kind: "SINGLE" });
      continue;
    }
    if (ch === '"') {
      stack.push({ kind: "DOUBLE" });
      continue;
    }
    if (ch === "`") {
      stack.push({ kind: "BACKTICK" });
      continue;
    }
    if (ch === "$" && code[idx + 1] === "(") {
      idx++;
      stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
      continue;
    }
    if (ch === "(" && frame.kind === "CMD_SUBST") {
      frame.parenDepth++;
      continue;
    }
    if (ch === ")" && frame.kind === "CMD_SUBST") {
      if (frame.parenDepth === 0) stack.pop();
      else frame.parenDepth--;
      continue;
    }

    if (
      stack.length === 1 &&
      frame.kind === "NORMAL" &&
      code.slice(idx, idx + 4) === " in "
    ) {
      const header = code.slice(0, idx + 3).trim();
      const rest = code.slice(idx + 4).trim();
      if (rest) {
        return [header, rest];
      }
      return null;
    }
  }
  return null;
};

const normalizeControlFlowSegments = (
  rawSegments: ShellSegment[],
): ShellSegment[] => {
  const segs = rawSegments.map((s) => ({ ...s }));
  const out: ShellSegment[] = [];
  let i = 0;

  while (i < segs.length) {
    const seg = segs[i];

    const caseParts = splitCaseHeader(seg.code);
    if (caseParts) {
      out.push({ code: caseParts[0], op: "" });
      segs[i] = { code: caseParts[1], op: seg.op };
      continue;
    }

    if (
      /^(if|elif|while|until|for)\b/.test(seg.code) &&
      seg.op === ";" &&
      i + 1 < segs.length
    ) {
      const nextSeg = segs[i + 1];
      const thenMatch = nextSeg.code.match(/^(then|do)(?:\s+([\s\S]*))?$/);
      if (thenMatch) {
        const kw = thenMatch[1];
        const remainder = (thenMatch[2] || "").trim();
        out.push({ code: `${seg.code}; ${kw}`, op: "" });
        if (remainder) {
          segs[i + 1] = { code: remainder, op: nextSeg.op };
        } else {
          segs.splice(i + 1, 1);
        }
        i++;
        continue;
      }
    }

    const elseMatch = seg.code.match(/^else\s+([\s\S]+)$/);
    if (elseMatch) {
      out.push({ code: "else", op: "" });
      segs[i] = { code: elseMatch[1].trim(), op: seg.op };
      continue;
    }

    out.push(seg);
    i++;
  }

  if (out.length > 0 && out[out.length - 1].op === ";") {
    out[out.length - 1].op = "";
  }
  return out;
};

const tokenizeShellWords = (code: string): string[] => {
  const tokens: string[] = [];
  const stack: LexFrame[] = [{ kind: "NORMAL" }];
  let current = "";

  for (let idx = 0; idx < code.length; idx++) {
    const ch = code[idx];
    const frame = stack[stack.length - 1];

    if (frame.kind === "SINGLE") {
      current += ch;
      if (ch === "'") stack.pop();
      continue;
    }
    if (frame.kind === "DOUBLE") {
      if (ch === "\\" && idx + 1 < code.length) {
        current += ch + code[idx + 1];
        idx++;
        continue;
      }
      if (ch === "$" && code[idx + 1] === "(") {
        current += "$(";
        idx++;
        stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
        continue;
      }
      if (ch === "`") {
        current += ch;
        stack.push({ kind: "BACKTICK" });
        continue;
      }
      current += ch;
      if (ch === '"') stack.pop();
      continue;
    }
    if (frame.kind === "BACKTICK") {
      if (ch === "\\" && idx + 1 < code.length) {
        current += ch + code[idx + 1];
        idx++;
        continue;
      }
      current += ch;
      if (ch === "`") stack.pop();
      continue;
    }

    if (ch === "\\" && idx + 1 < code.length) {
      current += ch + code[idx + 1];
      idx++;
      continue;
    }
    if (ch === "'") {
      current += ch;
      stack.push({ kind: "SINGLE" });
      continue;
    }
    if (ch === '"') {
      current += ch;
      stack.push({ kind: "DOUBLE" });
      continue;
    }
    if (ch === "`") {
      current += ch;
      stack.push({ kind: "BACKTICK" });
      continue;
    }
    if (ch === "$" && code[idx + 1] === "(") {
      current += "$(";
      idx++;
      stack.push({ kind: "CMD_SUBST", parenDepth: 0 });
      continue;
    }
    if (ch === "(" && frame.kind === "CMD_SUBST") {
      frame.parenDepth++;
      current += ch;
      continue;
    }
    if (ch === ")" && frame.kind === "CMD_SUBST") {
      if (frame.parenDepth === 0) stack.pop();
      else frame.parenDepth--;
      current += ch;
      continue;
    }

    if (
      stack.length === 1 &&
      frame.kind === "NORMAL" &&
      (ch === " " || ch === "\t")
    ) {
      if (current) {
        tokens.push(current);
        current = "";
      }
      continue;
    }
    current += ch;
  }

  if (current) {
    tokens.push(current);
  }
  return tokens;
};

export const formatShellLines = (body: string): string[] => {
  const normalized = normalizeControlFlowSegments(splitShellSegments(body));
  if (normalized.length === 0) return [];

  const lines: string[] = [];
  const INDENT = "    ";
  let indentLevel = 0;

  for (const seg of normalized) {
    if (/^(elif|else|fi|done|esac)\b/.test(seg.code)) {
      indentLevel = Math.max(0, indentLevel - 1);
    }

    const indent = INDENT.repeat(indentLevel);
    const opSuffix = !seg.op
      ? ""
      : seg.op === ";"
        ? ";"
        : ` ${seg.op}`;
    const lineWithOp = `${seg.code}${opSuffix}`;

    if (
      indent.length + lineWithOp.length > 92 &&
      seg.op !== ";;" &&
      !/^(case|if|elif|while|until|for)\b/.test(seg.code)
    ) {
      const tokens = tokenizeShellWords(seg.code);
      if (
        tokens.length >= 4 &&
        !tokens.some((t) => PIPE_OR_REDIRECT_TOKENS.has(t))
      ) {
        let curLine = `${indent}${tokens[0]} ${tokens[1]}`;
        for (let t = 2; t < tokens.length; t++) {
          const tok = tokens[t];
          if (curLine.length + 1 + tok.length <= 84) {
            curLine += ` ${tok}`;
          } else {
            lines.push(curLine);
            curLine = `${indent}${INDENT}${tok}`;
          }
        }
        lines.push(`${curLine}${opSuffix}`);
      } else {
        lines.push(`${indent}${lineWithOp}`);
      }
    } else {
      lines.push(`${indent}${lineWithOp}`);
    }

    if (
      /;\s*(then|do)$/.test(seg.code) ||
      seg.code === "else" ||
      /^case\b[\s\S]*\bin$/.test(seg.code)
    ) {
      indentLevel++;
    }
  }

  return lines;
};

const formatShellBody = (body: string): string => {
  const lines = formatShellLines(body);
  if (lines.length === 0) return "";
  return lines
    .map((line, idx) => (idx < lines.length - 1 ? `${line} \\` : line))
    .join("\n");
};

export const formatDockerfile = (dockerfile: string): string => {
  if (!dockerfile) return "";

  const blocks: { text: string; multiLine: boolean }[] = [];

  for (const rawLine of dockerfile.split("\n")) {
    let line = rawLine.trim();
    if (!line) continue;

    // Strip `# buildkit` trailing comment
    line = line.replace(/\s*#\s*buildkit\s*$/i, "").trim();
    if (!line) continue;

    // Unwrap legacy `/bin/sh -c #(nop) ...`
    line = line
      .replace(/^(?:RUN\s+)?\/bin\/(?:sh|bash|ash)\s+-c\s+#\(nop\)\s+/i, "")
      .trim();

    // Clean Go-style `EXPOSE map[6379/tcp:{}]`
    const exposeMatch = line.match(/^EXPOSE\s+map\[([^\]]+)\]\s*$/i);
    if (exposeMatch) {
      const ports = exposeMatch[1]
        .trim()
        .split(/\s+/)
        .map((entry) => entry.replace(/:.*$/, ""))
        .filter(Boolean);
      if (ports.length > 0) {
        line = `EXPOSE ${ports.join(" ")}`;
      }
    }

    // Detect shell-form RUN instructions (or bare BuildKit/sh -c lines)
    let shellRest: string | null = null;
    const runMatch = line.match(/^RUN\s+(?!\[)([\s\S]+)$/i);
    if (runMatch) {
      shellRest = runMatch[1].trim();
    } else if (
      /^(?:\|\d+\s+[\s\S]*?)?\/bin\/(?:sh|bash|ash)\s+-c\s+/.test(line)
    ) {
      shellRest = line;
    }

    if (shellRest !== null) {
      let body = shellRest;
      if (/^\|\d+\s+/.test(body)) {
        const shMatch = body.match(/\/bin\/(?:sh|bash|ash)\s+-c\s+([\s\S]*)$/);
        if (shMatch) {
          body = shMatch[1].trim();
        }
      } else {
        body = body
          .replace(/^\/bin\/(?:sh|bash|ash)\s+-c\s+/, "")
          .trim();
      }

      const shellLines = formatShellLines(body);
      if (shellLines.length <= 1) {
        blocks.push({
          text: `RUN ${shellLines[0] || body}`,
          multiLine: false,
        });
      } else {
        const outLines = [`RUN ${shellLines[0]} \\`];
        for (let k = 1; k < shellLines.length - 1; k++) {
          outLines.push(`    ${shellLines[k]} \\`);
        }
        outLines.push(`    ${shellLines[shellLines.length - 1]}`);
        blocks.push({
          text: outLines.join("\n"),
          multiLine: true,
        });
      }
    } else {
      blocks.push({ text: line, multiLine: false });
    }
  }

  let out = "";
  for (let i = 0; i < blocks.length; i++) {
    if (i > 0) {
      out += blocks[i - 1].multiLine || blocks[i].multiLine ? "\n\n" : "\n";
    }
    out += blocks[i].text;
  }
  return out;
};

export const parseLayerCommand = (
  rawCmd: string,
  layerIndex: number,
  layerSize = 0,
): ParsedLayerCommand => {
  let text = (rawCmd || "").trim();
  if (!text) {
    return {
      opcode: layerIndex === 0 && layerSize > 0 ? "FROM" : "META",
      summary: layerIndex === 0 ? "base image" : "metadata",
      envArgs: [],
      formatted: "",
    };
  }

  // Strip `/bin/sh -c #(nop) ` prefix common in Docker history.
  text = text.replace(/^\/bin\/sh\s+-c\s+#\(nop\)\s+/i, "").trim();

  // Check for BuildKit `RUN |N ARG=VAL ... /bin/sh -c ...` or `|N ARG=VAL ...`
  const envArgs: string[] = [];
  let explicitOp = "";

  const opMatch = text.match(
    /^(RUN|COPY|ADD|ENV|ARG|WORKDIR|EXPOSE|CMD|ENTRYPOINT|USER|LABEL|VOLUME|FROM|SHELL|STOPSIGNAL|HEALTHCHECK)\b\s*/i,
  );
  if (opMatch) {
    explicitOp = opMatch[1].toUpperCase();
    text = text.slice(opMatch[0].length).trim();
  }

  // Strip BuildKit `|N KEY=VALUE ... /bin/sh -c ` boilerplate.
  const bkMatch = text.match(/^\|(\d+)\s+([\s\S]*?)\s+\/bin\/sh\s+-c\s+([\s\S]*)$/);
  if (bkMatch) {
    const argCount = parseInt(bkMatch[1], 10);
    const argBlob = bkMatch[2].trim();
    if (argBlob && argCount > 0) {
      // Split roughly on ` KEY=` boundaries
      const parts = argBlob.split(/\s+(?=[A-Za-z_][A-Za-z0-9_]*=)/);
      for (const p of parts) {
        if (p.trim()) envArgs.push(p.trim());
      }
    }
    text = bkMatch[3].trim();
    if (!explicitOp) explicitOp = "RUN";
  } else {
    const shMatch = text.match(/^\/bin\/sh\s+-c\s+([\s\S]*)$/);
    if (shMatch) {
      text = shMatch[1].trim();
      if (!explicitOp) explicitOp = "RUN";
    }
  }

  // Strip `# buildkit` trailing comment
  text = text.replace(/\s*#\s*buildkit\s*$/i, "").trim();

  let opcode: LayerOpcode = "RUN";
  if (explicitOp === "WORKDIR") opcode = "DIR";
  else if (explicitOp === "ENTRYPOINT") opcode = "ENTRY";
  else if (explicitOp === "VOLUME") opcode = "VOL";
  else if (
    explicitOp === "FROM" ||
    explicitOp === "RUN" ||
    explicitOp === "COPY" ||
    explicitOp === "ADD" ||
    explicitOp === "ENV" ||
    explicitOp === "ARG" ||
    explicitOp === "EXPOSE" ||
    explicitOp === "CMD" ||
    explicitOp === "USER" ||
    explicitOp === "LABEL"
  ) {
    opcode = explicitOp as LayerOpcode;
  } else if (
    layerIndex === 0 &&
    /^(ADD|COPY)\s+file:/i.test(rawCmd)
  ) {
    opcode = "FROM";
  } else if (explicitOp) {
    opcode = "META";
  }

  const formatted =
    opcode === "RUN" ? formatShellBody(text) : text;

  // Build concise 1-line summary for the Layer Timeline
  const firstMeaningfulLine =
    formatted
      .split("\n")
      .map((l) => l.replace(/\s*\\$/, "").trim())
      .find(
        (l) =>
          l &&
          l !== "set -eux;" &&
          l !== "set -ex;" &&
          l !== "set -e;" &&
          !l.startsWith("#"),
      ) || text;
  const compactSummary = firstMeaningfulLine.replace(/\s+/g, " ").trim();

  return {
    opcode,
    summary: compactSummary || opcode.toLowerCase(),
    envArgs,
    formatted,
  };
};

export interface LayerFileCounts {
  total: number;
  added: number;
  modified: number;
  removed: number;
}

export const countLayerFiles = (
  items: FileTreeList[] | undefined,
): LayerFileCounts => {
  const counts: LayerFileCounts = {
    total: 0,
    added: 0,
    modified: 0,
    removed: 0,
  };
  if (!items) return counts;
  const walk = (nodes: FileTreeList[]) => {
    for (const node of nodes) {
      if (node.children.length > 0) {
        walk(node.children);
      } else if (node.name) {
        counts.total++;
        if (node.op === opRemoved) counts.removed++;
        else if (node.op === opModified) counts.modified++;
        else counts.added++;
      }
    }
  };
  walk(items);
  return counts;
};
