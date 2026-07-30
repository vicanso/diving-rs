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
  Space,
  theme,
  Typography,
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
  ImageSummaryCard,
  LatestImagesList,
  RecommendationsCard,
  SearchBar,
  SensitiveFilesCard,
  TagsCard,
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
const { Paragraph } = Typography;

const amd64Arch = "amd64";
const arm64Arch = "arm64";
const request = axios.create({
  timeout: 600 * 1000,
  baseURL: "./api",
});

// 跟随系统深浅色并监听变化（此前只在首次渲染读取一次）
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
  return dark;
};

/** Everything derived from one successful analyze response. */
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
      // Encode image so `?arch=` on the image ref and registry paths stay intact.
      let reqUrl = `/analyze?image=${encodeURIComponent(image)}`;
      if (!/^(file|docker):\/\//.test(image) && arch) {
        // Append arch as a query on the image ref (backend parse_image_info).
        reqUrl = `/analyze?image=${encodeURIComponent(`${image}?arch=${arch}`)}`;
      }
      const { data } = await request.get<ImageAnalyzeResult>(reqUrl, {
        timeout: 10 * 60 * 1000,
      });
      // 为每个file tree item增加key
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
        // 首屏的最近镜像列表是装饰性的，拉取失败静默忽略
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

  // 关键词/过滤条件通过 useDeferredValue 延迟到低优先级渲染，输入保持
  // 流畅；flatten 结果按依赖 memo，不再每次 setState 都全树重算。
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
      let label = `>= ${prettyBytes(size)}`;
      if (size === 0) {
        label = "No Limit";
      }
      return {
        value: size,
        label,
      };
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
      return <></>;
    }
    const layerInfo = report.layers[currentLayer];
    if (!layerInfo) {
      return <></>;
    }
    const layerFilter = (
      <Row gutter={20}>
        <Col span={6}>
          <Form.Item label={i18nGet("layerLabel")}>
            <Select
              defaultValue={0}
              style={{
                width: "100%",
              }}
              onChange={setCurrentLayer}
              options={layerOptions}
            />
          </Form.Item>
        </Col>
        <Col span={4}>
          <Form.Item label={i18nGet("sizeLabel")}>
            <Select
              defaultValue={0}
              options={sizeOptions}
              onChange={(limit: number) => {
                updateOption({ sizeLimit: limit });
              }}
            />
          </Form.Item>
        </Col>
        <Col span={3}>
          <Form.Item>
            <Checkbox
              onChange={(e) => {
                updateOption({ onlyModifiedRemoved: e.target.checked });
              }}
            >
              {i18nGet("modificationLabel")}
            </Checkbox>
          </Form.Item>
        </Col>
        <Col span={3}>
          <Form.Item>
            <Checkbox
              onChange={(e) => {
                updateOption({ expandAll: e.target.checked });
              }}
            >
              {i18nGet("expandLabel")}
            </Checkbox>
          </Form.Item>
        </Col>
        <Col span={8}>
          <Form.Item>
            <Input
              addonBefore={i18nGet("keywordsLabel")}
              allowClear
              onChange={(e) => {
                updateOption({ keyword: e.target.value.trim() });
              }}
            />
          </Form.Item>
        </Col>
      </Row>
    );
    let fileTreeListClassName = "fileTree";
    if (isDark) {
      fileTreeListClassName += " dark";
    }
    return (
      <div className="mtop30">
        <Card title={i18nGet("layerContentTitle")}>
          {layerFilter}
          <Card className="command">
            <Space direction="vertical">
              <span>
                <span className="bold">{i18nGet("createdLabel")}: </span>
                {new Date(layerInfo.created).toLocaleString()}
              </span>
              <span>
                <span className="bold">{i18nGet("commandLabel")}: </span>
                {layerInfo.cmd}
              </span>
            </Space>
          </Card>
          <ul className={fileTreeListClassName + " fileTreeHeaderOnly"}>
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
            isDark={isDark}
          />
        </Card>
      </div>
    );
  };

  let headerClass = "header";
  if (isDark) {
    headerClass += " dark";
  }

  return (
    <ConfigProvider
      theme={{
        algorithm: isDark ? darkAlgorithm : defaultAlgorithm,
      }}
    >
      <Layout>
        {getGithubIcon(isDark)}
        <Header className={headerClass}>
          <div className="contentWrapper">
            <div
              className="logo"
              onClick={() => {
                window.location.href = "/";
              }}
            >
              <Space>
                {getLogoIcon(isDark)}
                <span>Diving {version}</span>
              </Space>
            </div>
            {report && <div className="search">{searchBar}</div>}
          </div>
        </Header>
        {!report && (
          <div className="fixSearch">
            {searchBar}
            <div className="desc">
              <Paragraph>
                {i18nGet("imageAnalyzeDesc")}
                <br />
                redis:alpine, vicanso/diving
                <br />
                quay.io/prometheus/node-exporter
                <br />
                dragonwell-registry.cn-hangzhou.cr.aliyuncs.com/dragonwell/dragonwell
                <br />
                xxx.com/user/image:tag
                <br />
                {i18nGet("imageSlowDesc")}
              </Paragraph>
            </div>
          </div>
        )}
        {report && (
          <Content>
            <div className="contentWrapper">
              <ImageSummaryCard desc={report.imageDescriptions} />
              <TagsCard tags={report.tags} />
              {getLayerContentView()}
              <WastedSummaryCard
                wastedList={report.wastedList}
                isDark={isDark}
              />
              <SensitiveFilesCard
                files={report.sensitiveFiles}
                isDark={isDark}
              />
              <DuplicateGroupsCard
                groups={report.duplicateGroups}
                isDark={isDark}
              />
              <BigModifiedFilesCard
                files={report.bigModifiedFileList}
                isDark={isDark}
              />
              <RecommendationsCard
                recommendations={report.recommendations}
                isDark={isDark}
              />
              <DockerfileCard dockerfile={report.dockerfile} isDark={isDark} />
            </div>
          </Content>
        )}
        <LatestImagesList images={latestImages} />
      </Layout>
    </ConfigProvider>
  );
};

export default App;
