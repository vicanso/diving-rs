use chrono::{DateTime, Utc};
use http::StatusCode;
use lru::LruCache;
use once_cell::sync::OnceCell;
use regex::Regex;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, num::NonZeroUsize, str::FromStr, sync::Mutex, time::Duration};
use tracing::info;

use super::{
    get_files_from_layer,
    layer::ImageLayerInfo,
    oci_image::{ImageFileSummary, ImageManifestLayer},
    FileTreeItem, ImageConfig, ImageIndex, ImageLayer, ImageManifest, Op,
    MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST, MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST,
};
use crate::{
    error::HTTPError,
    image::{convert_files_to_file_tree, find_file_tree_item, ImageFileInfo},
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
    #[snafu(display("Task fail: {}", source))]
    Task { source: tokio::task::JoinError },
    #[snafu(display("Request {} code: {} fail: {}", url, code, message))]
    Docker {
        message: String,
        code: String,
        url: String,
    },
    #[snafu(display("{message}"))]
    Whatever { message: String },
}

impl From<Error> for HTTPError {
    fn from(err: Error) -> Self {
        // 对于部分error单独转换
        HTTPError::new_with_category(&err.to_string(), "docker")
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

static REGISTRY: &str = "https://index.docker.io/v2";

#[derive(Debug, Clone, Default)]
pub struct ImageInfo {
    // 镜像对应的registry
    pub registry: String,
    // 镜像用户
    pub user: String,
    // 镜像名称
    pub name: String,
    // 镜像版本
    pub tag: String,
}

pub fn parse_image_info(image: &str) -> ImageInfo {
    let mut value = image.to_string();
    if !value.contains(':') {
        value += ":latest";
    }
    let mut values: Vec<&str> = value.split(&['/', ':']).collect();
    let tag = values.pop().unwrap_or_default().to_string();
    let mut registry = REGISTRY.to_string();
    let mut user = "library".to_string();
    let mut name = "".to_string();
    match values.len() {
        1 => {
            name = values[0].to_string();
        }
        2 => {
            user = values[0].to_string();
            name = values[1].to_string();
        }
        3 => {
            // 默认仅支持https v2
            registry = format!("https://{}/v2", values[0]);
            user = values[1].to_string();
            name = values[2].to_string();
        }
        _ => {}
    }

    ImageInfo {
        registry,
        user,
        name,
        tag,
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthInfo {
    pub auth: String,
    pub service: String,
    pub scope: String,
}

fn parse_auth_info(auth: &str) -> Result<AuthInfo> {
    let re =
        Regex::new("(?P<key>\\S+?)=\"(?P<value>\\S+?)\",?").map_err(|err| Error::Whatever {
            message: err.to_string(),
        })?;
    let mut auth_info = AuthInfo {
        ..Default::default()
    };
    for caps in re.captures_iter(auth) {
        let value = caps["value"].to_string();
        match &caps["key"] {
            "realm" => auth_info.auth = value,
            "service" => auth_info.service = value,
            "scope" => auth_info.scope = value,
            _ => {}
        }
    }

    Ok(auth_info)
}

#[derive(Debug, Clone, Default)]
pub struct DockerClient {
    registry: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerTokenInfo {
    token: String,
    expires_in: i32,
    issued_at: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        let issued_at = self.issued_at.clone().unwrap_or_default();
        if let Ok(value) = DateTime::<Utc>::from_str(&issued_at) {
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
                    op = Op::Removed;
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
    pub fn new(register: &str) -> Self {
        DockerClient {
            registry: register.to_string(),
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
        if resp.status() >= StatusCode::UNAUTHORIZED {
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
    // 获取manifest
    pub async fn get_manifest(
        &self,
        user: &str,
        img: &str,
        tag: &str,
        token: &str,
    ) -> Result<ImageManifest> {
        // TODO 如果tag非latest，是否可以缓存
        // 需要注意以命令行或以web server执行的程序生命周期的差别

        // 根据tag获取manifest
        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
        info!(url = url, "Getting manifest");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        // 支持的类型
        let accepts = vec![
            MEDIA_TYPE_IMAGE_INDEX,
            MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
            MEDIA_TYPE_MANIFEST_LIST,
        ];

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
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
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
    pub async fn get_image_config(
        &self,
        user: &str,
        img: &str,
        tag: &str,
        token: &str,
    ) -> Result<ImageConfig> {
        let manifest = self.get_manifest(user, img, tag, token).await?;
        // 暂时只获取amd64, linux
        let data = self
            .get_blob(user, img, &manifest.config.digest, token)
            .await?;
        let result = serde_json::from_slice(&data.to_vec()).context(SerdeJsonSnafu {})?;
        Ok(result)
    }
    // 获取镜像分层的blob
    pub async fn get_blob(
        &self,
        user: &str,
        img: &str,
        digest: &str,
        token: &str,
    ) -> Result<Vec<u8>> {
        // 是否需要加锁避免同时读写
        // 忽略出错，如果出错直接从网络加载
        if let Ok(data) = get_blob_from_file(digest).await {
            return Ok(data);
        }
        let url = format!("{}/{user}/{img}/blobs/{digest}", self.registry);
        info!(url = url, "Getting blob");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        let resp = self.get_bytes(url.clone(), headers).await?;

        // 出错忽略
        // 写入数据失败不影响后续
        let _ = save_blob_to_file(digest, &resp).await;
        info!(url = url, "Got blob");
        Ok(resp.to_vec())
    }
    async fn get_layer_files(
        &self,
        user: &str,
        img: &str,
        token: &str,
        layer: ImageManifestLayer,
    ) -> Result<ImageLayerInfo> {
        let buf = self.get_blob(user, img, &layer.digest, token).await?;

        let info = get_files_from_layer(&buf, &layer.media_type)
            .await
            .context(LayerSnafu {})?;
        Ok(info)
    }
    async fn get_all_layer_info(
        &self,
        user: String,
        img: String,
        token: String,
        layers: Vec<ImageManifestLayer>,
    ) -> Result<Vec<ImageLayerInfo>> {
        let s = self.clone();
        let result = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("getAllLayerInfo")
                .worker_threads(layers.len())
                .build()
                .expect("Creating tokio runtime");
            runtime.block_on(async move {
                let mut handles = Vec::with_capacity(layers.len());
                for layer in layers {
                    handles.push(s.get_layer_files(&user, &img, &token, layer));
                }

                let arr = futures::future::join_all(handles).await;
                let mut info_list = vec![];
                for result in arr {
                    let info = result?;
                    info_list.push(info);
                }
                Ok::<Vec<ImageLayerInfo>, Error>(info_list)
            })
        })
        .join()
        .map_err(|_| Error::Whatever {
            message: "thread join error".to_string(),
        })?;
        let infos = result?;
        Ok(infos)
    }
    async fn get_auth_token(&self, user: &str, img: &str, tag: &str) -> Result<String> {
        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
        let mut builder = Client::builder()
            .build()
            .context(BuildSnafu { url: url.clone() })?
            .head(url.clone());
        builder = builder.timeout(Duration::from_secs(5 * 60));
        let resp = builder
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?;
        if resp.status() == StatusCode::UNAUTHORIZED {
            if let Some(value) = resp.headers().get("www-authenticate") {
                let auth_info = parse_auth_info(value.to_str().unwrap_or_default())?;
                let url = format!(
                    "{}?service={}&scope={}",
                    auth_info.auth, auth_info.service, auth_info.scope
                );
                let key = &url.clone();
                if let Some(info) = get_docker_token_from_cache(&url).await {
                    if !info.expired() {
                        return Ok(info.token);
                    }
                }
                info!(url = url, "Getting token");
                let mut resp = self
                    .get::<DockerTokenInfo>(url.clone(), HashMap::new())
                    .await?;
                if resp.issued_at.is_none() {
                    resp.issued_at = Some(Utc::now().to_rfc3339());
                }
                // 将token缓存，方便后续使用
                set_docker_token_to_cache(key, resp.clone()).await;
                info!(url = url, "Got token");
                return Ok(resp.token);
            }
        }
        Ok("".to_string())
    }
    pub async fn analyze(&self, user: &str, img: &str, tag: &str) -> Result<DockerAnalyzeResult> {
        let token = self.get_auth_token(user, img, tag).await?;
        let manifest = self.get_manifest(user, img, tag, &token).await?;
        let config = self.get_image_config(user, img, tag, &token).await?;
        let mut layers = vec![];
        // let mut layer_infos = vec![];
        let mut file_tree_list: Vec<Vec<FileTreeItem>> = vec![];
        let mut index = 0;
        let mut file_summary_list = vec![];
        info!(user = user, img = img, tag = tag, "analyzing image",);

        let mut image_size = 0;
        let mut image_total_size = 0;
        let info_list = self
            .get_all_layer_info(
                user.to_string(),
                img.to_string(),
                token.clone(),
                manifest.layers.clone(),
            )
            .await?;
        for (layer_index, history) in config.history.iter().enumerate() {
            let empty = history.empty_layer.unwrap_or_default();
            let mut digest = "".to_string();
            let mut info = &ImageLayerInfo {
                ..Default::default()
            };
            let mut size = 0;
            let mut file_tree = vec![];
            // 只有非空的layer需要获取files
            if !empty {
                // manifest中的layer只对应非空的操作
                if let Some(value) = manifest.layers.get(index) {
                    info = info_list.get(index).unwrap();
                    size = value.size;
                    digest = value.digest.clone();
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
                    // TODO 根据file summary判断文件是否更新或删除
                    file_tree = convert_files_to_file_tree(&info.files, &file_summary_list);
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
            file_tree_list.push(file_tree);
        }

        info!(user = user, img = img, tag = tag, "analyze image done",);

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

pub async fn analyze_docker_image(image_info: ImageInfo) -> Result<DockerAnalyzeResult> {
    let c = DockerClient::new(&image_info.registry);
    c.analyze(&image_info.user, &image_info.name, &image_info.tag)
        .await
}
