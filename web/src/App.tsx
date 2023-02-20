import { useState, FC } from 'react';
import { ConfigProvider, theme, Card, Layout, Input, message, Descriptions, Result, Form, Select, Col, Row, Checkbox } from 'antd';
import axios from 'axios';
import prettyBytes from 'pretty-bytes';

import logo from './assets/logo.png'
import './App.css'


interface ImageAnalyzeResult {
  name: string
  layers: Layer[]
  size: number
  totalSize: number
  fileTreeList: FileTreeList[][]
  fileSummaryList: FileSummaryList[]
}

interface Layer {
  created: string
  digest: string
  cmd: string
  size: number
  unpackSize: number
  empty: boolean
}

interface FileTreeList {
  name: string
  link: string
  size: number
  mode: string
  uid: number
  gid: number
  op: string
  children: Children[]
}

interface Children {
  name: string
  link: string
  size: number
  mode: string
  uid: number
  gid: number
  op: string
  children: Children[]
}

interface FileSummaryList {
  layerIndex: number
  op: string
  info: Info
}

interface Info {
  path: string
  link: string
  size: number
  mode: string
  uid: number
  gid: number
  isWhiteout: any
}
interface FileWastedSummary {
  path: string
  totalSize: number,
  count: number,
}


const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content } = Layout;
const { Search } = Input;


const getImageSummary = (result: ImageAnalyzeResult) => {
  let wastedSize = 0;
  let wastedList: FileWastedSummary[] = [];
  // 计算浪费的空间以及文件
  result.fileSummaryList.forEach((item) => {
    const {
      size,
      path,
    } = item.info;
    const found = wastedList.find(item => item.path === path);
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

  const score = (100 - wastedSize * 100 / result.totalSize).toFixed(2);

  const imageDescriptions = {
    "score": `${score}%`,
    "size": prettyBytes(result.totalSize),
    "otherSize": prettyBytes(otherLayerSize),
    "wastedSize": prettyBytes(wastedSize),
  };
  return {
    imageDescriptions,
  }
};

const App: FC = () => {

  const isDarkMode = window.matchMedia('(prefers-color-scheme: dark)').matches;

  const [messageApi, contextHolder] = message.useMessage();

  const [gotResult, setGotResult] = useState(false);
  const [loading, setLoading] = useState(false);
  const [imageDescriptions, setImageDescriptions] = useState({} as {
    score: string,
    size: string,
    otherSize: string,
    wastedSize: string,
  });
  const [layers, setLayers] = useState([] as Layer[]);
  const [currentLayer, setCurrentLayer] = useState(0);


  const onSearch = async (image: string) => {
    setLoading(true);
    try {
      const { data } = await axios.get<ImageAnalyzeResult>(`/api/analyze?image=${image}`);
      const result = getImageSummary(data);
      setImageDescriptions(result.imageDescriptions);
      setGotResult(true);
      setLayers(data.layers);
      setCurrentLayer(0);
      console.dir(data);
    } catch (err) {
      messageApi.error(err.message || 'analyze image fail');
    } finally {
      setLoading(false);
    }
  };

  const selectLayer = (index: number) => {
    setCurrentLayer(index);
  };

  const imageSummary = <Descriptions title="Image Summary">
    <Descriptions.Item label="Efficiency Score">{imageDescriptions["score"]}</Descriptions.Item>
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

  const subTitle = "Please input the name of image, e.g.: redis:alpine or xxx.com/user/image:tag, it will take a few minutes";

  const layerOptions = layers.map((item, index) => {
    let {
      digest
    } = item;
    if (digest) {
      digest = digest.replace('sha256:', '').substring(0, 8);
    }
    if (!digest) {
      digest = 'none';
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
      label = 'No Limit';
    }
    return {
      value: size,
      label,
    };
  });

  return (
    <ConfigProvider
      theme={{
        algorithm: isDarkMode ? darkAlgorithm : defaultAlgorithm,
      }}>
      {contextHolder}
      <Layout>
        <Header>
          <div className='contentWrapper'>
            <div className='logo'>
              <img src={logo} />Diving
            </div>
            <div className='search'>
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
        {!gotResult && <Result title="Diving" subTitle={subTitle} />}
        {gotResult && <Content>
          <div className='contentWrapper'>
            <div className='imageSummary mtop30'>
              {imageSummary}
            </div>
            <div className='mtop30'>
              <Card title="Layer Content">
                <Row gutter={20}>
                  <Col span={6}>
                    <Form.Item label="Layer">
                      <Select
                        defaultValue={0}
                        style={{
                          width: '100%',
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
                      />
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
                      <Input addonBefore="Keywords"
                        allowClear
                      />
                    </Form.Item>
                  </Col>
                </Row>
              </Card>
            </div>
          </div>
        </Content>}
      </Layout>
    </ConfigProvider>
  )
}

export default App
