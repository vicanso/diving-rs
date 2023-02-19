import { useState, FC } from 'react';
import { ConfigProvider, theme, Card, Layout, Input, Row, Col } from 'antd';

import logo from './assets/logo.png'
import './App.css'

const { defaultAlgorithm, darkAlgorithm } = theme;
const { Header, Content, Footer } = Layout;
const { Search } = Input;

const App: FC = () => {

  const [isDarkMode, setIsDarkMode] = useState(true);

  const [count, setCount] = useState(0)

  const onSearch = (value: string) => {
    console.dir(value);
  };

  const summaryList = [
    {
      title: 'Efficiency Score',
      content: '99.49%',
    },
    {
      title: 'Image Size',
      content: '30.88MB',
    },
    {
      title: 'Other Size',
      content: '20.12MB'
    },
    {
      title: 'Wasted Size',
      content: '192.13KB',
    },
  ].map((item) => {
    return <Col span={6}>
      <Card title={item.title} size='small'>
        <p>{item.content}</p>
      </Card>
    </Col>
  })

  return (
    <ConfigProvider
      theme={{
        algorithm: isDarkMode ? darkAlgorithm : defaultAlgorithm,
      }}>
      <Layout>
        <Header>
          <div className='contentWrapper'>
            <div className='logo'>
              <img src={logo} />Diving
            </div>
            <div className='search'>
              <Search
                placeholder="input the name of image"
                allowClear
                enterButton="Analyze"
                size="large"
                onSearch={onSearch}
              />
            </div>
          </div>
        </Header>
        <div className='contentWrapper'>
          <div className='imageSummary mtop30'>
            <Row gutter={16}>
              {summaryList}
            </Row>
          </div>
          <div className='mtop30'>
            <Card title="Layer Content">
            </Card>
          </div>
        </div>
      </Layout>
    </ConfigProvider>
  )
}

export default App
