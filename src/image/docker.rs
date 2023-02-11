use chrono::{DateTime, Utc};
use lru::LruCache;
use once_cell::sync::OnceCell;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, num::NonZeroUsize, str::FromStr, sync::Mutex, time::Duration};
use tracing::info;

use super::{
    get_files_from_layer, image::ImageFileSummary, layer::ImageLayerInfo, FileTreeItem,
    ImageConfig, ImageIndex, ImageLayer, ImageManifest, Op, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
    MEDIA_TYPE_IMAGE_INDEX,
};
use crate::{
    image::{find_file_tree_item, ImageFileInfo},
    store::{get_blob_from_file, save_blob_to_file},
};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Build request {} fail: {}", url, source))]
    Build { source: reqwest::Error, url: String },
    #[snafu(display("Request {} fail: {}", url, source))]
    Request { source: reqwest::Error, url: String },
    #[snafu(display("Parse {} json fail: {}", url, source))]
    Json { source: reqwest::Error, url: String },
    #[snafu(display("Get {} bytes fail: {}", url, source))]
    Bytes { source: reqwest::Error, url: String },
    #[snafu(display("Serde json fail: {}", source))]
    SerdeJson { source: serde_json::Error },
    #[snafu(display("Layer handle fail: {}", source))]
    Layer { source: super::layer::Error },
    #[snafu(display("Request {} code: {} fail: {}", url, code, message))]
    Docker {
        message: String,
        code: String,
        url: String,
    },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

static REGISTRY: &str = "https://index.docker.io/v2";
static AUTH: &str = "https://auth.docker.io";
static SERVICE: &str = "registry.docker.io";

#[derive(Debug, Clone, Default)]
pub struct DockerClient {
    registry: String,
    auth: String,
    service: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerTokenInfo {
    token: String,
    access_token: String,
    expires_in: i32,
    issued_at: String,
}

pub struct DockerAnalyzeResult {
    // 镜像名称
    pub name: String,
    // 镜像分层数据
    pub layers: Vec<ImageLayer>,
    // 镜像大小
    pub size: u64,
    // 镜像分层解压大小
    pub total_size: u64,
    // 镜像分层对应的文件树
    pub file_tree_list: Vec<Vec<FileTreeItem>>,
    // 镜像删除与更新文件汇总
    pub file_summary_list: Vec<ImageFileSummary>,
}

impl DockerTokenInfo {
    // 判断docker token是否已过期
    fn expired(&self) -> bool {
        if let Ok(value) = DateTime::<Utc>::from_str(self.issued_at.as_str()) {
            // 因为后续需要使用token获取数据
            // 因此提交10秒认为过期，避免请求时失效
            let offset = (self.expires_in - 10) as i64;
            let now = Utc::now().timestamp();
            return value.timestamp() + offset <= now;
        }
        false
    }
}

// 获取docker token的缓存实例
fn get_docker_token_cache() -> &'static Mutex<LruCache<String, DockerTokenInfo>> {
    static DOCKER_TOKEN_CACHE: OnceCell<Mutex<LruCache<String, DockerTokenInfo>>> = OnceCell::new();
    DOCKER_TOKEN_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(100).unwrap());
        Mutex::new(c)
    })
}

// 从缓存中获取docker token
async fn get_docker_token_from_cache(key: &String) -> Option<DockerTokenInfo> {
    if let Ok(mut cache) = get_docker_token_cache().lock() {
        if let Some(info) = cache.get(key) {
            return Some(info.clone());
        }
    }
    Option::None
}

