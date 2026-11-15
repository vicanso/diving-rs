// Presentational components. Report cards are memoized so filter typing
// in App does not re-render them.

import {
  Fragment,
  memo,
  PointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Alert,
  Card,
  Input,
  List,
  Modal,
  Select,
  Spin,
  Typography,
} from "antd";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  buildFinalTree,
  dominantLayer,
  FileTreeRow,
  formatDockerfile,
  FsNode,
  layerBytes,
  LayerFileCounts,
  LayerOpcode,
  opAdded,
  opModified,
  opRemoved,
  parseLayerCommand,
  squarify,
  TreemapRect,
} from "./analysis";
import {
  getDownloadIcon,
  getPreviewIcon,
  minusOutlined,
  plusOutlined,
} from "./icons";
import {
  DuplicateGroup,
  FilePreview,
  FileTreeList,
  ImageDescriptions,
  ImagePreview,
  Layer,
  McpStatus,
  ModifiedFile,
  Recommendation,
  SensitiveFile,
  FileWastedSummary,
  WasteChange,
} from "./types";

const { Option } = Select;
const { Search } = Input;

const FILE_TREE_ROW_HEIGHT = 34;
const FILE_TREE_OVERSCAN = 8;

const useFluidTreeHeight = () => {
  const [maxH, setMaxH] = useState(() =>
    typeof window !== "undefined"
      ? Math.max(460, window.innerHeight - 400)
      : 600,
  );
  useEffect(() => {
    const onResize = () =>
      setMaxH(Math.max(460, window.innerHeight - 400));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return maxH;
};

export const StatusBadge = ({
  op,
  isDir,
}: {
  op: number;
  isDir: boolean;
}) => {
  if (op === opRemoved) {
    return <span className="statusBadge del">- DEL</span>;
  }
  if (op === opModified) {
    return <span className="statusBadge mod">~ MOD</span>;
  }
  if (op === opAdded || (op === 0 && !isDir)) {
    return <span className="statusBadge add">+ ADD</span>;
  }
  return <span className="statusBadgeSpacer" aria-hidden="true" />;
};

export const SizeMicroBar = ({
  size,
  maxLayerSize,
}: {
  size: number;
  maxLayerSize: number;
}) => {
  const percent =
    maxLayerSize > 0
      ? Math.min(100, Math.round((size / maxLayerSize) * 100))
      : 0;
  const isExtreme = size > 50 * 1024 * 1024;
  const isHeavy = size > 5 * 1024 * 1024;
  return (
    <div
      className="sizeBarTrack"
      title={`${prettyBytes(size)} (${percent}%)`}
      aria-hidden="true"
    >
      <div
        className={`sizeBarFill ${isExtreme ? "extreme" : isHeavy ? "heavy" : ""}`}
        style={{ width: `${percent}%` }}
      />
    </div>
  );
};

const fileUrl = (api: string, layer: Layer, path: string) =>
  `./api/${api}?digest=${encodeURIComponent(
    layer.digest,
  )}&mediaType=${encodeURIComponent(layer.mediaType)}&file=${encodeURIComponent(
    path,
  )}`;

/** Read-only look at one file of a layer: text, or a hex dump for binaries. */
const FilePreviewModal = ({
  layer,
  row,
  onClose,
}: {
  layer: Layer;
  row: FileTreeRow | null;
  onClose: () => void;
}) => {
  const [preview, setPreview] = useState<FilePreview | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!row) {
      return;
    }
    setPreview(null);
    setError("");
    let cancelled = false;
    fetch(fileUrl("file-preview", layer, row.key))
      .then(async (resp) => {
        const data = await resp.json();
        if (!resp.ok) {
          throw new Error(data?.message || resp.statusText);
        }
        return data as FilePreview;
      })
      .then((data) => !cancelled && setPreview(data))
      .catch((err: Error) => !cancelled && setError(err.message));
    return () => {
      cancelled = true;
    };
  }, [layer, row]);

  let notice = "";
  if (preview?.binary) {
    notice = i18nGet("previewBinary");
  } else if (preview?.truncated) {
    notice = i18nGet("previewTruncated");
  }
  return (
    <Modal
      className="filePreview"
      title={row ? `/${row.key}` : ""}
      open={!!row}
      onCancel={onClose}
      footer={null}
      width={800}
    >
      {row && (
        <p className="filePreviewMeta">
          <code>{row.mode}</code>
          <span>
            {row.uid}:{row.gid}
          </span>
          <span>{prettyBytes(row.size)}</span>
          <a href={fileUrl("file", layer, row.key)}>
            {getDownloadIcon()} {i18nGet("previewDownload")}
          </a>
        </p>
      )}
      {error && <Alert type="error" showIcon title={error} />}
      {!preview && !error && <Spin />}
      {notice && <p className="filePreviewNotice">{notice}</p>}
      {preview && <pre className="filePreviewBody">{preview.content}</pre>}
    </Modal>
  );
};

