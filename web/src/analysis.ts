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

export interface FileTreeViewOption {
  expandAll: boolean;
  expandItems: string[];
  sizeLimit: number;
  onlyModifiedRemoved: boolean;
  keyword: string;
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
  const wastedList: FileWastedSummary[] = [];
  // 计算浪费的空间以及文件
  result.fileSummaryList.forEach((item) => {
    const { size, path } = item.info;
    const found = wastedList.find((item) => item.path === path);
    if (found) {
      found.count++;
      found.totalSize += size;
    } else {
      wastedList.push({
        path,
        count: 1,
        totalSize: size,
      });
    }
    wastedSize += size;
  });
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
  if (item.name.includes(keyword)) {
    return true;
  }
  // 如果子元素符合，则也符合
  for (let i = 0; i < item.children.length; i++) {
    if (isMatchKeyword(item.children[i], keyword)) {
      return true;
    }
  }
  return false;
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
  const expandAll = !!(opt.expandAll || opt.keyword);
  const shouldExpand = (key: string) =>
    expandAll || !!(opt.expandItems && opt.expandItems.includes(key));

  for (const item of items) {
    if (opt.sizeLimit && item.size < opt.sizeLimit) {
      continue;
    }
    if (opt.onlyModifiedRemoved && !isModifiedRemoved(item)) {
      continue;
    }
    if (opt.keyword && !isMatchKeyword(item, opt.keyword)) {
      continue;
    }
    const isDir = item.children.length > 0;
    const expanded = isDir && shouldExpand(item.key);
    out.push({
      key: item.key,
      mode: item.mode,
      uid: item.uid,
      gid: item.gid,
      size: item.size,
      name: item.name,
      link: item.link,
      op: item.op,
      depth,
      isDir,
      expanded,
    });
    if (isDir && expanded) {
      const beforeChildren = out.length;
      flattenFileTree(item.children, depth + 1, opt, out);
      // Drop empty directories when keyword filtering produced no children
      // and no keyword is active (matches prior list.pop() behaviour).
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
