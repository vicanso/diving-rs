import { useState, FC, Component } from "react";
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
} from "antd";
import axios from "axios";
import prettyBytes from "pretty-bytes";
import { nanoid } from "nanoid";

import logo from "./assets/logo.png";
import "./App.css";

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
  op: string;
  children: FileTreeList[];
}

interface FileSummaryList {
  layerIndex: number;
  op: string;
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

const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;
const { Search } = Input;

const iconStyle = {
  verticalAlign: "middle",
  margin: "-2px 5px 0 0",
};
const plusOutlined = (
  <svg
    viewBox="64 64 896 896"
    focusable="false"
    fill="currentColor"
    height="14px"
    aria-hidden="true"
    style={iconStyle}
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
    style={iconStyle}
  >
    <path d="M328 544h368c4.4 0 8-3.6 8-8v-48c0-4.4-3.6-8-8-8H328c-4.4 0-8 3.6-8 8v48c0 4.4 3.6 8 8 8z"></path>
    <path d="M880 112H144c-17.7 0-32 14.3-32 32v736c0 17.7 14.3 32 32 32h736c17.7 0 32-14.3 32-32V144c0-17.7-14.3-32-32-32zm-40 728H184V184h656v656z"></path>
  </svg>
);

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
    imageDescriptions,
  };
};

const addKeyToFileTreeItem = (items: FileTreeList[]) => {
  items.forEach((item) => {
    item.key = nanoid();
    addKeyToFileTreeItem(item.children);
  });
};

const addToFileTreeView = (
  list: JSX.Element[],
  items: FileTreeList[],
  isLastList: boolean[]
) => {
  if (!items) {
    return 0;
  }
  const max = items.length;
  let count = 0;
  items.forEach((item, index) => {
    const id = `${item.uid}:${item.gid}`;
    const isLast = index === max - 1;
    let name = item.name;
    if (item.link) {
      name = `${name} → ${item.link}`;
    }
    const padding = isLastList.length * 30;
    let icon: JSX.Element = <></>;
    if (item.children.length) {
      icon = plusOutlined;
    }
    list.push(
      <li key={item.key}>
        <span>{item.mode}</span>
        <span>{id}</span>
        <span>{prettyBytes(item.size)}</span>
        <span
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
    if (item.children.length) {
      const tmp = isLastList.slice(0);
      tmp.push(isLast);
      const childAppendCount = addToFileTreeView(list, item.children, tmp);
      // 如果子文件一个都没有插入
      // 则将当前目录也删除
      if (childAppendCount === 0) {
        list.pop();
        count -= 1;
      }
    }
  });
  return count;
};

const App: FC = () => {
  const isDarkMode = window.matchMedia("(prefers-color-scheme: dark)").matches;

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

  const onSearch = async (image: string) => {
    setLoading(true);
    try {
      const { data } = await axios.get<ImageAnalyzeResult>(
        `/api/analyze?image=${image}`
      );
      // 为每个file tree item增加key
      data.fileTreeList.forEach(addKeyToFileTreeItem);

      const result = getImageSummary(data);
      setImageDescriptions(result.imageDescriptions);
      setFileTreeList(data.fileTreeList);
      setLayers(data.layers);
      setCurrentLayer(0);
      // 设置已获取结果
      setGotResult(true);
      console.dir(data);
    } catch (err) {
      messageApi.error(err.message || "analyze image fail");
    } finally {
      setLoading(false);
    }
  };

  const selectLayer = (index: number) => {
    setCurrentLayer(index);
  };

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
  addToFileTreeView(fileTreeViewList, fileTreeList[currentLayer], []);

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
          <Select defaultValue={0} options={sizeOptions} />
        </Form.Item>
      </Col>
      <Col span={3}>
        <Form.Item>
          <Checkbox>Modifications</Checkbox>
        </Form.Item>
      </Col>
      <Col span={3}>
        <Form.Item>
          <Checkbox>Expand</Checkbox>
        </Form.Item>
      </Col>
      <Col span={8}>
        <Form.Item>
          <Input addonBefore="Keywords" allowClear />
        </Form.Item>
      </Col>
    </Row>
  );

  let fileTreeListClassName = "fileTree";
  if (isDarkMode) {
    fileTreeListClassName += " dark";
  }

  return (
    <ConfigProvider
      theme={{
        algorithm: isDarkMode ? darkAlgorithm : defaultAlgorithm,
      }}
    >
      {contextHolder}
      <Layout>
        <Header>
          <div className="contentWrapper">
            <div className="logo">
              <img src={logo} />
              Diving
            </div>
            <div className="search">
              <Search
                loading={loading}
                placeholder="input the name of image"
                allowClear
                enterButton="Analyze"
                size="large"
                onSearch={onSearch}
              />
            </div>
          </div>
        </Header>
        {!gotResult && <p>TODO 首次搜索框居中</p>}
        {gotResult && (
          <Content>
            <div className="contentWrapper">
              <div className="imageSummary mtop30">{imageSummary}</div>
              <div className="mtop30">
                <Card title="Layer Content">
                  {layerFilter}
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
            </div>
          </Content>
        )}
      </Layout>
    </ConfigProvider>
  );
};

export default App;
