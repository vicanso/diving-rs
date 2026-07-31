import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Card,
  Checkbox,
  Col,
  ConfigProvider,
  Form,
  Input,
  Layout,
  message,
  Row,
  Select,
  theme,
} from "antd";
import axios, { AxiosError } from "axios";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  addKeyToFileTreeItem,
  flattenFileTree,
  getImageSummary,
  FileTreeRow,
  FileTreeViewOption,
} from "./analysis";
import {
  BigModifiedFilesCard,
  DockerfileCard,
  DuplicateGroupsCard,
  EXAMPLE_IMAGES,
  ImageSummaryCard,
  LatestImagesList,
  RecommendationsCard,
  SearchBar,
  SensitiveFilesCard,
  VirtualFileTree,
  WastedSummaryCard,
} from "./components";
import { getGithubIcon, getLogoIcon } from "./icons";
import {
  DuplicateGroup,
  FileTreeList,
  FileWastedSummary,
  ImageAnalyzeResult,
  ImageDescriptions,
  LatestImages,
  Layer,
  ModifiedFile,
  Recommendation,
  SensitiveFile,
} from "./types";

import "./App.css";

const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;

const amd64Arch = "amd64";
const arm64Arch = "arm64";
const request = axios.create({
  timeout: 600 * 1000,
  baseURL: "./api",
});

const useDarkMode = () => {
  const [dark, setDark] = useState(
    () => window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);
  return dark;
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
}

const readQuery = () => {
  const urlInfo = new URL(window.location.href);
  const image = urlInfo.searchParams.get("image") || "";
  let arch = urlInfo.searchParams.get("arch") || amd64Arch;
  if ([amd64Arch, arm64Arch].indexOf(arch) === -1) {
    arch = amd64Arch;
  }
  return { image, arch };
};

