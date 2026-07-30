// Presentational components. All report cards are memoized so that typing
// in the keyword filter (which re-renders App) doesn't re-render them.

import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  Card,
  Descriptions,
  Input,
  List,
  Select,
  Space,
  Typography,
} from "antd";
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
const FILE_TREE_VIEWPORT_HEIGHT = 480;
const FILE_TREE_OVERSCAN = 8;

/** Windowed file-tree list — only mounts rows near the scroll viewport. */
export const VirtualFileTree = ({
  rows,
  layer,
  onToggleExpand,
  isDark,
}: {
  rows: FileTreeRow[];
  layer: Layer;
  onToggleExpand: (key: string) => void;
  isDark: boolean;
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

  const totalHeight = Math.max(
    rows.length * FILE_TREE_ROW_HEIGHT,
    FILE_TREE_ROW_HEIGHT,
  );
  const start = Math.max(
    0,
    Math.floor(scrollTop / FILE_TREE_ROW_HEIGHT) - FILE_TREE_OVERSCAN,
  );
  const visibleCount =
    Math.ceil(FILE_TREE_VIEWPORT_HEIGHT / FILE_TREE_ROW_HEIGHT) +
    FILE_TREE_OVERSCAN * 2;
  const end = Math.min(rows.length, start + visibleCount);
  const slice = rows.slice(start, end);

  let className = "fileTree virtualFileTree";
  if (isDark) {
    className += " dark";
  }

  return (
    <div
      className="virtualFileTreeScroller"
      ref={scrollerRef}
      onScroll={onScroll}
      style={{ height: FILE_TREE_VIEWPORT_HEIGHT }}
    >
      <ul
        className={className}
        style={{
          height: totalHeight,
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
              <span className={opClass} style={{ paddingLeft: row.depth * 30 }}>
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
}: {
  arch: string;
  defaultImage: string;
  loading: boolean;
  onArchChange: (arch: string) => void;
  onSearch: (image: string) => void;
}) => {
  const size = "large" as const;
  const selectBefore = (
    <Select
      size={size}
      defaultValue={arch}
      style={{
        width: "100px",
      }}
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

export const ImageSummaryCard = memo(
  ({ desc }: { desc: ImageDescriptions }) => {
    return (
      <div className="imageSummary mtop30">
        <Descriptions title={i18nGet("imageSummaryTitle")}>
          <Descriptions.Item label={i18nGet("imageScoreLabel")}>
            {desc.score}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("imageSizeLabel")}>
            {desc.size}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("otherLayerSizeLabel")}>
            {desc.otherSize}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("wastedSizeLabel")}>
            {desc.wastedSize}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("osArchLabel")}>
            {desc.osArch}
          </Descriptions.Item>
          {desc.runUser && (
            <Descriptions.Item label={i18nGet("runAsUserLabel")}>
              {desc.runUser}
            </Descriptions.Item>
          )}
          {desc.baseOs && (
            <Descriptions.Item label={i18nGet("baseOsLabel")}>
              {desc.baseOs}
            </Descriptions.Item>
          )}
          {desc.runtimeLibc && (
            <Descriptions.Item label={i18nGet("runtimeLibcLabel")}>
              <span
                style={{
                  color: desc.runtimeLibcIssue ? "#cf1322" : undefined,
                }}
              >
                {desc.runtimeLibc}
              </span>
            </Descriptions.Item>
          )}
          <Descriptions.Item label={i18nGet("createdLabel")}>
            {new Date(desc.created).toLocaleString()}
          </Descriptions.Item>
        </Descriptions>
      </div>
    );
  },
);

export const TagsCard = memo(({ tags }: { tags: string[] }) => {
  if (!tags || tags.length === 0) {
    return <></>;
  }
  return (
    <div className="mtop30">
      <Card title={i18nGet("tagsTitle")}>
        <Space wrap>
          {tags.map((t) => (
            <span key={t} className="riskTag">
              {t}
            </span>
          ))}
        </Space>
      </Card>
    </div>
  );
});

export const WastedSummaryCard = memo(
  ({
    wastedList,
    isDark,
  }: {
    wastedList: FileWastedSummary[];
    isDark: boolean;
  }) => {
    const arr = wastedList.filter((item) => item.totalSize > 0);
    if (arr.length === 0) {
      return <></>;
    }
    const list = arr.map((item) => {
      return (
        <li key={item.path}>
          <span>{prettyBytes(item.totalSize)}</span>
          <span>{item.count}</span>
          <span>/{item.path}</span>
        </li>
      );
    });
    let className = "wastedList";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("wastedSummaryTitle")}>
          <ul className={className}>
            <li>
              <span>{i18nGet("totalSizeLabel")}</span>
              <span>{i18nGet("countLabel")}</span>
              <span>{i18nGet("pathLabel")}</span>
            </li>
            {list}
          </ul>
        </Card>
      </div>
    );
  },
);

