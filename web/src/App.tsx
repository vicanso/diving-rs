import { Component, ReactNode } from "react";
import {
  ConfigProvider,
  theme,
  Card,
  Layout,
  Input,
  message,
  Descriptions,
  Form,
  Select,
  Col,
  Row,
  Checkbox,
  Typography,
  Space,
  List,
} from "antd";
import axios, { AxiosError } from "axios";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";

import "./App.css";

const { Option } = Select;
const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;
const { Search } = Input;
const { Paragraph } = Typography;

interface ImageAnalyzeResult {
  name: string;
  arch: string;
  os: string;
  layers: Layer[];
  size: number;
  totalSize: number;
  fileTreeList: FileTreeList[][];
  fileSummaryList: FileSummaryList[];
}

interface Layer {
  created: string;
  digest: string;
  mediaType: string;
  cmd: string;
  size: number;
  unpackSize: number;
  empty: boolean;
}

interface FileTreeList {
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

interface FileSummaryList {
  layerIndex: number;
  op: number;
  info: Info;
}

interface Info {
  path: string;
  link: string;
  size: number;
  mode: string;
  uid: number;
  gid: number;
  isWhiteout: any;
}
interface FileWastedSummary {
  path: string;
  totalSize: number;
  count: number;
}

const plusOutlined = (
  <svg
    viewBox="64 64 896 896"
    focusable="false"
    fill="currentColor"
    height="14px"
    aria-hidden="true"
  >
    <path d="M328 544h152v152c0 4.4 3.6 8 8 8h48c4.4 0 8-3.6 8-8V544h152c4.4 0 8-3.6 8-8v-48c0-4.4-3.6-8-8-8H544V328c0-4.4-3.6-8-8-8h-48c-4.4 0-8 3.6-8 8v152H328c-4.4 0-8 3.6-8 8v48c0 4.4 3.6 8 8 8z"></path>
    <path d="M880 112H144c-17.7 0-32 14.3-32 32v736c0 17.7 14.3 32 32 32h736c17.7 0 32-14.3 32-32V144c0-17.7-14.3-32-32-32zm-40 728H184V184h656v656z"></path>
  </svg>
);
const minusOutlined = (
  <svg
    viewBox="64 64 896 896"
    focusable="false"
    fill="currentColor"
    height="14px"
    aria-hidden="true"
  >
    <path d="M328 544h368c4.4 0 8-3.6 8-8v-48c0-4.4-3.6-8-8-8H328c-4.4 0-8 3.6-8 8v48c0 4.4 3.6 8 8 8z"></path>
    <path d="M880 112H144c-17.7 0-32 14.3-32 32v736c0 17.7 14.3 32 32 32h736c17.7 0 32-14.3 32-32V144c0-17.7-14.3-32-32-32zm-40 728H184V184h656v656z"></path>
  </svg>
);

const isDarkMode = () =>
  window.matchMedia("(prefers-color-scheme: dark)").matches;

const getLogoIcon = (isDarkMode: boolean) => {
  let color = `rgb(0, 0, 0)`;
  if (isDarkMode) {
    color = `rgb(255, 255, 255)`;
  }
  return (
    <svg
      height="32"
      viewBox="0 0 64 64"
      xmlns="http://www.w3.org/2000/svg"
      style={{
        fill: color,
      }}
    >
      <path d="m27.04 24.126c.419-.293.977-.288 1.39.013l7.807 5.681c4.489 3.265 10.827 2.623 14.43-1.465 2.143-2.431 3.324-5.553 3.324-8.791 0-7.889-6.359-14.308-14.174-14.308h-25.397c-7.4 0-13.42 6.077-13.42 13.546v.762c0 4.31 2.104 8.369 5.627 10.859 1.685 1.191 3.671 1.785 5.669 1.785 2.028-.001 4.069-.613 5.82-1.838zm-18.578 3.7c-2.682-1.895-4.282-4.983-4.282-8.262v-.762c0-5.716 4.594-10.366 10.241-10.366h25.397c6.063 0 10.995 4.992 10.995 11.128 0 2.463-.898 4.839-2.53 6.688-2.531 2.868-6.999 3.307-10.174.997l-8.046-5.855c-1.368-.995-3.217-1.012-4.603-.043l-9.166 6.414c-2.379 1.665-5.528 1.69-7.832.061z" />
      <path d="m29.679 19.546c2.333 1.677 6.026 4.332 8.501 6.11 2.42 1.739 5.768 1.776 8.047-.144 1.85-1.558 3.029-3.887 3.029-6.478 0-4.955-4.054-9.009-9.009-9.009h-25.36c-4.745 0-8.627 3.882-8.627 8.627v.382c0 2.358.978 4.5 2.548 6.041 2.323 2.279 6.024 2.379 8.629.428l7.896-5.912c1.285-.962 3.042-.982 4.346-.045z" />
      <path d="m62.973 1.017h-7.419c.007.177.027.351.027.53v40.274c0 7.305-5.943 13.248-13.248 13.248-6.765 0-12.337-5.103-13.126-11.658h6.238v-8.479h-19.077v8.479h5.406c.819 10.65 9.703 19.077 20.56 19.077 11.395-.001 20.666-9.272 20.666-20.667v-40.274c0-.179-.022-.352-.027-.53zm-2.093 9.539h-3.179v-8.479h3.179z" />
    </svg>
  );
};

const getGithubIcon = (isDarkMode: boolean) => {
  let color = `rgb(0, 0, 0)`;
  if (isDarkMode) {
    color = `rgb(255, 255, 255)`;
  }
  return (
    <a
      href="https://github.com/vicanso/diving-rs"
      style={{
        position: "absolute",
        padding: "15px 30px",
        right: 0,
        top: 0,
      }}
    >
      <svg
        height="32"
        viewBox="0 0 16 16"
        width="32"
        aria-hidden="true"
        style={{
          fill: color,
        }}
      >
        <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
      </svg>
    </a>
  );
};

const getDownloadIcon = () => {
  const color = `#646cff`;
  return (
    <svg
      width="16px"
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M12 16L12 8"
        stroke={color}
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M9 13L11.913 15.913V15.913C11.961 15.961 12.039 15.961 12.087 15.913V15.913L15 13"
        stroke={color}
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M3 15L3 16L3 19C3 20.1046 3.89543 21 5 21L19 21C20.1046 21 21 20.1046 21 19L21 16L21 15"
        stroke={color}
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
};

const getImageSummary = (result: ImageAnalyzeResult) => {
  let wastedSize = 0;
  let wastedList: FileWastedSummary[] = [];
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

  const score = (100 - (wastedSize * 100) / result.totalSize).toFixed(2);

  const imageDescriptions = {
    score: `${score}%`,
    size: `${prettyBytes(result.totalSize)} / ${prettyBytes(result.size)}`,
    otherSize: prettyBytes(otherLayerSize),
    wastedSize: prettyBytes(wastedSize),
    osArch: `${result.os}/${result.arch}`,
  };
  return {
    wastedList,
    imageDescriptions,
  };
};

const addKeyToFileTreeItem = (items: FileTreeList[], prefix: string) => {
  items.forEach((item) => {
    let key = item.name;
    if (prefix) {
      key = `${prefix}/${key}`;
    }
    item.key = key;
    addKeyToFileTreeItem(item.children, key);
  });
};

interface FileTreeViewOption {
  expandAll: boolean;
  expandItems: string[];
  sizeLimit: number;
  onlyModifiedRemoved: boolean;
  keyword: string;
}

const opRemoved = 1;
const opModified = 2;

const isModifiedRemoved = (item: FileTreeList) => {
  const arr = [opRemoved, opModified];
  if (arr.includes(item.op)) {
    return true;
  }
  // 如果子元素符合，则也符合
  for (let i = 0; i < item.children.length; i++) {
    const { op } = item.children[i];
    if (arr.includes(op)) {
      return true;
    }
  }
  return false;
};

const isMatchKeyword = (item: FileTreeList, keyword: string) => {
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

const addToFileTreeView = (
  onToggleExpand: (key: string) => void,
  layer: Layer,
  list: JSX.Element[],
  items: FileTreeList[],
  isLastList: boolean[],
  opt: FileTreeViewOption
) => {
  if (!items) {
    return 0;
  }
  const max = items.length;
  let count = 0;
  const isExpandAll = () => {
    if (opt.expandAll || opt.keyword) {
      return true;
    }
    return false;
  };

  const shouldExpand = (key: string) => {
    if (isExpandAll()) {
      return true;
    }
    if (opt.expandItems?.includes(key)) {
      return true;
    }
    return false;
  };
  items.forEach((item, index) => {
    // 如果限制了大小
    if (opt.sizeLimit && item.size < opt.sizeLimit) {
      return;
    }
    // 如果仅展示更新、删除选项
    if (opt.onlyModifiedRemoved && !isModifiedRemoved(item)) {
      return;
    }
    // 如果指定关键字筛选
    if (opt.keyword && !isMatchKeyword(item, opt.keyword)) {
      return;
    }
    const id = `${item.uid}:${item.gid}`;
    const isLast = index === max - 1;
    let name = item.name;
    if (item.link) {
      name = `${name} → ${item.link}`;
    }
    const padding = isLastList.length * 30;

    let className = "";
    if (item.op === opRemoved) {
      className = "removed";
    } else if (item.op === opModified) {
      className = "modified";
    }
    let icon: JSX.Element = <></>;
    if (item.children.length) {
      const { key } = item;
      if (isExpandAll() || opt.expandItems?.includes(key)) {
        icon = minusOutlined;
      } else {
        icon = plusOutlined;
      }
      icon = (
        <a
          href="#"
          className="icon"
          onClick={(e) => {
            e.preventDefault();
            onToggleExpand(key);
          }}
        >
          {icon}
        </a>
      );
    }
    let downloadIcon: JSX.Element = <></>;
    if (item.children.length === 0 && item.size > 0) {
      downloadIcon = (
        <a
          className="download"
          href={`/api/file?digest=${layer.digest}&mediaType=${layer.mediaType}&file=${item.key}`}
        >
          {getDownloadIcon()}
        </a>
      );
    }
    list.push(
      <li key={item.key}>
        <span>{item.mode}</span>
        <span>{id}</span>
        <span>{prettyBytes(item.size)}</span>
        <span
          className={className}
          style={{
            paddingLeft: padding,
          }}
        >
          {icon}
          {name}
          {downloadIcon}
        </span>
      </li>
    );
    count++;
    if (item.children.length && shouldExpand(item.key)) {
      const tmp = isLastList.slice(0);
      tmp.push(isLast);
      const childAppendCount = addToFileTreeView(
        onToggleExpand,
        layer,
        list,
        item.children,
        tmp,
        opt
      );
      // 如果子文件一个都没有插入
      // 也未指定keyword
      // 则将当前目录也删除
      if (childAppendCount === 0 && opt.keyword === "") {
        list.pop();
        count -= 1;
      }
    }
  });
  return count;
};

interface ImageDescriptions {
  score: string;
  size: string;
  otherSize: string;
  wastedSize: string;
  osArch: string;
}
interface AppState {
  gotResult: boolean;
  loading: boolean;
  imageDescriptions: ImageDescriptions;
  layers: Layer[];
  currentLayer: number;
  fileTreeList: FileTreeList[][];
  fileTreeViewOption: FileTreeViewOption;
  wastedList: FileWastedSummary[];
  imageName: string;
  arch: string;
  latestAnalyzeImages: string[];
}
interface App {
  state: AppState;
}
const amd64Arch = "amd64";
const arm64Arch = "arm64";

class App extends Component {
  constructor(props: any) {
    super(props);
    const urlInfo = new URL(window.location.href);
    const image = urlInfo.searchParams.get("image") || "";
    let arch = urlInfo.searchParams.get("arch") || amd64Arch;
    if ([amd64Arch, arm64Arch].indexOf(arch) === -1) {
      arch = amd64Arch;
    }
    this.state = {
      gotResult: false,
      loading: false,
      imageDescriptions: {} as ImageDescriptions,
      layers: [],
      currentLayer: 0,
      fileTreeList: [],
      fileTreeViewOption: {} as FileTreeViewOption,
      wastedList: [],
      imageName: image,
      arch,
      latestAnalyzeImages: [],
    };
  }
  async componentDidMount() {
    if (this.state.imageName) {
      this.onSearch(this.state.imageName);
    }
    const { data } = await axios.get<string[]>("/api/latest-images", {
      timeout: 5 * 1000,
    });
    this.setState({
      latestAnalyzeImages: data,
    });
  }
  async onSearch(value: String) {
    const image = value.trim();
    if (!image) {
      return;
    }
    const { arch } = this.state;
    const url = `/?image=${image}&arch=${arch}`;
    if (window.location.href !== url) {
      window.history.pushState(null, "", url);
    }

    this.setState({
      imageName: image,
      loading: true,
    });
    try {
      let url = `/api/analyze?image=${image}`;
      if (!/^(file|docker):\/\//.test(image) && arch) {
        url += `?arch=${arch}`;
      }
      const { data } = await axios.get<ImageAnalyzeResult>(url, {
        timeout: 10 * 60 * 1000,
      });
      // 为每个file tree item增加key
      data.fileTreeList.forEach((fileTree) => {
        addKeyToFileTreeItem(fileTree, "");
      });

      const result = getImageSummary(data);
      this.setState({
        imageDescriptions: result.imageDescriptions,
        wastedList: result.wastedList,
        fileTreeList: data.fileTreeList,
        layers: data.layers,
        currentLayer: 0,
        gotResult: true,
      });
    } catch (err: any) {
      let msg = err?.message as string;
      let axiosErr = err as AxiosError;
      if (axiosErr?.response?.data) {
        let data = axiosErr.response.data as {
          message: string;
        };
        msg = data.message || "";
      }
      message.error(msg || "analyze image fail", 10);
    } finally {
      this.setState({
        loading: false,
      });
    }
  }
  render(): ReactNode {
    const {
      imageName,
      gotResult,
      loading,
      imageDescriptions,
      layers,
      currentLayer,
      fileTreeList,
      fileTreeViewOption,
      wastedList,
      arch,
      latestAnalyzeImages,
    } = this.state;
    const onToggleExpand = (key: string) => {
      const opt = Object.assign({}, this.state.fileTreeViewOption);
      const items = opt.expandItems || [];
      const index = items.indexOf(key);
      if (index === -1) {
        items.push(key);
      } else {
        items.splice(index, 1);
      }
      opt.expandItems = items;
      this.setState({
        fileTreeViewOption: opt,
      });
    };

    const selectLayer = (index: number) => {
      this.setState({
        currentLayer: index,
      });
    };

    const getImageSummaryView = () => {
      const imageSummary = (
        <Descriptions title={i18nGet("imageSummaryTitle")}>
          <Descriptions.Item label={i18nGet("imageScoreLabel")}>
            {imageDescriptions["score"]}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("imageSizeLabel")}>
            {imageDescriptions["size"]}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("otherLayerSizeLabel")}>
            {imageDescriptions["otherSize"]}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("wastedSizeLabel")}>
            {imageDescriptions["wastedSize"]}
          </Descriptions.Item>
          <Descriptions.Item label={i18nGet("osArchLabel")}>
            {imageDescriptions["osArch"]}
          </Descriptions.Item>
        </Descriptions>
      );
      return <div className="imageSummary mtop30">{imageSummary}</div>;
    };

    const layerOptions = layers.map((item, index) => {
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

      let label = `${index + 1}: ${digest.toUpperCase()}${sizeDesc}`;
      return {
        value: index,
        label,
      };
    });

    const sizeOptions = [
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

    const fileTreeViewList = [] as JSX.Element[];
    addToFileTreeView(
      onToggleExpand,
      layers[currentLayer],
      fileTreeViewList,
      fileTreeList[currentLayer],
      [],
      fileTreeViewOption
    );

    const layerFilter = (
      <Row gutter={20}>
        <Col span={6}>
          <Form.Item label={i18nGet("layerLabel")}>
            <Select
              defaultValue={0}
              style={{
                width: "100%",
              }}
              onChange={selectLayer}
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
                const opt = Object.assign({}, this.state.fileTreeViewOption);
                opt.sizeLimit = limit;
                this.setState({
                  fileTreeViewOption: opt,
                });
              }}
            />
          </Form.Item>
        </Col>
        <Col span={3}>
          <Form.Item>
            <Checkbox
              onChange={(e) => {
                const opt = Object.assign({}, fileTreeViewOption);
                opt.onlyModifiedRemoved = e.target.checked;
                this.setState({
                  fileTreeViewOption: opt,
                });
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
                const opt = Object.assign({}, fileTreeViewOption);
                opt.expandAll = e.target.checked;
                this.setState({
                  fileTreeViewOption: opt,
                });
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
                const opt = Object.assign({}, fileTreeViewOption);
                opt.keyword = e.target.value.trim();
                this.setState({
                  fileTreeViewOption: opt,
                });
              }}
            />
          </Form.Item>
        </Col>
      </Row>
    );