const App = () => {
  const isDark = useDarkMode();
  const [initial] = useState(readQuery);
  const [imageName, setImageName] = useState(initial.image);
  const [arch, setArch] = useState(initial.arch);
  const [loading, setLoading] = useState(false);
  const [report, setReport] = useState<ReportState | null>(null);
  const [currentLayer, setCurrentLayer] = useState(0);
  const [viewOption, setViewOption] = useState<FileTreeViewOption>(
    {} as FileTreeViewOption,
  );
  const [latestImages, setLatestImages] = useState<string[]>([]);
  const [version, setVersion] = useState("");

  const onSearch = async (value: string) => {
    const image = value.trim();
    if (!image) {
      return;
    }
    const url = `./?image=${image}&arch=${arch}`;
    if (window.location.href !== url) {
      window.history.pushState(null, "", url);
    }
    setImageName(image);
    setLoading(true);
    try {
      let reqUrl = `/analyze?image=${encodeURIComponent(image)}`;
      if (!/^(file|docker):\/\//.test(image) && arch) {
        reqUrl = `/analyze?image=${encodeURIComponent(`${image}?arch=${arch}`)}`;
      }
      const { data } = await request.get<ImageAnalyzeResult>(reqUrl, {
        timeout: 10 * 60 * 1000,
      });
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
      });
      setCurrentLayer(0);
    } catch (err: unknown) {
      let msg = (err as Error)?.message as string;
      const axiosErr = err as AxiosError;
      if (axiosErr?.response?.data) {
        const data = axiosErr.response.data as {
          message: string;
        };
        msg = data.message || "";
      }
      message.error(msg || "analyze image fail", 10);
    } finally {
      setLoading(false);
    }
  };

  const mounted = useRef(false);
  useEffect(() => {
    if (mounted.current) {
      return;
    }
    mounted.current = true;
    if (initial.image) {
      onSearch(initial.image);
    }
    request
      .get<LatestImages>("/latest-images", { timeout: 5 * 1000 })
      .then(({ data }) => {
        setLatestImages(data.images);
        setVersion(data.version);
      })
      .catch(() => {
        /* decorative list */
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onToggleExpand = useCallback((key: string) => {
    setViewOption((opt) => {
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

  const layerOptions = useMemo(() => {
    return (report?.layers || []).map((item, index) => {
      let { digest } = item;
      if (digest) {
        digest = digest.replace("sha256:", "").substring(0, 8);
      }
      if (!digest) {
        digest = "none";
      }
      const size = item.size || 0;
      let sizeDesc = "";
      if (size > 0) {
        sizeDesc = ` (${prettyBytes(size)})`;
      }
      return {
        value: index,
        label: `${index + 1}: ${digest.toUpperCase()}${sizeDesc}`,
      };
    });
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

  const searchBar = (
    <SearchBar
      arch={arch}
      defaultImage={imageName}
      loading={loading}
      onArchChange={setArch}
      onSearch={onSearch}
    />
  );

  const getLayerContentView = () => {
    if (!report) {
      return null;
    }
    const layerInfo = report.layers[currentLayer];
    if (!layerInfo) {
      return null;
    }
    return (
      <Card className="panel" title={i18nGet("layerContentTitle")}>
        <div className="layerToolbar">
          <Row gutter={[12, 4]}>
            <Col xs={24} sm={12} md={8}>
              <Form.Item
                label={i18nGet("layerLabel")}
                style={{ marginBottom: 12 }}
              >
                <Select
                  value={currentLayer}
                  style={{ width: "100%" }}
                  onChange={setCurrentLayer}
                  options={layerOptions}
                />
              </Form.Item>
            </Col>
            <Col xs={12} sm={6} md={4}>
              <Form.Item
                label={i18nGet("sizeLabel")}
                style={{ marginBottom: 12 }}
              >
                <Select
                  defaultValue={0}
                  options={sizeOptions}
                  style={{ width: "100%" }}
                  onChange={(limit: number) => {
                    updateOption({ sizeLimit: limit });
                  }}
                />
              </Form.Item>
            </Col>
            <Col xs={12} sm={6} md={4}>
              <Form.Item label=" " colon={false} style={{ marginBottom: 12 }}>
                <Checkbox
                  onChange={(e) => {
                    updateOption({ onlyModifiedRemoved: e.target.checked });
                  }}
                >
                  {i18nGet("modificationLabel")}
                </Checkbox>
              </Form.Item>
            </Col>
            <Col xs={12} sm={6} md={3}>
              <Form.Item label=" " colon={false} style={{ marginBottom: 12 }}>
                <Checkbox
                  onChange={(e) => {
                    updateOption({ expandAll: e.target.checked });
                  }}
                >
                  {i18nGet("expandLabel")}
                </Checkbox>
              </Form.Item>
            </Col>
            <Col xs={24} sm={12} md={5}>
              <Form.Item
                label={i18nGet("keywordsLabel")}
                style={{ marginBottom: 12 }}
              >
                <Input
                  allowClear
                  placeholder="path…"
                  onChange={(e) => {
                    updateOption({ keyword: e.target.value.trim() });
                  }}
                />
              </Form.Item>
            </Col>
          </Row>
        </div>
        <div className="layerCmd">
          <div>
            <span className="label">{i18nGet("createdLabel")}</span>
            {new Date(layerInfo.created).toLocaleString()}
          </div>
          <div style={{ marginTop: 6 }}>
            <span className="label">{i18nGet("commandLabel")}</span>
            <span className="cmd">{layerInfo.cmd || "—"}</span>
          </div>
        </div>
        <ul className="fileTreeHeaderOnly">
          <li>
            <span>{i18nGet("permissionLabel")}</span>
            <span>UID:GID</span>
            <span>{i18nGet("sizeLabel")}</span>
            <span>{i18nGet("fileTreeLabel")}</span>
          </li>
        </ul>
        <VirtualFileTree
          rows={fileTreeRows}
          layer={layerInfo}
          onToggleExpand={onToggleExpand}
        />
      </Card>
    );
  };

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? darkAlgorithm : defaultAlgorithm,
        token: {
          colorPrimary: isDark ? "#2dd4bf" : "#0d9488",
          colorInfo: isDark ? "#38bdf8" : "#0284c7",
          colorSuccess: isDark ? "#34d399" : "#059669",
          colorWarning: isDark ? "#fbbf24" : "#d97706",
          colorError: isDark ? "#f87171" : "#dc2626",
          borderRadius: 8,
          fontFamily: '"IBM Plex Sans", system-ui, -apple-system, sans-serif',
          colorBgContainer: isDark ? "#141d26" : "#ffffff",
          colorBgLayout: isDark ? "#0b1218" : "#eef3f6",
          colorText: isDark ? "#e8eef2" : "#0f1c24",
          colorBorder: isDark
            ? "rgba(232, 238, 242, 0.1)"
            : "rgba(15, 28, 36, 0.1)",
        },
      }}
    >
      <Layout className="appLayout">
        {getGithubIcon(isDark)}
        <Header className="header">
          <div className="headerInner">
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
            {report && <div className="headerSearch">{searchBar}</div>}
          </div>
        </Header>

        {!report && (
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

        {report && (
          <Content>
            <div className="contentWrapper reportStack">
              <ImageSummaryCard
                desc={report.imageDescriptions}
                tags={report.tags}
              />
              {getLayerContentView()}
              <div className="findingsGrid">
                <WastedSummaryCard wastedList={report.wastedList} />
                <SensitiveFilesCard files={report.sensitiveFiles} />
                <DuplicateGroupsCard groups={report.duplicateGroups} />
                <BigModifiedFilesCard files={report.bigModifiedFileList} />
                <RecommendationsCard recommendations={report.recommendations} />
                <DockerfileCard dockerfile={report.dockerfile} />
              </div>
            </div>
          </Content>
        )}
        <LatestImagesList images={latestImages} />
      </Layout>
    </ConfigProvider>
  );
};

export default App;
