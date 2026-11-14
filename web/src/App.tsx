import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Alert,
  Card,
  Checkbox,
  ConfigProvider,
  Input,
  InputRef,
  Layout,
  message,
  Segmented,
  Select,
  Tabs,
  theme,
} from "antd";
import axios, { AxiosError } from "axios";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  addKeyToFileTreeItem,
  countLayerFiles,
  flattenFileTree,
  getImageSummary,
  FileTreeRow,
  FileTreeViewOption,
} from "./analysis";
import {
  BigModifiedFilesCard,
  CommandInspector,
  DockerfileCard,
  DuplicateGroupsCard,
  EXAMPLE_IMAGES,
  ImagePreviewCard,
  ImageTreemap,
  KpiStrip,
  LatestImagesList,
  LayerTimeline,
  McpGuide,
  RecommendationsCard,
  SearchBar,
  SensitiveFilesCard,
  VirtualFileTree,
  WastedSummaryCard,
} from "./components";
import { CompareButton, DiffRefs, DiffView } from "./diff";
import { getGithubIcon, getLogoIcon, getThemeIcon } from "./icons";
import {
  DuplicateGroup,
  FileTreeList,
  FileWastedSummary,
  ImageAnalyzeResult,
  ImageDescriptions,
  ImageDiff,
  ImagePreview,
  LatestImages,
  Layer,
  McpStatus,
  ModifiedFile,
  Recommendation,
  SensitiveFile,
} from "./types";

import "./App.css";

const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;

const amd64Arch = "amd64";
const request = axios.create({
  timeout: 600 * 1000,
  baseURL: "./api",
});

type ThemeMode = "system" | "light" | "dark";
const THEME_STORAGE_KEY = "diving-theme";

