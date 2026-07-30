// Data shapes returned by the diving JSON API plus view-model interfaces
// shared across components.

export interface ModifiedFile {
  digest: string;
  path: string;
  size: number;
}

export interface SensitiveFile {
  path: string;
  size: number;
  layerIndex: number;
  reason: string;
}

export interface DuplicatePath {
  layerIndex: number;
  path: string;
}

export interface DuplicateGroup {
  hash: string;
  size: number;
  count: number;
  totalWasted: number;
  paths: DuplicatePath[];
}

export interface RuntimeCompat {
  entrypoint: string;
  libc: string;
  requiredGlibc: string;
  neededLibs: string[];
  osGlibc: string;
  issue: string;
}

export interface Recommendation {
  category: string;
  severity: string;
  title: string;
  detail: string;
  dockerfileHint: string;
  estSavedBytes: number;
  heuristic: boolean;
  paths: string[];
}

export interface Layer {
  created: string;
  digest: string;
  mediaType: string;
  cmd: string;
  size: number;
  unpackSize: number;
  empty: boolean;
}

export interface FileTreeList {
  key: string;
  name: string;
  link: string;
  size: number;
  mode: string;
  uid: number;
  gid: number;
  op: number;
  children: FileTreeList[];
}

export interface Info {
  path: string;
  link: string;
  size: number;
  mode: string;
  uid: number;
  gid: number;
  isWhiteout: boolean;
}

export interface FileSummaryList {
  layerIndex: number;
  op: number;
  info: Info;
}

export interface FileWastedSummary {
  path: string;
  totalSize: number;
  count: number;
}

export interface ImageAnalyzeResult {
  name: string;
  arch: string;
  os: string;
  user?: string;
  baseOs?: string;
  dockerfile?: string;
  tags?: string[];
  layers: Layer[];
  size: number;
  totalSize: number;
  fileTreeList: FileTreeList[][];
  fileSummaryList: FileSummaryList[];
  bigModifiedFileList: ModifiedFile[];
  sensitiveFiles?: SensitiveFile[];
  duplicateGroups?: DuplicateGroup[];
  recommendations?: Recommendation[];
  runtimeCompat?: RuntimeCompat;
}

export interface ImageDescriptions {
  score: string;
  size: string;
  otherSize: string;
  wastedSize: string;
  osArch: string;
  created: string;
  baseOs: string;
  runtimeLibc: string;
  runtimeLibcIssue: string;
  runUser: string;
}

export interface LatestImages {
  images: string[];
  version: string;
}