export const SensitiveFilesCard = memo(
  ({ files, isDark }: { files: SensitiveFile[]; isDark: boolean }) => {
    if (!files || files.length === 0) {
      return <></>;
    }
    let className = "sensitiveList";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("sensitiveFilesTitle")}>
          <ul className={className}>
            <li>
              <span>{i18nGet("layerLabel")}</span>
              <span>{i18nGet("sizeLabel")}</span>
              <span>{i18nGet("sensitiveReasonLabel")}</span>
              <span>{i18nGet("pathLabel")}</span>
            </li>
            {files.map((f) => (
              <li key={`${f.layerIndex}-${f.path}`}>
                <span>{f.layerIndex}</span>
                <span>{prettyBytes(f.size || 0)}</span>
                <span>{f.reason}</span>
                <span>/{f.path}</span>
              </li>
            ))}
          </ul>
        </Card>
      </div>
    );
  },
);

export const DuplicateGroupsCard = memo(
  ({ groups, isDark }: { groups: DuplicateGroup[]; isDark: boolean }) => {
    if (!groups || groups.length === 0) {
      return <></>;
    }
    const sorted = [...groups].sort((a, b) => b.totalWasted - a.totalWasted);
    let className = "duplicateList";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("duplicateGroupsTitle")}>
          <ul className={className}>
            <li>
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
                  <span>{prettyBytes(g.totalWasted)}</span>
                  <span>{g.count}</span>
                  <span>{prettyBytes(g.size)}</span>
                  <span title={g.hash}>{sample}</span>
                </li>
              );
            })}
          </ul>
        </Card>
      </div>
    );
  },
);

export const BigModifiedFilesCard = memo(
  ({ files, isDark }: { files: ModifiedFile[]; isDark: boolean }) => {
    if (files.length === 0) {
      return <></>;
    }
    const arr = files.slice(0);
    arr.sort((item1, item2) => {
      return item2.size - item1.size;
    });
    const list = arr.map((item) => {
      let { digest } = item;
      if (digest) {
        digest = digest.replace("sha256:", "").substring(0, 8);
      }
      return (
        <li key={item.path}>
          <span>{digest.toUpperCase()}</span>
          <span>{prettyBytes(item.size)}</span>
          <span>/{item.path}</span>
        </li>
      );
    });
    let className = "bigModifiedFileList";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("modifiedAddedLargeFileTitle")}>
          <ul className={className}>
            <li>
              <span>{i18nGet("layerLabel")}</span>
              <span>{i18nGet("totalSizeLabel")}</span>
              <span>{i18nGet("pathLabel")}</span>
            </li>
            {list}
          </ul>
        </Card>
      </div>
    );
  },
);

export const RecommendationsCard = memo(
  ({
    recommendations,
    isDark,
  }: {
    recommendations: Recommendation[];
    isDark: boolean;
  }) => {
    if (!recommendations || recommendations.length === 0) {
      return <></>;
    }
    const severityColor: Record<string, string> = {
      high: "#cf1322",
      medium: "#d46b08",
      low: "#d4b106",
      info: "#0958d9",
    };
    const list = recommendations.map((r, idx) => {
      const color = severityColor[r.severity] || "#0958d9";
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
    });
    let className = "recommendationList";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("recommendationsTitle")}>
          <ul className={className}>{list}</ul>
        </Card>
      </div>
    );
  },
);

export const DockerfileCard = memo(
  ({ dockerfile, isDark }: { dockerfile: string; isDark: boolean }) => {
    if (!dockerfile) {
      return <></>;
    }
    let className = "dockerfileBlock";
    if (isDark) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("dockerfileTitle")}>
          <pre className={className}>{dockerfile}</pre>
        </Card>
      </div>
    );
  },
);

export const LatestImagesList = memo(({ images }: { images: string[] }) => {
  if (images.length === 0) {
    return <></>;
  }
  return (
    <List
      className="analyzeImages"
      bordered={true}
      size={"small"}
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
          </Typography.Text>{" "}
        </List.Item>
      )}
    />
  );
});