/** Windowed file-tree list — only mounts rows near the scroll viewport. */
export const VirtualFileTree = ({
  rows,
  layer,
  onToggleExpand,
  showMetaColumns = false,
  emptyMessage,
}: {
  rows: FileTreeRow[];
  layer: Layer;
  onToggleExpand: (key: string) => void;
  showMetaColumns?: boolean;
  emptyMessage?: string;
  isDark?: boolean;
}) => {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [previewRow, setPreviewRow] = useState<FileTreeRow | null>(null);
  const fluidMaxHeight = useFluidTreeHeight();

  const onScroll = useCallback(() => {
    if (scrollerRef.current) {
      setScrollTop(scrollerRef.current.scrollTop);
    }
  }, []);

  useEffect(() => {
    if (scrollerRef.current) {
      scrollerRef.current.scrollTop = 0;
    }
    setScrollTop(0);
  }, [rows]);

  const maxFileSize = useMemo(() => {
    let max = 0;
    for (const r of rows) {
      if (!r.isDir && r.size > max) max = r.size;
    }
    if (max === 0) {
      for (const r of rows) {
        if (r.size > max) max = r.size;
      }
    }
    return max;
  }, [rows]);

  if (rows.length === 0) {
    return (
      <div className="fileTreeEmpty">
        {emptyMessage || i18nGet("noMatchingFiles")}
      </div>
    );
  }

  const contentHeight = Math.max(
    rows.length * FILE_TREE_ROW_HEIGHT,
    FILE_TREE_ROW_HEIGHT,
  );
  const viewportHeight = Math.min(contentHeight, fluidMaxHeight);
  const start = Math.max(
    0,
    Math.floor(scrollTop / FILE_TREE_ROW_HEIGHT) - FILE_TREE_OVERSCAN,
  );
  const visibleCount =
    Math.ceil(viewportHeight / FILE_TREE_ROW_HEIGHT) + FILE_TREE_OVERSCAN * 2;
  const end = Math.min(rows.length, start + visibleCount);
  const slice = rows.slice(start, end);

  return (
    <div
      className="virtualFileTreeScroller"
      ref={scrollerRef}
      onScroll={onScroll}
      style={{
        height: viewportHeight,
      }}
    >
      <ul
        className="fileTree virtualFileTree"
        style={{
          height: contentHeight,
          position: "relative",
          margin: 0,
          padding: 0,
        }}
      >
        {slice.map((row, i) => {
          const index = start + i;
          const rowTop = index * FILE_TREE_ROW_HEIGHT;
          let opClass = "";
          if (row.op === opRemoved) {
            opClass = "removed";
          } else if (row.op === opModified) {
            opClass = "modified";
          }
          const isHeavy = !row.isDir && row.size > 5 * 1024 * 1024;
          let name = row.name;
          if (row.link) {
            name = `${name} → ${row.link}`;
          }
          return (
            <li
              key={row.key}
              className={`virtualFileTreeRow ${showMetaColumns ? "withMeta" : ""}`}
              style={{
                position: "absolute",
                top: rowTop,
                left: 0,
                right: 0,
                height: FILE_TREE_ROW_HEIGHT,
              }}
            >
              <span className="colStatus">
                <StatusBadge op={row.op} isDir={row.isDir} />
              </span>
              {showMetaColumns && (
                <>
                  <span className="colMode">{row.mode || "—"}</span>
                  <span className="colUid">
                    {row.uid}:{row.gid}
                  </span>
                </>
              )}
              <span className={`colSize ${isHeavy ? "heavy" : ""}`}>
                {prettyBytes(row.size)}
              </span>
              <span className="colBar">
                <SizeMicroBar size={row.size} maxLayerSize={maxFileSize} />
              </span>
              <span
                className={`colPath ${opClass} ${row.isDir ? "isDir" : ""}`}
                style={{ paddingLeft: row.depth * 18 }}
              >
                {row.depth > 0 && (
                  <span
                    className="treeGuides"
                    aria-hidden="true"
                    style={{ width: row.depth * 18 }}
                  >
                    {Array.from({ length: row.depth }).map((_, d) => (
                      <span
                        key={d}
                        className="treeGuideLine"
                        style={{ left: d * 18 + 7 }}
                      />
                    ))}
                  </span>
                )}
                {row.isDir && (
                  <a
                    href="#"
                    className="icon"
                    onClick={(e) => {
                      e.preventDefault();
                      onToggleExpand(row.key);
                    }}
                  >
                    {row.expanded ? minusOutlined : plusOutlined}
                  </a>
                )}
                <span className="fileNameText" title={row.key}>
                  {name}
                </span>
                {!row.isDir && row.size > 0 && row.op !== opRemoved && (
                  <span className="rowActions">
                    <a
                      href="#"
                      className="download"
                      title={i18nGet("previewButton")}
                      onClick={(e) => {
                        e.preventDefault();
                        setPreviewRow(row);
                      }}
                    >
                      {getPreviewIcon()}
                    </a>
                    <a
                      className="download"
                      title={i18nGet("previewDownload")}
                      href={fileUrl("file", layer, row.key)}
                    >
                      {getDownloadIcon()}
                    </a>
                  </span>
                )}
              </span>
            </li>
          );
        })}
      </ul>
      <FilePreviewModal
        layer={layer}
        row={previewRow}
        onClose={() => setPreviewRow(null)}
      />
    </div>
  );
};

export const SearchBar = ({
  arch,
  platforms,
  defaultImage,
  loading,
  onArchChange,
  onSearch,
  size = "large",
}: {
  arch: string;
  /** The shown image's platforms; amd64 / arm64 when there is none. */
  platforms?: string[];
  defaultImage: string;
  loading: boolean;
  onArchChange: (arch: string) => void;
  onSearch: (image: string) => void;
  size?: "large" | "middle";
}) => {
  const choices =
    platforms && platforms.length > 0 ? platforms : ["amd64", "arm64"];
  const selectBefore = (
    <Select
      size={size}
      value={arch}
      style={{ width: size === "large" ? 108 : 96 }}
      popupMatchSelectWidth={false}
      onChange={onArchChange}
    >
      {(choices.includes(arch) ? choices : [...choices, arch]).map((p) => (
        <Option key={p} value={p}>
          {p.toUpperCase()}
        </Option>
      ))}
    </Select>
  );
  return (
    <Search
      addonBefore={selectBefore}
      defaultValue={defaultImage}
      key={defaultImage}
      autoFocus={true}
      loading={loading}
      placeholder={i18nGet("imageInputPlaceholder")}
      allowClear
      enterButton={i18nGet("analyzeButton")}
      size={size}
      // antd also passes the event and the trigger source.
      onSearch={(value) => onSearch(value)}
    />
  );
};

const scoreTone = (scoreStr: string) => {
  const n = parseInt(scoreStr, 10);
  if (Number.isNaN(n)) return "";
  if (n < 80) return "score-bad";
  if (n < 95) return "score-warn";
  return "";
};

/** SVG ring for efficiency score (0–100). */
const ScoreGauge = ({ score }: { score: string }) => {
  const n = Math.min(100, Math.max(0, parseInt(score, 10) || 0));
  const r = 46;
  const c = 2 * Math.PI * r;
  const offset = c * (1 - n / 100);
  const tone = scoreTone(score);
  return (
    <div className={`scoreGauge ${tone}`.trim()}>
      <svg viewBox="0 0 112 112" aria-hidden>
        <circle className="track" cx="56" cy="56" r={r} />
        <circle
          className="value"
          cx="56"
          cy="56"
          r={r}
          strokeDasharray={c}
          strokeDashoffset={offset}
        />
      </svg>
      <div className="scoreGaugeCenter">
        <span className="scoreGaugeNum">{score}</span>
        <span className="scoreGaugeLabel">%</span>
      </div>
    </div>
  );
};

export const ImageSummaryCard = memo(
  ({ desc, tags }: { desc: ImageDescriptions; tags?: string[] }) => {
    const hasTags = tags && tags.length > 0;
    const scoreOnly = (desc.score || "0").replace("%", "");
    // Compact key/value rows — avoid Ant Descriptions' equal-width columns
    // which leave a large empty middle when only 2–4 fields are present.
    const metaItems: { label: string; value: React.ReactNode }[] = [
      { label: i18nGet("otherLayerSizeLabel"), value: desc.otherSize },
      { label: i18nGet("osArchLabel"), value: desc.osArch },
    ];
    if (desc.runUser) {
      metaItems.push({ label: i18nGet("runAsUserLabel"), value: desc.runUser });
    }
    if (desc.baseOs) {
      metaItems.push({ label: i18nGet("baseOsLabel"), value: desc.baseOs });
    }
    if (desc.runtimeLibc) {
      metaItems.push({
        label: i18nGet("runtimeLibcLabel"),
        value: (
          <span
            style={{
              color: desc.runtimeLibcIssue ? "var(--danger)" : undefined,
            }}
          >
            {desc.runtimeLibc}
          </span>
        ),
      });
    }
    metaItems.push({
      label: i18nGet("createdLabel"),
      value: desc.created ? new Date(desc.created).toLocaleString() : "—",
    });

    return (
      <section className="panel summaryPanel">
        <div className="summaryHero">
          <ScoreGauge score={scoreOnly} />
          <div className="metricStrip">
            <div className="metricChip">
              <div className="metricChipLabel">
                {i18nGet("imageScoreLabel")}
              </div>
              <div className="metricChipValue">{desc.score}</div>
            </div>
            <div className="metricChip">
              <div className="metricChipLabel">{i18nGet("imageSizeLabel")}</div>
              <div className="metricChipValue">{desc.size}</div>
            </div>
            <div className="metricChip">
              <div className="metricChipLabel">
                {i18nGet("wastedSizeLabel")}
              </div>
              <div className="metricChipValue warn">{desc.wastedSize}</div>
            </div>
          </div>
        </div>
        <dl className="summaryMeta">
          {metaItems.map((item) => (
            <div key={item.label} className="summaryMetaItem">
              <dt>{item.label}</dt>
              <dd>{item.value}</dd>
            </div>
          ))}
        </dl>
        {hasTags && (
          <div className="summaryTags">
            {tags.map((t) => (
              <span key={t} className="riskTag">
                {t}
              </span>
            ))}
          </div>
        )}
      </section>
    );
  },
);

