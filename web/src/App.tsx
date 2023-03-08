import { useState, FC } from "react";
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
} from "antd";
import axios, { AxiosError } from "axios";
import prettyBytes from "pretty-bytes";

import "./App.css";

const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;
const { Search } = Input;
const { Paragraph } = Typography;

interface ImageAnalyzeResult {
  name: string;
  layers: Layer[];
  size: number;
  totalSize: number;
  fileTreeList: FileTreeList[][];
  fileSummaryList: FileSummaryList[];
}

interface Layer {
  created: string;
  digest: string;
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

  // 除去第一层layer的大小
  const otherLayerSize = result.totalSize - result.layers[0].size;

  const score = (100 - (wastedSize * 100) / result.totalSize).toFixed(2);

  const imageDescriptions = {
    score: `${score}%`,
    size: prettyBytes(result.totalSize),
    otherSize: prettyBytes(otherLayerSize),
    wastedSize: prettyBytes(wastedSize),
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
    const { name } = item.children[i];
    if (name.includes(keyword)) {
      return true;
    }
  }
  return false;
};

const addToFileTreeView = (
  onToggleExpand: (key: string) => void,
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
  const shouldExpand = (key: string) => {
    if (opt.expandAll) {
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
      if (opt.expandAll || opt.expandItems?.includes(key)) {
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
        </span>
      </li>
    );
    count++;
    if (item.children.length && shouldExpand(item.key)) {
      const tmp = isLastList.slice(0);
      tmp.push(isLast);
      const childAppendCount = addToFileTreeView(
        onToggleExpand,
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

const App: FC = () => {
  const isDarkMode = window.matchMedia("(prefers-color-scheme: dark)").matches;
  // const isDarkMode = false;

  const [messageApi, contextHolder] = message.useMessage();

  const [gotResult, setGotResult] = useState(false);
  const [loading, setLoading] = useState(false);
  const [imageDescriptions, setImageDescriptions] = useState(
    {} as {
      score: string;
      size: string;
      otherSize: string;
      wastedSize: string;
    }
  );
  const [layers, setLayers] = useState([] as Layer[]);
  const [currentLayer, setCurrentLayer] = useState(0);
  const [fileTreeList, setFileTreeList] = useState([] as FileTreeList[][]);
  const [fileTreeViewOption, setFileTreeViewOption] = useState(
    {} as FileTreeViewOption
  );
  const [wastedList, setWastedList] = useState([] as FileWastedSummary[]);
  const [imageName, setImageName] = useState("");

  const onToggleExpand = (key: string) => {
    const opt = Object.assign({}, fileTreeViewOption);
    const items = opt.expandItems || [];
    const index = items.indexOf(key);
    if (index === -1) {
      items.push(key);
    } else {
      items.splice(index, 1);
    }
    opt.expandItems = items;
    setFileTreeViewOption(opt);
  };

  const onSearch = async (value: string) => {
    const image = value.trim();
    if (!image) {
      return;
    }
    setImageName(image);
    setLoading(true);
    try {
      const { data } = await axios.get<ImageAnalyzeResult>(
        `/api/analyze?image=${image}`
      );
      // 为每个file tree item增加key
      data.fileTreeList.forEach((fileTree) => {
        addKeyToFileTreeItem(fileTree, "");
      });

      const result = getImageSummary(data);
      setImageDescriptions(result.imageDescriptions);
      setWastedList(result.wastedList);
      setFileTreeList(data.fileTreeList);
      setLayers(data.layers);
      setCurrentLayer(0);
      // 设置已获取结果
      setGotResult(true);
    } catch (err: any) {
      let message = err?.message as string;
      let axiosErr = err as AxiosError;
      if (axiosErr?.response?.data) {
        let data = axiosErr.response.data as {
          message: string;
        };
        message = data.message || "";
      }
      messageApi.error(message || "analyze image fail", 10);
    } finally {
      setLoading(false);
    }
  };

  const selectLayer = (index: number) => {
    setCurrentLayer(index);
  };

  const getImageSummaryView = () => {
    const imageSummary = (
      <Descriptions title="Image Summary">
        <Descriptions.Item label="Efficiency Score">
          {imageDescriptions["score"]}
        </Descriptions.Item>
        <Descriptions.Item label="Image Size">
          {imageDescriptions["size"]}
        </Descriptions.Item>
        <Descriptions.Item label="Other Layer Size">
          {imageDescriptions["otherSize"]}
        </Descriptions.Item>
        <Descriptions.Item label="Wasted Size">
          {imageDescriptions["wastedSize"]}
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

    let label = `${index + 1}: ${digest.toUpperCase()}(${prettyBytes(size)})`;
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
    fileTreeViewList,
    fileTreeList[currentLayer],
    [],
    fileTreeViewOption
  );

  const layerFilter = (
    <Row gutter={20}>
      <Col span={6}>
        <Form.Item label="Layer">
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
        <Form.Item label="Size">
          <Select
            defaultValue={0}
            options={sizeOptions}
            onChange={(limit: number) => {
              const opt = Object.assign({}, fileTreeViewOption);
              opt.sizeLimit = limit;
              setFileTreeViewOption(opt);
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
              setFileTreeViewOption(opt);
            }}
          >
            Modifications
          </Checkbox>
        </Form.Item>
      </Col>
      <Col span={3}>
        <Form.Item>
          <Checkbox
            onChange={(e) => {
              const opt = Object.assign({}, fileTreeViewOption);
              opt.expandAll = e.target.checked;
              setFileTreeViewOption(opt);
            }}
          >
            Expand
          </Checkbox>
        </Form.Item>
      </Col>
      <Col span={8}>
        <Form.Item>
          <Input
            addonBefore="Keywords"
            allowClear
            onChange={(e) => {
              const opt = Object.assign({}, fileTreeViewOption);
              opt.keyword = e.target.value.trim();
              setFileTreeViewOption(opt);
            }}
          />
        </Form.Item>
      </Col>
    </Row>
  );

  const getLayerContentView = () => {
    let fileTreeListClassName = "fileTree";
    if (isDarkMode) {
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
        <Card title="Layer Content">
          {layerFilter}
          {cmd}
          <ul className={fileTreeListClassName}>
            <li>
              <span>Permission</span>
              <span>UID:GID</span>
              <span>Size</span>
              <span>FileTree</span>
            </li>
            {fileTreeViewList}
          </ul>
        </Card>
      </div>
    );
  };

  const getWastedSummaryView = () => {
    if (wastedList.length === 0) {
      return <></>;
    }
    const list = wastedList.map((item) => {
      return (
        <li key={item.path}>
          <span>{prettyBytes(item.totalSize)}</span>
          <span>{item.count}</span>
          <span>/{item.path}</span>
        </li>
      );
    });
    let className = "wastedList";
    if (isDarkMode) {
      className += " dark";
    }
    return (
      <div className="mtop30">
        <Card title="Wasted Summary">
          <ul className={className}>
            <li>
              <span>Total Size</span>
              <span>Count</span>
              <span>Path</span>
            </li>
            {list}
          </ul>
        </Card>
      </div>
    );
  };
  const getSearchView = () => {
    return (
      <Search
        defaultValue={imageName}
        autoFocus={true}
        loading={loading}
        placeholder="input the name of image"
        allowClear
        enterButton="Analyze"
        size="large"
        onSearch={onSearch}
      />
    );
  };
  let headerClass = "header";
  if (isDarkMode) {
    headerClass += " dark";
  }

  return (
    <ConfigProvider
      theme={{
        algorithm: isDarkMode ? darkAlgorithm : defaultAlgorithm,
      }}
    >
      {contextHolder}
      <Layout>
        {getGithubIcon(isDarkMode)}
        <Header className={headerClass}>
          <div className="contentWrapper">
            <div className="logo">
              <Space>
                {getLogoIcon(isDarkMode)}
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
                Input the name of image to explore each layer in a docker image,
                for example:
                <br />
                redis:alpine, vicanso/diving
                <br />
                quay.io/prometheus/node-exporter
                <br />
                xxx.com/user/image:tag
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
      </Layout>
    </ConfigProvider>
  );
};

export default App;
