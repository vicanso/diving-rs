// Presentational components. Report cards are memoized so filter typing
// in App does not re-render them.

import { memo, useCallback, useEffect, useRef, useState } from "react";
import { Card, Input, List, Select, Typography } from "antd";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import { FileTreeRow, opModified, opRemoved } from "./analysis";
import { getDownloadIcon, minusOutlined, plusOutlined } from "./icons";
import {
  DuplicateGroup,
  ImageDescriptions,
  Layer,
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
                {!row.isDir && row.size > 0 && (
                  <a
                    className="download"
                    href={`./api/file?digest=${encodeURIComponent(
                      layer.digest,
                    )}&mediaType=${encodeURIComponent(
                      layer.mediaType,
                    )}&file=${encodeURIComponent(row.key)}`}
                  >
                    {getDownloadIcon()}
                  </a>
                )}
              </span>
            </li>
          );
        })}
      </ul>
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

export const EXAMPLE_IMAGES = [
  "redis:alpine",
  "nginx:alpine",
  "vicanso/diving",
  "quay.io/prometheus/node-exporter",
];