const useThemeMode = () => {
  const [mode, setMode] = useState<ThemeMode>(() => {
    try {
      const saved = localStorage.getItem(THEME_STORAGE_KEY);
      if (saved === "light" || saved === "dark" || saved === "system") {
        return saved;
      }
    } catch {
      /* ignore storage errors */
    }
    return "system";
  });
  const [systemDark, setSystemDark] = useState(() =>
    window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const isDark = mode === "system" ? systemDark : mode === "dark";

  useEffect(() => {
    document.documentElement.classList.toggle("dark", isDark);
  }, [isDark]);

  const cycleMode = useCallback(() => {
    setMode((prev) => {
      const next: ThemeMode =
        prev === "system" ? "light" : prev === "light" ? "dark" : "system";
      try {
        localStorage.setItem(THEME_STORAGE_KEY, next);
      } catch {
        /* ignore */
      }
      return next;
    });
  }, []);

  return { mode, isDark, cycleMode };
};

interface ReportState {
  imageDescriptions: ImageDescriptions;
  wastedList: FileWastedSummary[];
  fileTreeList: FileTreeList[][];
  layers: Layer[];
  bigModifiedFileList: ModifiedFile[];
  recommendations: Recommendation[];
  tags: string[];
  sensitiveFiles: SensitiveFile[];
  duplicateGroups: DuplicateGroup[];
  dockerfile: string;
  staleAsOf: string;
  supportedArchs: string[];
}

const readQuery = () => {
  const urlInfo = new URL(window.location.href);
  const image = urlInfo.searchParams.get("image") || "";
  const base = urlInfo.searchParams.get("base") || "";
  const target = urlInfo.searchParams.get("target") || "";
  // Any platform `?arch=` takes: `arm64`, `arm/v7`, `s390x`, …
  const requested = (urlInfo.searchParams.get("arch") || "").toLowerCase();
  const arch = /^[a-z0-9_]+(\/[a-z0-9]+)?$/.test(requested)
    ? requested
    : amd64Arch;
  return { image, arch, base, target };
};

/** How a navigation touches the browser history: a user action adds an
 *  entry, the first load only normalizes the URL, and back / forward has
 *  already moved it. */
type HistoryMode = "push" | "replace" | "none";

const setLocation = (search: string, mode: HistoryMode) => {
  if (mode === "none" || window.location.search === search) {
    return;
  }
  if (mode === "push") {
    window.history.pushState(null, "", `./${search}`);
  } else {
    window.history.replaceState(null, "", `./${search}`);
  }
};

// `docker://` / `podman://` name one local image; a registry image or an
// OCI archive / layout may hold several platforms. A `?arch=` already in
// the reference wins.
const withArch = (image: string, arch: string) =>
  /^(docker|podman):\/\//.test(image) || image.includes("?arch=") || !arch
    ? image
    : `${image}?arch=${arch}`;

/** A first analysis that has not finished by then shows the layer list
 *  from the manifest; a cached one is back sooner and never flashes it. */
const PREVIEW_DELAY_MS = 500;

const showRequestError = (err: unknown, fallback: string) => {
  let msg = (err as Error)?.message as string;
  const axiosErr = err as AxiosError;
  if (axiosErr?.response?.data) {
    const data = axiosErr.response.data as {
      message: string;
    };
    msg = data.message || "";
  }
  message.error(msg || fallback, 10);
};

const App = () => {
  const { mode: themeMode, isDark, cycleMode: cycleTheme } = useThemeMode();
  const [initial] = useState(readQuery);
  const [imageName, setImageName] = useState(initial.image);
  const [arch, setArch] = useState(initial.arch);
  const [loading, setLoading] = useState(false);
  const [report, setReport] = useState<ReportState | null>(null);
  const [currentLayer, setCurrentLayer] = useState(0);
  const [viewOption, setViewOption] = useState<FileTreeViewOption>({
    expandAll: false,
    expandItems: [],
    collapsedItems: [],
    sizeLimit: 0,
    onlyModifiedRemoved: false,
    keyword: "",
    compactPaths: true,
  });
  const [showMetaColumns, setShowMetaColumns] = useState(false);
  const [mobilePane, setMobilePane] = useState<"layers" | "workspace">(
    "workspace",
  );
  const fileSearchRef = useRef<InputRef>(null);
  const [latestImages, setLatestImages] = useState<string[]>([]);
  const [version, setVersion] = useState("");
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [diff, setDiff] = useState<{ refs: DiffRefs; data: ImageDiff } | null>(
    null,
  );
  const [preview, setPreview] = useState<ImagePreview | null>(null);

  // Every request takes a ticket; a response whose ticket is no longer the
  // latest — the user moved on, e.g. with the back button — is dropped.
  const ticket = useRef(0);

  const onSearch = async (
    value: string,
    history: HistoryMode = "push",
    archValue = arch,
  ) => {
    const image = value.trim();
    if (!image) {
      return;
    }
    setLocation(
      `?image=${encodeURIComponent(image)}&arch=${archValue}`,
      history,
    );
    setImageName(image);
    setLoading(true);
    setPreview(null);
    const current = ++ticket.current;
    const ref = encodeURIComponent(withArch(image, archValue));
    let analyzeDone = false;
    const manifestController = new AbortController();
    // Registry images only: a local source has no cheap manifest to read.
    const previewTimer = /^(file|docker|podman):\/\//.test(image)
      ? undefined
      : window.setTimeout(() => {
          if (analyzeDone || current !== ticket.current) {
            return;
          }
          request
            .get<ImagePreview>(`/manifest?image=${ref}`, {
              timeout: 60 * 1000,
              signal: manifestController.signal,
            })
            .then(({ data }) => {
              if (!analyzeDone && current === ticket.current) {
                setPreview(data);
              }
            })
            .catch(() => {
              /* the analysis reports any error */
            });
        }, PREVIEW_DELAY_MS);
    try {
      const reqUrl = `/analyze?image=${ref}`;
      const { data } = await request.get<ImageAnalyzeResult>(reqUrl, {
        timeout: 10 * 60 * 1000,
      });
      if (current !== ticket.current) {
        return;
      }
      (data.fileTreeList || []).forEach((fileTree) => {
        addKeyToFileTreeItem(fileTree, "");
      });
      const summary = getImageSummary(data);
      setReport({
        imageDescriptions: summary.imageDescriptions,
        wastedList: summary.wastedList,
        fileTreeList: data.fileTreeList || [],
        layers: data.layers || [],
        bigModifiedFileList: data.bigModifiedFileList || [],
        recommendations: data.recommendations || [],
        tags: data.tags || [],
        sensitiveFiles: data.sensitiveFiles || [],
        duplicateGroups: data.duplicateGroups || [],
        dockerfile: data.dockerfile || "",
        staleAsOf: data.staleAsOf || "",
        supportedArchs: data.supportedArchs || [],
      });
      setCurrentLayer(0);
      setDiff(null);
    } catch (err: unknown) {
      if (current === ticket.current) {
        showRequestError(err, "analyze image fail");
      }
    } finally {
      analyzeDone = true;
      window.clearTimeout(previewTimer);
      manifestController.abort();
      if (current === ticket.current) {
        setLoading(false);
        setPreview(null);
      }
    }
  };

  const onCompare = async (
    base: string,
    target: string,
    history: HistoryMode = "push",
    archValue = arch,
  ) => {
    setLocation(
      `?base=${encodeURIComponent(base)}&target=${encodeURIComponent(target)}&arch=${archValue}`,
      history,
    );
    setLoading(true);
    const current = ++ticket.current;
    try {
      const { data } = await request.get<ImageDiff>(
        `/diff?base=${encodeURIComponent(withArch(base, archValue))}&target=${encodeURIComponent(withArch(target, archValue))}`,
        { timeout: 20 * 60 * 1000 },
      );
      if (current !== ticket.current) {
        return;
      }
      setDiff({ refs: { base, target }, data });
      setReport(null);
    } catch (err: unknown) {
      if (current === ticket.current) {
        showRequestError(err, "compare images fail");
      }
    } finally {
      if (current === ticket.current) {
        setLoading(false);
      }
    }
  };

  // Show whatever the URL names: on the first load and after back / forward.
  const openQuery = (history: HistoryMode) => {
    const query = readQuery();
    setArch(query.arch);
    if (query.base && query.target) {
      onCompare(query.base, query.target, history, query.arch);
    } else if (query.image) {
      onSearch(query.image, history, query.arch);
    } else {
      ticket.current++;
      setLoading(false);
      setPreview(null);
      setReport(null);
      setDiff(null);
      setImageName("");
    }
  };
  const openQueryRef = useRef(openQuery);
  useEffect(() => {
    openQueryRef.current = openQuery;
  });
  useEffect(() => {
    const onPopState = () => openQueryRef.current("none");
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const mounted = useRef(false);
  useEffect(() => {
    if (mounted.current) {
      return;
    }
    mounted.current = true;
    openQuery("replace");
    request
      .get<LatestImages>("/latest-images", { timeout: 5 * 1000 })
      .then(({ data }) => {
        setLatestImages(data.images);
        setVersion(data.version);
        setMcp(data.mcp || null);
      })
      .catch(() => {
        /* decorative list */
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onToggleExpand = useCallback((key: string) => {
    setViewOption((opt) => {
      if (opt.expandAll || opt.keyword) {
        const collapsed = opt.collapsedItems || [];
        const next = collapsed.includes(key)
          ? collapsed.filter((k) => k !== key)
          : [...collapsed, key];
        return { ...opt, collapsedItems: next };
      }
      const items = opt.expandItems || [];
      const next = items.includes(key)
        ? items.filter((k) => k !== key)
        : [...items, key];
      return { ...opt, expandItems: next };
    });
  }, []);

  const updateOption = (patch: Partial<FileTreeViewOption>) => {
    setViewOption((opt) => ({ ...opt, ...patch }));
  };

  // Keyboard shortcuts: ↑/↓ navigate layers, / focuses file filter, Esc blurs
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const isEditing =
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable);

      if (e.key === "Escape") {
        if (isEditing && target) {
          target.blur();
        }
        return;
      }
      if (isEditing || !report || preview) {
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCurrentLayer((i) => Math.min(report.layers.length - 1, i + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setCurrentLayer((i) => Math.max(0, i - 1));
      } else if (e.key === "/") {
        e.preventDefault();
        fileSearchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [report, preview]);

  const deferredOption = useDeferredValue(viewOption);
  const fileTreeRows = useMemo(() => {
    const rows: FileTreeRow[] = [];
    if (report) {
      flattenFileTree(
        report.fileTreeList[currentLayer],
        0,
        deferredOption,
        rows,
      );
    }
    return rows;
  }, [report, currentLayer, deferredOption]);

  const activeLayerCounts = useMemo(
    () => countLayerFiles(report?.fileTreeList[currentLayer]),
    [report, currentLayer],
  );

  const findingsCount = useMemo(() => {
    if (!report) return 0;
    return (
      report.wastedList.filter((w) => w.totalSize > 0).length +
      report.sensitiveFiles.length +
      report.duplicateGroups.length +
      report.bigModifiedFileList.length +
      report.recommendations.length
    );
  }, [report]);

  const sizeOptions = useMemo(() => {
    return [
      0,
      10 * 1000,
      30 * 1000,
      100 * 1000,
      500 * 1000,
      1000 * 1000,
      10 * 1000 * 1000,
    ].map((size) => {
      let label = `≥ ${prettyBytes(size)}`;
      if (size === 0) {
        label = "No limit";
      }
      return { value: size, label };
    });
  }, []);

  // Switching platform redoes what is on screen for that platform.
  const onArchChange = (value: string) => {
    setArch(value);
    if (diff) {
      onCompare(diff.refs.base, diff.refs.target, "push", value);
    } else if (report && imageName) {
      onSearch(imageName, "push", value);
    }
  };

  const searchBar = (
    <SearchBar
      arch={arch}
      platforms={report?.supportedArchs}
      defaultImage={imageName}
      loading={loading}
      onArchChange={onArchChange}
      onSearch={onSearch}
    />
  );

  const renderFileTreeTab = () => {
    if (!report) {
      return null;
    }
    const layerInfo = report.layers[currentLayer];
    if (!layerInfo) {
      return null;
    }
    const isEmptyLayer =
      !report.fileTreeList[currentLayer] ||
      report.fileTreeList[currentLayer].length === 0;
    return (
      <div className="fileTreeWorkspace">
        <CommandInspector
          layer={layerInfo}
          layerIndex={currentLayer}
          fileCounts={activeLayerCounts}
        />
        <div className="fileTreeToolbar">
          <Input
            ref={fileSearchRef}
            className="fileTreeSearchInput"
            allowClear
            size="small"
            placeholder={`${i18nGet("keywordsLabel")} (e.g. usr/local, *.so)  [/]`}
            onChange={(e) => {
              updateOption({ keyword: e.target.value.trim(), collapsedItems: [] });
            }}
          />
          <Select
            size="small"
            value={viewOption.sizeLimit || 0}
            options={sizeOptions}
            className="fileTreeSizeSelect"
            onChange={(limit: number) => {
              updateOption({ sizeLimit: limit });
            }}
          />
          <Checkbox
            checked={!!viewOption.onlyModifiedRemoved}
            onChange={(e) => {
              updateOption({ onlyModifiedRemoved: e.target.checked });
            }}
          >
            {i18nGet("modificationLabel")}
          </Checkbox>
          <Checkbox
            checked={!!viewOption.expandAll}
            onChange={(e) => {
              updateOption({
                expandAll: e.target.checked,
                collapsedItems: [],
                expandItems: [],
              });
            }}
          >
            {i18nGet("expandLabel")}
          </Checkbox>
          <Checkbox
            checked={viewOption.compactPaths !== false}
            onChange={(e) => {
              updateOption({ compactPaths: e.target.checked });
            }}
          >
            {i18nGet("compactPathsLabel")}
          </Checkbox>
          <Checkbox
            checked={showMetaColumns}
            onChange={(e) => {
              setShowMetaColumns(e.target.checked);
            }}
          >
            {i18nGet("showColumnsLabel")}
          </Checkbox>
        </div>
        <ul
          className={`fileTreeHeaderOnly ${showMetaColumns ? "withMeta" : ""}`}
        >
          <li>
            <span className="colStatus">{i18nGet("statusLabel")}</span>
            {showMetaColumns && (
              <>
                <span className="colMode">{i18nGet("permissionLabel")}</span>
                <span className="colUid">UID:GID</span>
              </>
            )}
            <span className="colSize">{i18nGet("sizeLabel")}</span>
            <span className="colBar" aria-hidden="true" />
            <span className="colPath">{i18nGet("fileTreeLabel")}</span>
          </li>
        </ul>
        <VirtualFileTree
          rows={fileTreeRows}
          layer={layerInfo}
          onToggleExpand={onToggleExpand}
          showMetaColumns={showMetaColumns}
          emptyMessage={
            isEmptyLayer ? i18nGet("noFilesInLayer") : undefined
          }
        />
      </div>
    );
  };

  const githubIcon = getGithubIcon(isDark);
  const themeTitle =
    themeMode === "light"
      ? i18nGet("themeLight")
      : themeMode === "dark"
        ? i18nGet("themeDark")
        : i18nGet("themeSystem");

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? darkAlgorithm : defaultAlgorithm,
        token: {
          colorPrimary: isDark ? "#38bdf8" : "#0ea5e9",
          colorInfo: isDark ? "#818cf8" : "#6366f1",
          colorSuccess: isDark ? "#34d399" : "#059669",
          colorWarning: isDark ? "#fbbf24" : "#d97706",
          colorError: isDark ? "#f87171" : "#dc2626",
          borderRadius: 6,
          fontFamily: '"IBM Plex Sans", system-ui, -apple-system, sans-serif',
          colorBgContainer: isDark ? "#111720" : "#ffffff",
          colorBgLayout: isDark ? "#090d12" : "#f8fafc",
          colorText: isDark ? "#f1f5f9" : "#0f172a",
          colorBorder: isDark
            ? "rgba(255, 255, 255, 0.12)"
            : "rgba(15, 23, 42, 0.12)",
        },
      }}
    >
      <Layout className="appLayout">
        <Header
          className={report || diff || preview ? "header hasSearch" : "header"}
        >
          <div className="headerInner">
            <div className="headerLeft">
              <div
                className="logo"
                onClick={() => {
                  window.location.href = "/";
                }}
              >
                {getLogoIcon(isDark)}
                <span>Diving</span>
                {version && <span className="version">v{version}</span>}
              </div>
              {report && !preview && (
                <div className="headerBreadcrumb" title={imageName}>
                  <span className="breadcrumbSep">/</span>
                  <code className="breadcrumbImage">{imageName}</code>
                </div>
              )}
            </div>
            {(report || diff || preview) && (
              <div className="headerSearch">{searchBar}</div>
            )}
            <div className="headerActions">
              {report && !preview && (
                <CompareButton
                  target={imageName}
                  platform={arch}
                  platforms={report.supportedArchs}
                  onCompare={onCompare}
                />
              )}
              {mcp?.enabled && <McpGuide status={mcp} />}
              <button
                type="button"
                className="headerIconBtn"
                onClick={cycleTheme}
                title={themeTitle}
                aria-label={themeTitle}
              >
                {getThemeIcon(themeMode)}
              </button>
              {githubIcon}
            </div>
          </div>
        </Header>

        {preview && (
          <Content>
            <div className="contentWrapper reportStack">
              <ImagePreviewCard preview={preview} />
            </div>
          </Content>
        )}

        {!report && !diff && !preview && (
          <div className="landing">
            <div className="landingInner">
              <div className="landingEyebrow">Docker · OCI · layers</div>
              <h1 className="landingTitle">{i18nGet("landingTitle")}</h1>
              <p className="landingLead">{i18nGet("landingLead")}</p>
              <div className="landingSearch">{searchBar}</div>
              <div className="exampleChips">
                {EXAMPLE_IMAGES.map((img) => (
                  <button
                    key={img}
                    type="button"
                    className="exampleChip"
                    onClick={() => onSearch(img)}
                  >
                    {img}
                  </button>
                ))}
              </div>
              <p className="landingHint">{i18nGet("imageSlowDesc")}</p>
            </div>
          </div>
        )}

        {report && !preview && (
          <Content>
            <div className="contentWrapper workspaceShell">
              {report.staleAsOf && (
                <Alert
                  type="warning"
                  showIcon
                  title={i18nGet("staleResultNotice").replace(
                    "{time}",
                    new Date(report.staleAsOf).toLocaleString(),
                  )}
                />
              )}
              <KpiStrip
                desc={report.imageDescriptions}
                tags={report.tags}
              />
              <div className="workspaceMobileSwitch">
                <Segmented
                  block
                  value={mobilePane}
                  onChange={(v) =>
                    setMobilePane(v as "layers" | "workspace")
                  }
                  options={[
                    {
                      label: `${i18nGet("mobileViewLayers")} (${report.layers.length})`,
                      value: "layers",
                    },
                    {
                      label: i18nGet("mobileViewWorkspace"),
                      value: "workspace",
                    },
                  ]}
                />
              </div>
              <div className={`workspaceGrid mobile-${mobilePane}`}>
                <aside className="workspaceMaster">
                  <LayerTimeline
                    layers={report.layers}
                    fileTreeList={report.fileTreeList}
                    currentLayer={currentLayer}
                    onSelectLayer={(idx) => {
                      setCurrentLayer(idx);
                      setMobilePane("workspace");
                    }}
                  />
                </aside>
                <main className="workspaceDetail">
                  <Tabs
                    className="workspaceTabs"
                    defaultActiveKey="files"
                    items={[
                      {
                        key: "files",
                        label: `${i18nGet("tabFiles")} (${activeLayerCounts.total})`,
                        children: renderFileTreeTab(),
                      },
                      {
                        key: "treemap",
                        label: i18nGet("tabTreemap"),
                        children: (
                          <ImageTreemap
                            fileTreeList={report.fileTreeList}
                            layers={report.layers}
                          />
                        ),
                      },
                      {
                        key: "findings",
                        label: `${i18nGet("tabFindings")} (${findingsCount})`,
                        children: (
                          <div className="findingsGrid">
                            <WastedSummaryCard wastedList={report.wastedList} />
                            <SensitiveFilesCard files={report.sensitiveFiles} />
                            <DuplicateGroupsCard
                              groups={report.duplicateGroups}
                            />
                            <BigModifiedFilesCard
                              files={report.bigModifiedFileList}
                            />
                            <RecommendationsCard
                              recommendations={report.recommendations}
                            />
                          </div>
                        ),
                      },
                      {
                        key: "dockerfile",
                        label: i18nGet("tabDockerfile"),
                        children: (
                          <DockerfileCard dockerfile={report.dockerfile} />
                        ),
                      },
                    ]}
                  />
                </main>
              </div>
              <div className="workspaceStatusBar">
                <span>{i18nGet("shortcutsHint")}</span>
              </div>
            </div>
          </Content>
        )}
        {diff && !preview && (
          <Content>
            <div className="contentWrapper reportStack">
              <DiffView
                diff={diff.data}
                refs={diff.refs}
                onSwap={() => onCompare(diff.refs.target, diff.refs.base)}
                onOpen={(image) => onSearch(image)}
              />
            </div>
          </Content>
        )}
        <LatestImagesList images={latestImages} />
      </Layout>
    </ConfigProvider>
  );
};

export default App;