const fill = (template: string, values: Record<string, string>) =>
  Object.entries(values).reduce(
    (text, [key, value]) => text.replace(`{${key}}`, value),
    template,
  );

/** The layer list from the manifest, shown while a first analysis reads
 *  the layers; replaced by the full report when it is done. */
export const ImagePreviewCard = memo(
  ({ preview }: { preview: ImagePreview }) => {
    const fileLayers = preview.layers.filter((l) => !l.empty).length;
    return (
      <Card
        className="panel"
        title={<code className="previewName">{preview.name}</code>}
        extra={
          <span className="previewStatus">
            <Spin size="small" /> {i18nGet("previewAnalyzing")}
          </span>
        }
      >
        <p className="previewMeta">
          {fill(i18nGet("previewMeta"), {
            platform: `${preview.os}/${preview.arch}`,
            layers: String(fileLayers),
            size: prettyBytes(preview.size),
          })}
        </p>
        <ol className="previewLayers">
          {preview.layers.map((layer, i) => (
            <li key={i} className={layer.empty ? "empty" : undefined}>
              <span>{i + 1}</span>
              <span>{layer.empty ? "" : prettyBytes(layer.size)}</span>
              <code title={layer.cmd}>{layer.cmd || "—"}</code>
            </li>
          ))}
        </ol>
        <p className="previewHint">{i18nGet("previewHint")}</p>
      </Card>
    );
  },
);

/** Changes listed per wasted path; the count column covers the rest. */
const WASTE_CHANGES_SHOWN = 3;

/** "written in layer 1, deleted in layer 3" — 1-based, like the layer
 *  select. Analyses cached by 0.12.0 lack the writing layer. */
const wasteChangeText = (change: WasteChange) => {
  const to = String(change.layer + 1);
  if (change.writtenIn === undefined) {
    return fill(
      i18nGet(change.removed ? "wasteRemovedIn" : "wasteOverwrittenIn"),
      { to },
    );
  }
  return fill(i18nGet(change.removed ? "wasteRemoved" : "wasteOverwritten"), {
    from: String(change.writtenIn + 1),
    to,
  });
};

export const WastedSummaryCard = memo(
  ({ wastedList }: { wastedList: FileWastedSummary[]; isDark?: boolean }) => {
    const arr = wastedList.filter((item) => item.totalSize > 0);
    if (arr.length === 0) {
      return null;
    }
    return (
      <Card className="panel flushPanel" title={i18nGet("wastedSummaryTitle")}>
        <ul className="dataTable cols-wasted">
          <li className="head">
            <span>{i18nGet("totalSizeLabel")}</span>
            <span>{i18nGet("countLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {arr.map((item) => (
            <li key={item.path}>
              <span className="monoCell">{prettyBytes(item.totalSize)}</span>
              <span className="monoCell">{item.count}</span>
              <span className="monoCell">
                /{item.path}
                <span className="wasteChanges">
                  {item.changes
                    .slice(0, WASTE_CHANGES_SHOWN)
                    .map(wasteChangeText)
                    .join(" · ")}
                  {item.changes.length > WASTE_CHANGES_SHOWN ? " · …" : ""}
                </span>
              </span>
            </li>
          ))}
        </ul>
      </Card>
    );
  },
);

export const SensitiveFilesCard = memo(
  ({ files }: { files: SensitiveFile[]; isDark?: boolean }) => {
    if (!files || files.length === 0) {
      return null;
    }
    return (
      <Card className="panel flushPanel" title={i18nGet("sensitiveFilesTitle")}>
        <ul className="dataTable cols-sensitive">
          <li className="head">
            <span>{i18nGet("layerLabel")}</span>
            <span>{i18nGet("sizeLabel")}</span>
            <span>{i18nGet("sensitiveReasonLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {files.map((f) => (
            <li key={`${f.layerIndex}-${f.path}`}>
              <span className="monoCell">{f.layerIndex + 1}</span>
              <span className="monoCell">{prettyBytes(f.size || 0)}</span>
              <span>{f.reason}</span>
              <span className="monoCell">/{f.path}</span>
            </li>
          ))}
        </ul>
      </Card>
    );
  },
);

export const DuplicateGroupsCard = memo(
  ({ groups }: { groups: DuplicateGroup[]; isDark?: boolean }) => {
    if (!groups || groups.length === 0) {
      return null;
    }
    const sorted = [...groups].sort((a, b) => b.totalWasted - a.totalWasted);
    return (
      <Card className="panel flushPanel" title={i18nGet("duplicateGroupsTitle")}>
        <ul className="dataTable cols-dup">
          <li className="head">
            <span>{i18nGet("duplicateWastedLabel")}</span>
            <span>{i18nGet("duplicateCountLabel")}</span>
            <span>{i18nGet("sizeLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {sorted.map((g) => {
            const sample =
              g.paths && g.paths.length > 0
                ? g.paths
                    .slice(0, 3)
                    .map((p) => `L${p.layerIndex + 1}:/${p.path}`)
                    .join(" · ")
                : g.hash.slice(0, 12);
            return (
              <li key={g.hash}>
                <span className="monoCell">{prettyBytes(g.totalWasted)}</span>
                <span className="monoCell">{g.count}</span>
                <span className="monoCell">{prettyBytes(g.size)}</span>
                <span className="monoCell" title={g.hash}>
                  {sample}
                </span>
              </li>
            );
          })}
        </ul>
      </Card>
    );
  },
);

export const BigModifiedFilesCard = memo(
  ({ files }: { files: ModifiedFile[]; isDark?: boolean }) => {
    if (files.length === 0) {
      return null;
    }
    const arr = files.slice(0).sort((a, b) => b.size - a.size);
    return (
      <Card className="panel flushPanel" title={i18nGet("modifiedAddedLargeFileTitle")}>
        <ul className="dataTable cols-bigmod">
          <li className="head">
            <span>{i18nGet("layerLabel")}</span>
            <span>{i18nGet("totalSizeLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {arr.map((item) => {
            let { digest } = item;
            if (digest) {
              digest = digest.replace("sha256:", "").substring(0, 8);
            }
            return (
              <li key={item.path}>
                <span className="monoCell">
                  {(digest || "—").toUpperCase()}
                </span>
                <span className="monoCell">{prettyBytes(item.size)}</span>
                <span className="monoCell">/{item.path}</span>
              </li>
            );
          })}
        </ul>
      </Card>
    );
  },
);