    const getLayerContentView = () => {
      let fileTreeListClassName = "fileTree";
      if (isDarkMode()) {
        fileTreeListClassName += " dark";
      }

      const layerInfo = layers[currentLayer];

      const cmd = (
        <>
          <Card className="command">
            <Space direction="vertical">
              <span>
                <span className="bold">Created: </span>
                {new Date(layerInfo.created).toLocaleString()}
              </span>
              <span>
                <span className="bold">Command: </span>
                {layerInfo.cmd}
              </span>
            </Space>
          </Card>
        </>
      );
      return (
        <div className="mtop30">
          <Card title={i18nGet("layerContentTitle")}>
            {layerFilter}
            {cmd}
            <ul className={fileTreeListClassName}>
              <li>
                <span>{i18nGet("permissionLabel")}</span>
                <span>UID:GID</span>
                <span>{i18nGet("sizeLabel")}</span>
                <span>{i18nGet("fileTreeLabel")}</span>
              </li>
              {fileTreeViewList}
            </ul>
          </Card>
        </div>
      );
    };

    const getWastedSummaryView = () => {
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
      if (isDarkMode()) {
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
    };
    const getSearchView = () => {
      const size = "large";
      const selectBefore = (
        <Select
          size={size}
          defaultValue={arch}
          style={{
            width: "100px",
          }}
          onChange={(value) => {
            this.setState({
              arch: value,
            });
          }}
        >
          <Option value="amd64">AMD64</Option>
          <Option value="arm64">ARM64</Option>
        </Select>
      );
      return (
        <Search
          addonBefore={selectBefore}
          defaultValue={imageName}
          autoFocus={true}
          loading={loading}
          placeholder={i18nGet("imageInputPlaceholder")}
          allowClear
          enterButton={i18nGet("analyzeButton")}
          size={size}
          onSearch={this.onSearch.bind(this)}
        />
      );
    };
    let headerClass = "header";
    if (isDarkMode()) {
      headerClass += " dark";
    }

    const getLatestAnalyzeImagesView = () => {
      if (latestAnalyzeImages.length === 0) {
        return <></>;
      }
      return (
        <List
          className="analyzeImages"
          bordered={true}
          size={"small"}
          header={<div>{i18nGet("latestAnalyzeImagesTitle")}</div>}
          dataSource={latestAnalyzeImages}
          renderItem={(item) => (
            <List.Item>
              <Typography.Text>
                <a
                  href="#"
                  onClick={(e) => {
                    const arr = item.split("?");
                    const image = arr[0];
                    let arch = amd64Arch;
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
    };

    return (
      <ConfigProvider
        theme={{
          algorithm: isDarkMode() ? darkAlgorithm : defaultAlgorithm,
        }}
      >
        <Layout>
          {getGithubIcon(isDarkMode())}
          <Header className={headerClass}>
            <div className="contentWrapper">
              <div
                className="logo"
                onClick={() => {
                  window.location.href = "/";
                }}
              >
                <Space>
                  {getLogoIcon(isDarkMode())}
                  <span>Diving</span>
                </Space>
              </div>
              {gotResult && <div className="search">{getSearchView()}</div>}
            </div>
          </Header>
          {!gotResult && (
            <div className="fixSearch">
              {getSearchView()}
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
          {gotResult && (
            <Content>
              <div className="contentWrapper">
                {getImageSummaryView()}
                {getLayerContentView()}
                {getWastedSummaryView()}
              </div>
            </Content>
          )}
          {getLatestAnalyzeImagesView()}
        </Layout>
      </ConfigProvider>
    );
  }
}

export default App;