// 将docker token写入缓存
async fn set_docker_token_to_cache(key: &String, info: DockerTokenInfo) {
    // 失败忽略
    if let Ok(mut cache) = get_docker_token_cache().lock() {
        cache.put(key.to_string(), info);
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerRequestErrorResp {
    pub errors: Vec<DockerRequestError>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerRequestError {
    pub code: String,
    pub message: String,
}

fn get_value_from_json(v: &[u8], key: &str) -> Result<String> {
    let mut root: Value = serde_json::from_slice(v).context(SerdeJsonSnafu {})?;
    for k in key.split('.') {
        let value = root.get(k);
        if value.is_none() {
            return Ok("".to_string());
        }
        root = value.unwrap().to_owned();
    }
    Ok(root.as_str().unwrap_or("").to_string())
}

fn add_to_file_summary(
    file_summary_list: &mut Vec<ImageFileSummary>,
    layer_index: usize,
    files: &[ImageFileInfo],
    file_tree_list: &[Vec<FileTreeItem>],
) {
    for file in files.iter() {
        for items in file_tree_list.iter() {
            let arr: Vec<&str> = file.path.split('/').collect();
            if let Some(found) = find_file_tree_item(items, arr) {
                // 以前已存在，因此为修改或删除
                // 文件删除
                let mut op = Op::Modified;
                let mut info = file.clone();
                if file.is_whiteout.is_some() {
                    op = Op::Remove;
                    info.size = found.size;
                }
                file_summary_list.push(ImageFileSummary {
                    layer_index,
                    op,
                    info,
                });
            }
        }
    }
}

impl DockerClient {
    pub fn new() -> Self {
        DockerClient {
            registry: REGISTRY.to_string(),
            auth: AUTH.to_string(),
            service: SERVICE.to_string(),
        }
    }
    async fn get_bytes(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<bytes::Bytes> {
        let mut builder = Client::builder()
            .build()
            .context(BuildSnafu { url: url.clone() })?
            .get(url.clone());
        builder = builder.timeout(Duration::from_secs(5 * 60));
        for (key, value) in headers {
            builder = builder.header(key, value);
        }
        let resp = builder
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?;
        if resp.status().as_u16() >= 400 {
            let err = resp
                .json::<DockerRequestErrorResp>()
                .await
                .context(JsonSnafu { url: url.clone() })?;
            return Err(Error::Docker {
                message: err.errors[0].message.clone(),
                code: err.errors[0].code.clone(),
                url: url.clone(),
            });
        }

        let result = resp.bytes().await.context(JsonSnafu { url: url.clone() })?;
        Ok(result)
    }
    async fn get<T: DeserializeOwned>(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<T> {
        let data = self.get_bytes(url.clone(), headers).await?;
        let result = serde_json::from_slice(&data).context(SerdeJsonSnafu {})?;
        Ok(result)
    }
    pub fn new_custom(register: &str, auth: &str, service: &str) -> Self {
        DockerClient {
            registry: register.to_string(),
            auth: auth.to_string(),
            service: service.to_string(),
        }
    }
    // 获取docker的token
    async fn get_token(&self, scope: &String) -> Result<DockerTokenInfo> {
        let url = format!("{}/token?service={}&scope={scope}", self.auth, self.service);
        let key = &url.clone();
        if let Some(info) = get_docker_token_from_cache(&url).await {
            if !info.expired() {
                return Ok(info);
            }
        }
        info!(url = url, "Getting token");
        let resp = self
            .get::<DockerTokenInfo>(url.clone(), HashMap::new())
            .await?;
        // 将token缓存，方便后续使用
        set_docker_token_to_cache(key, resp.clone()).await;
        info!(url = url, "Got token");
        Ok(resp)
    }
    // 获取pull时使用的token
    async fn get_pull_token(&self, user: &str, img: &str) -> Result<DockerTokenInfo> {
        let scope = format!("repository:{user}/{img}:pull");
        let token = self.get_token(&scope).await?;
        Ok(token)
    }
    // 获取manifest
    pub async fn get_manifest(&self, user: &str, img: &str, tag: &str) -> Result<ImageManifest> {
        // TODO 如果tag非latest，是否可以缓存
        // 需要注意以命令行或以web server执行的程序生命周期的差别
        let token = self.get_pull_token(user, img).await?;

        // 根据tag获取manifest
        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
        info!(url = url, "Getting manifest");
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", token.token),
        );
        // 支持的类型
        let accepts = vec![MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST];

        headers.insert("Accept".to_string(), accepts.join(", "));
        let data = self.get_bytes(url.clone(), headers).await?;
        let media_type = get_value_from_json(&data, "mediaType")?;
        let resp = if media_type == MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST {
            // docker的版本则可直接返回
            serde_json::from_slice(&data).context(SerdeJsonSnafu {})?
        } else {
            let manifest = serde_json::from_slice::<ImageIndex>(&data)
                .context(SerdeJsonSnafu {})?
                // TODO 后续是否可根据系统或客户自动选择
                .guess_manifest();
            let mut headers = HashMap::new();
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", token.token),
            );
            headers.insert("Accept".to_string(), manifest.media_type);
            // 根据digest再次获取
            let url = format!(
                "{}/{user}/{img}/manifests/{}",
                self.registry, manifest.digest
            );
            let data = self.get_bytes(url.clone(), headers).await?;
            serde_json::from_slice(&data).context(SerdeJsonSnafu {})?
        };
        info!(url = url, "Got manifest");
        Ok(resp)
    }
    // 获取镜像的信息
    pub async fn get_image_config(&self, user: &str, img: &str, tag: &str) -> Result<ImageConfig> {
        let manifest = self.get_manifest(user, img, tag).await?;
        // 暂时只获取amd64, linux
        let data = self.get_blob(user, img, &manifest.config.digest).await?;
        let result = serde_json::from_slice(&data.to_vec()).context(SerdeJsonSnafu {})?;
        Ok(result)
    }
    // 获取镜像分层的blob
    pub async fn get_blob(&self, user: &str, img: &str, digest: &str) -> Result<Vec<u8>> {
        let token = self.get_pull_token(user, img).await?;
        // 是否需要加锁避免同时读写
        // 忽略出错，如果出错直接从网络加载
        if let Ok(data) = get_blob_from_file(digest).await {
            return Ok(data);
        }
        let url = format!("{}/{user}/{img}/blobs/{digest}", self.registry);
        info!(url = url, "Getting blob");
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", token.token),
        );
        let resp = self.get_bytes(url.clone(), headers).await?;

        // 出错忽略
        // 写入数据失败不影响后续
        let _ = save_blob_to_file(digest, &resp).await;
        info!(url = url, "Got blob");
        Ok(resp.to_vec())
    }
    pub async fn analyze(&self, user: &str, img: &str, tag: &str) -> Result<DockerAnalyzeResult> {
        let manifest = self.get_manifest(user, img, tag).await?;
        let config = self.get_image_config(user, img, tag).await?;
        let mut layers = vec![];
        // let mut layer_infos = vec![];
        let mut file_tree_list: Vec<Vec<FileTreeItem>> = vec![];
        let mut index = 0;
        let mut file_summary_list = vec![];
        info!(user = user, img = img, tag = tag, "analyzing image",);

        let mut image_size = 0;
        let mut image_total_size = 0;
        for (layer_index, history) in config.history.iter().enumerate() {
            let empty = history.empty_layer.unwrap_or_default();
            let mut digest = "".to_string();
            let mut info = ImageLayerInfo {
                ..Default::default()
            };
            let mut size = 0;
            // 只有非空的layer需要获取files
            if !empty {
                // manifest中的layer只对应非空的操作
                if let Some(value) = manifest.layers.get(index) {
                    size = value.size;
                    digest = value.digest.clone();
                    // 判断是否压缩
                    let buf = self.get_blob(user, img, &digest).await?;

                    info = get_files_from_layer(&buf, &value.media_type)
                        .await
                        .context(LayerSnafu {})?;
                    if layer_index != 0 {
                        add_to_file_summary(
                            &mut file_summary_list,
                            layer_index,
                            &info.files,
                            &file_tree_list,
                        );
                    }
                    image_size += info.size;
                    image_total_size += info.unpack_size;
                }
                index += 1;
            }

            layers.push(ImageLayer {
                created: history.created.clone(),
                cmd: history.created_by.clone(),
                empty,
                digest,
                unpack_size: info.unpack_size,
                size,
            });
            file_tree_list.push(info.to_file_tree());
        }

        Ok(DockerAnalyzeResult {
            name: format!("{user}/{img}:{tag}"),
            layers,
            size: image_size,
            total_size: image_total_size,
            file_tree_list,
            file_summary_list,
        })
    }
}
