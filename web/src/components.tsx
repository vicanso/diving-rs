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
  FsNode,
  layerBytes,
  opModified,
  opRemoved,
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
  Layer,
  McpStatus,
  ModifiedFile,
  Recommendation,
  SensitiveFile,
  FileWastedSummary,
} from "./types";

const { Option } = Select;
const { Search } = Input;

const FILE_TREE_ROW_HEIGHT = 36;
/** Cap for the scroller; content shorter than this shrinks the panel. */
const FILE_TREE_MAX_HEIGHT = 480;
const FILE_TREE_OVERSCAN = 8;

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
}: {
  rows: FileTreeRow[];
  layer: Layer;
  onToggleExpand: (key: string) => void;
  isDark?: boolean;
}) => {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [previewRow, setPreviewRow] = useState<FileTreeRow | null>(null);

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

  // Empty list still shows one row of breathing room.
  const contentHeight = Math.max(
    rows.length * FILE_TREE_ROW_HEIGHT,
    FILE_TREE_ROW_HEIGHT,
  );
  // Grow with content until the max; only then scroll.
  const viewportHeight = Math.min(contentHeight, FILE_TREE_MAX_HEIGHT);
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
        maxHeight: FILE_TREE_MAX_HEIGHT,
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
          let name = row.name;
          if (row.link) {
            name = `${name} → ${row.link}`;
          }
          return (
            <li
              key={row.key}
              className="virtualFileTreeRow"
              style={{
                position: "absolute",
                top: rowTop,
                left: 0,
                right: 0,
                height: FILE_TREE_ROW_HEIGHT,
              }}
            >
              <span>{row.mode}</span>
              <span>
                {row.uid}:{row.gid}
              </span>
              <span>{prettyBytes(row.size)}</span>
              <span className={opClass} style={{ paddingLeft: row.depth * 16 }}>
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
                {name}
                {!row.isDir && row.size > 0 && row.op !== opRemoved && (
                  <>
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
                      href={fileUrl("file", layer, row.key)}
                    >
                      {getDownloadIcon()}
                    </a>
                  </>
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
  defaultImage,
  loading,
  onArchChange,
  onSearch,
  size = "large",
}: {
  arch: string;
  defaultImage: string;
  loading: boolean;
  onArchChange: (arch: string) => void;
  onSearch: (image: string) => void;
  size?: "large" | "middle";
}) => {
  const selectBefore = (
    <Select
      size={size}
      defaultValue={arch}
      style={{ width: size === "large" ? 108 : 96 }}
      onChange={onArchChange}
    >
      <Option value="amd64">AMD64</Option>
      <Option value="arm64">ARM64</Option>
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
      onSearch={onSearch}
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

export const WastedSummaryCard = memo(
  ({ wastedList }: { wastedList: FileWastedSummary[]; isDark?: boolean }) => {
    const arr = wastedList.filter((item) => item.totalSize > 0);
    if (arr.length === 0) {
      return null;
    }
    return (
      <Card className="panel" title={i18nGet("wastedSummaryTitle")}>
        <ul className="dataTable cols-wasted" style={{ margin: "-16px -18px" }}>
          <li className="head">
            <span>{i18nGet("totalSizeLabel")}</span>
            <span>{i18nGet("countLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {arr.map((item) => (
            <li key={item.path}>
              <span className="monoCell">{prettyBytes(item.totalSize)}</span>
              <span className="monoCell">{item.count}</span>
              <span className="monoCell">/{item.path}</span>
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
      <Card className="panel" title={i18nGet("sensitiveFilesTitle")}>
        <ul
          className="dataTable cols-sensitive"
          style={{ margin: "-16px -18px" }}
        >
          <li className="head">
            <span>{i18nGet("layerLabel")}</span>
            <span>{i18nGet("sizeLabel")}</span>
            <span>{i18nGet("sensitiveReasonLabel")}</span>
            <span>{i18nGet("pathLabel")}</span>
          </li>
          {files.map((f) => (
            <li key={`${f.layerIndex}-${f.path}`}>
              <span className="monoCell">{f.layerIndex}</span>
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
      <Card className="panel" title={i18nGet("duplicateGroupsTitle")}>
        <ul className="dataTable cols-dup" style={{ margin: "-16px -18px" }}>
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
                    .map((p) => `L${p.layerIndex}:/${p.path}`)
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
      <Card className="panel" title={i18nGet("modifiedAddedLargeFileTitle")}>
        <ul className="dataTable cols-bigmod" style={{ margin: "-16px -18px" }}>
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
      <Card className="panel span2" title={i18nGet("recommendationsTitle")}>
        <ul className="recommendationList" style={{ margin: "-16px -18px" }}>
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
    if (!dockerfile) {
      return null;
    }
    return (
      <Card className="panel span2" title={i18nGet("dockerfileTitle")}>
        <pre className="dockerfileBlock">{dockerfile}</pre>
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
const TREEMAP_MAX_TILES = 40;
/** Tiles drawn inside each nested directory. */
const TREEMAP_MAX_CHILD_TILES = 24;
/** Layers that get their own color; every other layer is "Other". Three is
 *  the most the categorical palette keeps distinguishable when any two
 *  colors may touch, as tiles in a treemap can. */
const TREEMAP_COLORED_LAYERS = 3;
const TREEMAP_HEIGHT = 360;
/** Name strip on top of a directory whose children are drawn inside it. */
const TREEMAP_GROUP_HEADER = 20;
/** Room a directory needs before its children are drawn inside it. */
const TREEMAP_GROUP_MIN = 56;
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

const fill = (template: string, values: Record<string, string>) =>
  Object.entries(values).reduce(
    (text, [key, value]) => text.replace(`{${key}}`, value),
    template,
  );

/** `node`'s children as tiles, largest first, the tail past `max` merged. */
const childTiles = (
  node: FsNode,
  max: number,
  slotOf: (layer: number | null) => number,
): TreemapTile[] => {
  const children = [...(node.children?.values() || [])]
    .filter((c) => c.size > 0)
    .sort((a, b) => b.size - a.size);
  const tiles: TreemapTile[] = children.slice(0, max).map((child) => {
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
  const rest = children.slice(max);
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
      const tiles = childTiles(current, TREEMAP_MAX_TILES, slotOf);
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
          rect.w >= TREEMAP_GROUP_MIN &&
          rect.h >= TREEMAP_GROUP_MIN
        ) {
          const x = rect.x + 3;
          const y = rect.y + TREEMAP_GROUP_HEADER;
          const inner = childTiles(tile.node, TREEMAP_MAX_CHILD_TILES, slotOf);
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