export const RecommendationsCard = memo(
  ({
    recommendations,
  }: {
    recommendations: Recommendation[];
    isDark?: boolean;
  }) => {
    if (!recommendations || recommendations.length === 0) {
      return null;
    }
    const severityColor: Record<string, string> = {
      high: "var(--danger)",
      medium: "var(--warn)",
      low: "var(--ok)",
      info: "var(--info)",
    };
    return (
      <Card className="panel flushPanel span2" title={i18nGet("recommendationsTitle")}>
        <ul className="recommendationList">
          {recommendations.map((r, idx) => {
            const color = severityColor[r.severity] || "var(--info)";
            return (
              <li key={`${r.title}-${idx}`} className="recommendationItem">
                <div className="recommendationHead">
                  <span
                    className="recommendationBadge"
                    style={{ backgroundColor: color }}
                  >
                    {r.severity.toUpperCase()} · {r.category}
                    {r.heuristic ? " · heuristic" : ""}
                  </span>
                  <span className="recommendationTitle">{r.title}</span>
                  {r.estSavedBytes > 0 && (
                    <span className="recommendationSaved">
                      ~{prettyBytes(r.estSavedBytes)} {i18nGet("recSavedLabel")}
                    </span>
                  )}
                </div>
                <div className="recommendationDetail">{r.detail}</div>
                {r.dockerfileHint && (
                  <div className="recommendationHint">
                    <b>{i18nGet("recFixLabel")}:</b> {r.dockerfileHint}
                  </div>
                )}
                {r.paths && r.paths.length > 0 && (
                  <ul className="recommendationPaths">
                    {r.paths.map((p) => (
                      <li key={p}>
                        <code>{p}</code>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      </Card>
    );
  },
);

export const DockerfileCard = memo(
  ({ dockerfile }: { dockerfile: string; isDark?: boolean }) => {
    const [showRaw, setShowRaw] = useState(false);
    const [wrap, setWrap] = useState(false);
    const [copied, setCopied] = useState(false);

    const formatted = useMemo(
      () => (dockerfile ? formatDockerfile(dockerfile) : ""),
      [dockerfile],
    );

    if (!dockerfile) {
      return null;
    }

    const content = showRaw ? dockerfile : formatted;
    const lines = content.split("\n");

    const onCopy = () => {
      navigator.clipboard?.writeText(content).then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      });
    };

    return (
      <Card
        className="panel flushPanel span2"
        title={i18nGet("dockerfileTitle")}
        extra={
          <div className="commandBoxActions">
            <button
              type="button"
              className={`cmdActionBtn ${showRaw ? "active" : ""}`}
              onClick={() => setShowRaw((r) => !r)}
            >
              {i18nGet("rawLabel")}
            </button>
            <button
              type="button"
              className={`cmdActionBtn ${wrap ? "active" : ""}`}
              onClick={() => setWrap((w) => !w)}
            >
              {i18nGet("wrapLinesLabel")}
            </button>
            <button type="button" className="cmdActionBtn" onClick={onCopy}>
              {copied ? i18nGet("copiedButton") : i18nGet("copyButton")}
            </button>
          </div>
        }
      >
        <pre className={`dockerfileBlock ${wrap ? "wrap" : ""}`}>
          {lines.map((line, idx) => (
            <div key={idx} className="cmdLine">
              <span className="cmdLineNo">{idx + 1}</span>
              <span className="cmdLineCode">{line}</span>
            </div>
          ))}
        </pre>
      </Card>
    );
  },
);

export const LatestImagesList = memo(({ images }: { images: string[] }) => {
  if (images.length === 0) {
    return null;
  }
  return (
    <List
      className="analyzeImages"
      bordered={true}
      size="small"
      header={<div>{i18nGet("latestAnalyzeImagesTitle")}</div>}
      dataSource={images}
      renderItem={(item) => (
        <List.Item>
          <Typography.Text>
            <a
              href="#"
              onClick={(e) => {
                const arr = item.split("?");
                const image = arr[0];
                let arch = "amd64";
                if (arr[1]) {
                  const result = /arch=(\S+)/.exec(arr[1]);
                  if (result && result.length === 2) {
                    arch = result[1];
                  }
                }
                window.location.href = `/?image=${image}&arch=${arch}`;
                e.preventDefault();
              }}
            >
              {item}
            </a>
          </Typography.Text>
        </List.Item>
      )}
    />
  );
});

const MCP_TOOLS = [
  ["analyze_image", "mcpToolAnalyzeImage"],
  ["get_findings", "mcpToolGetFindings"],
  ["list_files", "mcpToolListFiles"],
  ["read_file", "mcpToolReadFile"],
  ["diff_images", "mcpToolDiffImages"],
  ["latest_images", "mcpToolLatestImages"],
];

const CodeSnippet = ({ code }: { code: string }) => (
  <div className="codeSnippet">
    <pre>{code}</pre>
    <Typography.Text className="codeSnippetCopy" copyable={{ text: code }} />
  </div>
);

/** Header button + dialog explaining how to connect an AI client to `/mcp`. */
export const McpGuide = memo(({ status }: { status: McpStatus }) => {
  const [open, setOpen] = useState(false);
  const [hostBlocked, setHostBlocked] = useState(false);
  // Same origin and base path the page itself was served from.
  const endpoint = new URL("./mcp", window.location.href).href;

  useEffect(() => {
    // Without a token the server only accepts allow-listed Host headers.
    // Ask it whether this address passes rather than mirroring its rules.
    if (!open || status.tokenRequired) {
      return;
    }
    fetch(endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-06-18",
      },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" }),
    })
      .then((resp) => setHostBlocked(resp.status === 403))
      .catch(() => {
        /* advisory check only */
      });
  }, [open, status.tokenRequired, endpoint]);

  const bearer = "Bearer <token>";
  const claudeCommand = status.tokenRequired
    ? `claude mcp add --transport http diving ${endpoint} \\\n  --header "Authorization: ${bearer}"`
    : `claude mcp add --transport http diving ${endpoint}`;
  const jsonConfig = JSON.stringify(
    {
      mcpServers: {
        diving: {
          type: "http",
          url: endpoint,
          ...(status.tokenRequired && {
            headers: { Authorization: bearer },
          }),
        },
      },
    },
    null,
    2,
  );

  return (
    <>
      <button type="button" className="mcpButton" onClick={() => setOpen(true)}>
        {i18nGet("mcpButton")}
      </button>
      <Modal
        className="mcpGuide"
        title={i18nGet("mcpTitle")}
        open={open}
        onCancel={() => setOpen(false)}
        footer={null}
        width={640}
      >
        <p className="mcpIntro">{i18nGet("mcpIntro")}</p>
        {hostBlocked && (
          <Alert
            type="warning"
            showIcon
            title={i18nGet("mcpHostBlocked").replace(
              "{host}",
              window.location.host,
            )}
          />
        )}
        {status.tokenRequired && (
          <Alert type="info" showIcon title={i18nGet("mcpTokenNotice")} />
        )}
        <h4>{i18nGet("mcpEndpointLabel")}</h4>
        <CodeSnippet code={endpoint} />
        <h4>{i18nGet("mcpClaudeLabel")}</h4>
        <CodeSnippet code={claudeCommand} />
        <h4>{i18nGet("mcpJsonLabel")}</h4>
        <CodeSnippet code={jsonConfig} />
        <p className="mcpNote">{i18nGet("mcpJsonNote")}</p>
        <h4>{i18nGet("mcpToolsLabel")}</h4>
        <ul className="mcpTools">
          {MCP_TOOLS.map(([name, descKey]) => (
            <li key={name}>
              <code>{name}</code>
              <span>{i18nGet(descKey)}</span>
            </li>
          ))}
        </ul>
      </Modal>
    </>
  );
});

/** Tiles drawn per directory; the rest merge into one "smaller items" tile. */
const TREEMAP_MAX_TILES = 20;
/** Tiles drawn inside each nested directory. */
const TREEMAP_MAX_CHILD_TILES = 8;
/** Minimum share of parent size for a tile to be drawn individually;
 *  smaller items fold into the "smaller items" summary tile. */
const TREEMAP_MIN_SHARE = 0.015;
const TREEMAP_CHILD_MIN_SHARE = 0.035;
/** Layers that get their own color; every other layer is "Other". Three is
 *  the most the categorical palette keeps distinguishable when any two
 *  colors may touch, as tiles in a treemap can. */
const TREEMAP_COLORED_LAYERS = 3;
const TREEMAP_HEIGHT = 440;
/** Name strip on top of a directory whose children are drawn inside it. */
const TREEMAP_GROUP_HEADER = 22;
/** Room a directory needs before its children are drawn inside it. */
const TREEMAP_GROUP_MIN_W = 110;
const TREEMAP_GROUP_MIN_H = 90;
/** Start below a directory holding at least this share of its parent —
 *  in most images `/` is nearly all `usr/`. */
const TREEMAP_AUTO_OPEN = 0.9;
const TREEMAP_FONT = '12px "IBM Plex Sans", system-ui, sans-serif';
/** Names are drawn at weight 500–600; the canvas measures the 400 face. */
const TREEMAP_BOLD_WIDTH = 1.1;

interface TreemapTile {
  key: string;
  name: string;
  size: number;
  node: FsNode | null;
  /** Color slot 1–3, or 0 for "Other". */
  slot: number;
  /** Layer the tile is attributed to (0-based) and that layer's share. */
  layer: number | null;
  share: number;
}

interface PlacedTile {
  tile: TreemapTile;
  rect: TreemapRect;
}

let measureCtx: CanvasRenderingContext2D | null = null;
const textWidth = (text: string) => {
  if (!measureCtx) {
    measureCtx = document.createElement("canvas").getContext("2d");
    if (measureCtx) measureCtx.font = TREEMAP_FONT;
  }
  return measureCtx ? measureCtx.measureText(text).width : text.length * 7;
};

/** `node`'s children as tiles, largest first, tiny/tail items merged. */
const childTiles = (
  node: FsNode,
  max: number,
  minShare: number,
  slotOf: (layer: number | null) => number,
): TreemapTile[] => {
  const children = [...(node.children?.values() || [])]
    .filter((c) => c.size > 0)
    .sort((a, b) => b.size - a.size);
  const totalSize = node.size || children.reduce((s, c) => s + c.size, 0);
  const kept: FsNode[] = [];
  const rest: FsNode[] = [];
  for (const child of children) {
    const share = totalSize > 0 ? child.size / totalSize : 0;
    if (kept.length < max && (kept.length < 3 || share >= minShare)) {
      kept.push(child);
    } else {
      rest.push(child);
    }
  }
  const tiles: TreemapTile[] = kept.map((child) => {
    const attribution =
      child.layer !== undefined
        ? { layer: child.layer, share: 1 }
        : dominantLayer(child);
    return {
      key: child.path,
      name: child.name,
      size: child.size,
      node: child,
      slot: slotOf(attribution?.layer ?? null),
      layer: attribution?.layer ?? null,
      share: attribution?.share ?? 0,
    };
  });
  if (rest.length > 0) {
    tiles.push({
      key: node.path ? `${node.path}/…` : "…",
      name: fill(i18nGet("treemapSmaller"), { n: String(rest.length) }),
      size: rest.reduce((sum, c) => sum + c.size, 0),
      node: null,
      slot: 0,
      layer: null,
      share: 0,
    });
  }
  return tiles;
};

/** Path to start at: down through directories that hold nearly all of
 *  their parent. */
const autoOpen = (root: FsNode): string[] => {
  const path: string[] = [];
  let node = root;
  while (node.children && node.size > 0) {
    let biggest: FsNode | null = null;
    for (const child of node.children.values()) {
      if (!biggest || child.size > biggest.size) biggest = child;
    }
    if (!biggest?.children || biggest.size < node.size * TREEMAP_AUTO_OPEN) {
      break;
    }
    path.push(biggest.name);
    node = biggest;
  }
  return path;
};

const tileBox = (r: TreemapRect) => ({
  left: r.x + 1,
  top: r.y + 1,
  width: Math.max(r.w - 2, 0),
  height: Math.max(r.h - 2, 0),
});

/** Where the image's bytes are: the final filesystem as a treemap, two
 *  directory levels at a time. Area is size; color is the layer that wrote
 *  the bytes (the three biggest layers, the rest as "Other"). */
export const ImageTreemap = memo(
  ({
    fileTreeList,
    layers,
  }: {
    fileTreeList: FileTreeList[][];
    layers: Layer[];
  }) => {
    const root = useMemo(() => buildFinalTree(fileTreeList), [fileTreeList]);
    // Colored layers are fixed for the whole image, so a layer keeps its
    // color while drilling in and out.
    const colored = useMemo(() => {
      const totals = [...layerBytes(root).entries()].sort(
        (a, b) => b[1] - a[1],
      );
      return totals.slice(0, TREEMAP_COLORED_LAYERS).map(([layer, bytes]) => ({
        layer,
        bytes,
      }));
    }, [root]);
    const slotOf = useCallback(
      (layer: number | null) => {
        const i = colored.findIndex((c) => c.layer === layer);
        return i >= 0 ? i + 1 : 0;
      },
      [colored],
    );

    const [path, setPath] = useState<string[]>(() => autoOpen(root));
    // A new image starts over (state reset during render, not in an
    // effect, so the old path never paints against the new tree).
    const [pathRoot, setPathRoot] = useState(root);
    if (pathRoot !== root) {
      setPathRoot(root);
      setPath(autoOpen(root));
    }
    const [showTable, setShowTable] = useState(false);
    const [hover, setHover] = useState<{
      tile: TreemapTile;
      x: number;
      y: number;
    } | null>(null);
    const boxRef = useRef<HTMLDivElement>(null);
    const [width, setWidth] = useState(0);
    useLayoutEffect(() => {
      const box = boxRef.current;
      if (!box) return;
      const observer = new ResizeObserver(() => setWidth(box.clientWidth));
      observer.observe(box);
      setWidth(box.clientWidth);
      return () => observer.disconnect();
    }, [showTable]);

    let current = root;
    for (const name of path) {
      const next = current.children?.get(name);
      if (!next || !next.children) break;
      current = next;
    }

    // Each directory big enough gets its own children drawn inside it,
    // under a name strip.
    const groups = useMemo(() => {
      const tiles = childTiles(
        current,
        TREEMAP_MAX_TILES,
        TREEMAP_MIN_SHARE,
        slotOf,
      );
      const rects = squarify(
        tiles.map((t) => t.size),
        width,
        TREEMAP_HEIGHT,
      );
      return tiles.map((tile, i) => {
        const rect = rects[i];
        let children: PlacedTile[] = [];
        if (
          tile.node?.children &&
          rect.w >= TREEMAP_GROUP_MIN_W &&
          rect.h >= TREEMAP_GROUP_MIN_H
        ) {
          const x = rect.x + 3;
          const y = rect.y + TREEMAP_GROUP_HEADER;
          const inner = childTiles(
            tile.node,
            TREEMAP_MAX_CHILD_TILES,
            TREEMAP_CHILD_MIN_SHARE,
            slotOf,
          );
          const innerRects = squarify(
            inner.map((t) => t.size),
            rect.w - 6,
            rect.h - TREEMAP_GROUP_HEADER - 3,
          );
          children = inner.map((t, j) => ({
            tile: t,
            rect: {
              ...innerRects[j],
              x: x + innerRects[j].x,
              y: y + innerRects[j].y,
            },
          }));
        }
        return { tile, rect, children };
      });
    }, [current, slotOf, width]);

    const open = (tile: TreemapTile) => {
      if (tile.node?.children) {
        setPath(tile.node.path.split("/"));
        setHover(null);
      }
    };
    const layerLabel = (tile: TreemapTile) => {
      if (tile.layer === null) return "";
      const n = String(tile.layer + 1);
      return tile.share >= 0.999
        ? fill(i18nGet("treemapLayer"), { layer: n })
        : fill(i18nGet("treemapMostly"), {
            layer: n,
            pct: `${Math.round(tile.share * 100)}%`,
          });
    };
    const percent = (size: number) =>
      current.size > 0 ? `${((size / current.size) * 100).toFixed(1)}%` : "";
    const displayName = (tile: TreemapTile) =>
      tile.node?.children ? `${tile.name}/` : tile.name;
    const hoverProps = (tile: TreemapTile, r: TreemapRect) => ({
      onClick: () => open(tile),
      onPointerMove: (e: PointerEvent<HTMLButtonElement>) => {
        const box = boxRef.current!.getBoundingClientRect();
        setHover({ tile, x: e.clientX - box.left, y: e.clientY - box.top });
      },
      onFocus: () => setHover({ tile, x: r.x + r.w / 2, y: r.y + r.h / 2 }),
      onBlur: () => setHover(null),
      "aria-label": `${displayName(tile)} ${prettyBytes(tile.size)} ${layerLabel(tile)}`,
    });
    const renderTile = ({ tile, rect: r }: PlacedTile) => {
      if (r.w < 1 || r.h < 1) return null;
      const name = displayName(tile);
      const size = prettyBytes(tile.size);
      // Labels only where they fit; the tooltip and the table view carry
      // the rest.
      const room = r.w - 14;
      const showName =
        r.h >= 20 && textWidth(name) * TREEMAP_BOLD_WIDTH <= room;
      const showSize = showName && r.h >= 38 && textWidth(size) <= room;
      return (
        <button
          type="button"
          key={tile.key}
          className={
            tile.node
              ? `treemapTile slot${tile.slot}${tile.node.children ? " dir" : ""}`
              : "treemapTile rest"
          }
          style={tileBox(r)}
          {...hoverProps(tile, r)}
        >
          {showName && <span className="treemapName">{name}</span>}
          {showSize && <span className="treemapSize">{size}</span>}
        </button>
      );
    };

    return (
      <Card
        className="panel span2"
        title={i18nGet("treemapTitle")}
        extra={
          <a
            href="#"
            onClick={(e) => {
              e.preventDefault();
              setShowTable(!showTable);
            }}
          >
            {i18nGet(showTable ? "treemapShowMap" : "treemapShowTable")}
          </a>
        }
      >
        <div className="treemapBar">
          <nav className="treemapCrumbs" aria-label="path">
            <button type="button" onClick={() => setPath([])}>
              /
            </button>
            {path.map((name, i) => (
              <button
                type="button"
                key={path.slice(0, i + 1).join("/")}
                onClick={() => setPath(path.slice(0, i + 1))}
              >
                {name}/
              </button>
            ))}
            <span className="treemapTotal">{prettyBytes(current.size)}</span>
          </nav>
          <ul className="treemapLegend">
            {colored.map((c, i) => (
              <li key={c.layer}>
                <span className={`treemapSwatch slot${i + 1}`} />
                {fill(i18nGet("treemapLayer"), { layer: String(c.layer + 1) })}
                <span className="treemapLegendSize">
                  {prettyBytes(c.bytes)}
                </span>
              </li>
            ))}
            {layers.filter((l) => !l.empty).length > colored.length && (
              <li>
                <span className="treemapSwatch slot0" />
                {i18nGet("treemapOther")}
              </li>
            )}
          </ul>
        </div>
        {showTable ? (
          <table className="treemapTable">
            <tbody>
              {groups.map(({ tile }) => (
                <tr key={tile.key}>
                  <td>
                    {tile.node?.children ? (
                      <a
                        href="#"
                        onClick={(e) => {
                          e.preventDefault();
                          open(tile);
                        }}
                      >
                        {displayName(tile)}
                      </a>
                    ) : (
                      tile.name
                    )}
                  </td>
                  <td>{prettyBytes(tile.size)}</td>
                  <td>{percent(tile.size)}</td>
                  <td>{layerLabel(tile)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <div
            className="treemap"
            ref={boxRef}
            style={{ height: TREEMAP_HEIGHT }}
            onPointerLeave={() => setHover(null)}
          >
            {groups.map((group) => {
              if (group.children.length === 0) return renderTile(group);
              const { tile, rect: r } = group;
              const name = displayName(tile);
              const size = prettyBytes(tile.size);
              const room = r.w - 14;
              const nameWidth = textWidth(name) * TREEMAP_BOLD_WIDTH;
              const showName = nameWidth <= room;
              const showSize = nameWidth + 6 + textWidth(size) <= room;
              return (
                <Fragment key={tile.key}>
                  <button
                    type="button"
                    className="treemapGroup"
                    style={tileBox(r)}
                    {...hoverProps(tile, r)}
                  >
                    {showName && <span className="treemapName">{name}</span>}
                    {showSize && <span className="treemapSize">{size}</span>}
                  </button>
                  {group.children.map(renderTile)}
                </Fragment>
              );
            })}
            {hover && (
              <div
                className="treemapTooltip"
                role="tooltip"
                style={{
                  left: Math.min(hover.x + 14, Math.max(width - 260, 0)),
                  top: Math.min(hover.y + 14, TREEMAP_HEIGHT - 90),
                }}
              >
                <strong>{prettyBytes(hover.tile.size)}</strong>
                <span> · {percent(hover.tile.size)}</span>
                <div className="treemapTooltipPath">
                  /{hover.tile.key}
                  {hover.tile.node?.children ? "/" : ""}
                </div>
                {hover.tile.layer !== null && (
                  <div className="treemapTooltipLayer">
                    <span className={`treemapKey slot${hover.tile.slot}`} />
                    {layerLabel(hover.tile)}
                    {layers[hover.tile.layer]?.cmd
                      ? ` — ${layers[hover.tile.layer].cmd.slice(0, 80)}`
                      : ""}
                  </div>
                )}
              </div>
            )}
          </div>
        )}
        <p className="treemapHint">{i18nGet("treemapHint")}</p>
      </Card>
    );
  },
);

export const EXAMPLE_IMAGES = [
  "redis:alpine",
  "nginx:alpine",
  "vicanso/diving",
  "quay.io/prometheus/node-exporter",
];

// ---- Compact KPI Strip ----

export const KpiStrip = memo(
  ({ desc, tags }: { desc: ImageDescriptions; tags?: string[] }) => {
    const scoreOnly = (desc.score || "0").replace("%", "");
    const tone = scoreTone(scoreOnly);
    return (
      <section className="kpiStrip" aria-label={i18nGet("imageSummaryTitle")}>
        <div className="kpiItem">
          <span className="kpiLabel">{i18nGet("imageScoreLabel")}</span>
          <span className={`kpiValue kpiScore ${tone}`.trim()}>
            {desc.score}
          </span>
        </div>
        <span className="kpiDivider" aria-hidden="true" />
        <div className="kpiItem">
          <span className="kpiLabel">{i18nGet("imageSizeLabel")}</span>
          <span className="kpiValue mono">{desc.size}</span>
        </div>
        <span className="kpiDivider" aria-hidden="true" />
        <div className="kpiItem">
          <span className="kpiLabel">{i18nGet("wastedSizeLabel")}</span>
          <span
            className={`kpiValue mono ${desc.wastedSize !== "0 B" ? "warn" : ""}`}
          >
            {desc.wastedSize}
          </span>
        </div>
        <span className="kpiDivider" aria-hidden="true" />
        <div className="kpiItem">
          <span className="kpiLabel">{i18nGet("osArchLabel")}</span>
          <span className="kpiValue mono">{desc.osArch}</span>
        </div>
        {desc.baseOs && (
          <>
            <span className="kpiDivider" aria-hidden="true" />
            <div className="kpiItem">
              <span className="kpiLabel">{i18nGet("baseOsLabel")}</span>
              <span className="kpiValue">{desc.baseOs}</span>
            </div>
          </>
        )}
        {desc.runUser && (
          <>
            <span className="kpiDivider" aria-hidden="true" />
            <div className="kpiItem">
              <span className="kpiLabel">{i18nGet("runAsUserLabel")}</span>
              <span className="kpiValue mono">{desc.runUser}</span>
            </div>
          </>
        )}
        {desc.runtimeLibc && (
          <>
            <span className="kpiDivider" aria-hidden="true" />
            <div className="kpiItem">
              <span className="kpiLabel">{i18nGet("runtimeLibcLabel")}</span>
              <span
                className={`kpiValue ${desc.runtimeLibcIssue ? "danger" : ""}`}
              >
                {desc.runtimeLibc}
              </span>
            </div>
          </>
        )}
        {tags && tags.length > 0 && (
          <div className="kpiTags">
            {tags.map((t) => (
              <span key={t} className="riskTag">
                {t}
              </span>
            ))}
          </div>
        )}
      </section>
    );
  },
);

// ---- Master Pane: Persistent Layer Timeline ----

type LayerFilterMode = "all" | "files" | "large";

const opcodeClass = (op: LayerOpcode) => {
  switch (op) {
    case "FROM":
      return "op-from";
    case "RUN":
      return "op-run";
    case "COPY":
    case "ADD":
      return "op-copy";
    case "CMD":
    case "ENTRY":
      return "op-cmd";
    default:
      return "op-meta";
  }
};

export const LayerTimeline = memo(
  ({
    layers,
    fileTreeList,
    currentLayer,
    onSelectLayer,
  }: {
    layers: Layer[];
    fileTreeList: FileTreeList[][];
    currentLayer: number;
    onSelectLayer: (index: number) => void;
  }) => {
    const [filter, setFilter] = useState<LayerFilterMode>("all");
    const activeRef = useRef<HTMLButtonElement | null>(null);

    useEffect(() => {
      activeRef.current?.scrollIntoView({
        block: "nearest",
        behavior: "smooth",
      });
    }, [currentLayer]);

    const parsedLayers = useMemo(() => {
      let maxSize = 0;
      let totalCompressed = 0;
      for (const l of layers) {
        if (l.size > maxSize) maxSize = l.size;
        totalCompressed += l.size || 0;
      }
      return {
        maxSize,
        totalCompressed,
        items: layers.map((layer, index) => {
          const parsed = parseLayerCommand(layer.cmd, index, layer.size);
          const hasFiles = (fileTreeList[index]?.length || 0) > 0;
          const isLarge = (layer.size || 0) >= 1024 * 1024;
          return { layer, index, parsed, hasFiles, isLarge };
        }),
      };
    }, [layers, fileTreeList]);

    const counts = useMemo(() => {
      let files = 0;
      let large = 0;
      for (const item of parsedLayers.items) {
        if (item.hasFiles) files++;
        if (item.isLarge) large++;
      }
      return { all: parsedLayers.items.length, files, large };
    }, [parsedLayers]);

    const visibleItems = useMemo(() => {
      if (filter === "files") {
        return parsedLayers.items.filter((i) => i.hasFiles);
      }
      if (filter === "large") {
        return parsedLayers.items.filter((i) => i.isLarge);
      }
      return parsedLayers.items;
    }, [parsedLayers, filter]);

    return (
      <section className="layerTimeline" aria-label={i18nGet("layersTitle")}>
        <div className="layerTimelineHeader">
          <div className="layerTimelineTitleRow">
            <span className="layerTimelineTitle">{i18nGet("layersTitle")}</span>
            <span className="layerTimelineMeta">
              {layers.length} · {prettyBytes(parsedLayers.totalCompressed)}
            </span>
          </div>
          <div className="layerFilterPills" role="group">
            <button
              type="button"
              className={`filterPill ${filter === "all" ? "active" : ""}`}
              onClick={() => setFilter("all")}
            >
              {i18nGet("filterAll")} ({counts.all})
            </button>
            <button
              type="button"
              className={`filterPill ${filter === "files" ? "active" : ""}`}
              onClick={() => setFilter("files")}
            >
              {i18nGet("filterHasFiles")} ({counts.files})
            </button>
            <button
              type="button"
              className={`filterPill ${filter === "large" ? "active" : ""}`}
              onClick={() => setFilter("large")}
            >
              {i18nGet("filterLarge")} ({counts.large})
            </button>
          </div>
        </div>
        <div className="layerTimelineList" role="listbox">
          {visibleItems.map(({ layer, index, parsed }) => {
            const isActive = index === currentLayer;
            const pct =
              parsedLayers.maxSize > 0
                ? Math.round(((layer.size || 0) / parsedLayers.maxSize) * 100)
                : 0;
            const idxStr = String(index + 1).padStart(2, "0");
            return (
              <button
                key={`${index}-${layer.digest || "meta"}`}
                ref={isActive ? activeRef : undefined}
                type="button"
                role="option"
                aria-selected={isActive}
                className={`layerTimelineRow ${isActive ? "active" : ""} ${
                  layer.size === 0 ? "isMeta" : ""
                }`}
                onClick={() => onSelectLayer(index)}
              >
                <span className="ltIndex">{idxStr}</span>
                <span className={`opcodePill ${opcodeClass(parsed.opcode)}`}>
                  {parsed.opcode}
                </span>
                <span className="ltSize">
                  {layer.size > 0 ? prettyBytes(layer.size) : "0 B"}
                </span>
                <span className="ltBarTrack" aria-hidden="true">
                  <span
                    className="ltBarFill"
                    style={{ width: `${pct}%` }}
                  />
                </span>
                <span className="ltCmd" title={parsed.summary}>
                  {parsed.summary}
                </span>
              </button>
            );
          })}
        </div>
      </section>
    );
  },
);

// ---- Command Inspector & Layer Metadata ----

export const CommandInspector = memo(
  ({
    layer,
    layerIndex,
    fileCounts,
  }: {
    layer: Layer;
    layerIndex: number;
    fileCounts: LayerFileCounts;
  }) => {
    const [expanded, setExpanded] = useState(false);
    const [wrap, setWrap] = useState(false);
    const [copiedCmd, setCopiedCmd] = useState(false);
    const [copiedDigest, setCopiedDigest] = useState(false);

    useEffect(() => {
      setExpanded(false);
      setCopiedCmd(false);
      setCopiedDigest(false);
    }, [layerIndex, layer.digest]);

    const parsed = useMemo(
      () => parseLayerCommand(layer.cmd, layerIndex, layer.size),
      [layer.cmd, layerIndex, layer.size],
    );

    const codeLines = useMemo(() => {
      const lines: string[] = [];
      if (parsed.envArgs.length > 0) {
        lines.push(`# BuildKit args: ${parsed.envArgs.join(" ")}`);
      }
      if (parsed.formatted) {
        lines.push(...parsed.formatted.split("\n"));
      }
      return lines;
    }, [parsed]);

    const canCollapse = codeLines.length > 6;
    const visibleLines =
      canCollapse && !expanded ? codeLines.slice(0, 6) : codeLines;
    const hiddenCount = codeLines.length - 6;

    const ratio =
      layer.size > 0 && layer.unpackSize > 0
        ? (layer.unpackSize / layer.size).toFixed(2)
        : null;

    const copyText = (text: string, which: "cmd" | "digest") => {
      navigator.clipboard?.writeText(text).then(() => {
        if (which === "cmd") {
          setCopiedCmd(true);
          window.setTimeout(() => setCopiedCmd(false), 1500);
        } else {
          setCopiedDigest(true);
          window.setTimeout(() => setCopiedDigest(false), 1500);
        }
      });
    };

    const shortDigest = layer.digest
      ? `${layer.digest.slice(0, 19)}…`
      : "—";

    return (
      <div className="commandInspector">
        <div className="layerMetaStrip">
          <span className="layerMetaTitle">
            #{layerIndex + 1}
            <span className={`opcodePill ${opcodeClass(parsed.opcode)}`}>
              {parsed.opcode}
            </span>
          </span>
          <span className="layerMetaChip">
            <strong>{prettyBytes(layer.size || 0)}</strong>{" "}
            {i18nGet("compressedLabel")}
          </span>
          {layer.unpackSize > 0 && (
            <span className="layerMetaChip">
              <strong>{prettyBytes(layer.unpackSize)}</strong>{" "}
              {i18nGet("unpackedLabel")}
            </span>
          )}
          {ratio && (
            <span className="layerMetaChip">
              <strong>{ratio}×</strong>
            </span>
          )}
          <span className="layerMetaChip">
            <strong>{fileCounts.total}</strong> {i18nGet("filesCountLabel")}
            {fileCounts.total > 0 && (
              <span className="fileCountBreakdown">
                {" "}
                (+{fileCounts.added} / ~{fileCounts.modified} / -
                {fileCounts.removed})
              </span>
            )}
          </span>
          <span className="layerMetaRight">
            {layer.digest && (
              <button
                type="button"
                className="metaCopyBtn"
                onClick={() => copyText(layer.digest, "digest")}
                title={layer.digest}
              >
                {shortDigest}{" "}
                <span className="metaCopyTag">
                  {copiedDigest ? i18nGet("copiedButton") : i18nGet("copyButton")}
                </span>
              </button>
            )}
            {layer.created && (
              <span className="layerCreated">
                {new Date(layer.created).toLocaleString()}
              </span>
            )}
          </span>
        </div>

        {codeLines.length > 0 && (
          <div className="commandBox">
            <div className="commandBoxToolbar">
              <span className="commandBoxLabel">
                {i18nGet("commandLabel")}
              </span>
              <div className="commandBoxActions">
                <button
                  type="button"
                  className={`cmdActionBtn ${wrap ? "active" : ""}`}
                  onClick={() => setWrap((w) => !w)}
                >
                  {i18nGet("wrapLinesLabel")}
                </button>
                <button
                  type="button"
                  className="cmdActionBtn"
                  onClick={() => copyText(codeLines.join("\n"), "cmd")}
                >
                  {copiedCmd ? i18nGet("copiedButton") : i18nGet("copyButton")}
                </button>
              </div>
            </div>
            <pre className={`commandPre ${wrap ? "wrap" : ""}`}>
              {visibleLines.map((line, idx) => (
                <div
                  key={idx}
                  className={`cmdLine ${
                    line.startsWith("#") ? "isComment" : ""
                  }`}
                >
                  <span className="cmdLineNo">{idx + 1}</span>
                  <span className="cmdLineCode">{line}</span>
                </div>
              ))}
            </pre>
            {canCollapse && (
              <button
                type="button"
                className="cmdExpandBtn"
                onClick={() => setExpanded((e) => !e)}
              >
                {expanded
                  ? i18nGet("showLessLines")
                  : fill(i18nGet("showMoreLines"), {
                      n: String(hiddenCount),
                    })}
              </button>
            )}
          </div>
        )}
      </div>
    );
  },
);
