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
