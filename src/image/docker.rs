use chrono::{DateTime, Utc};
use lru::LruCache;
use once_cell::sync::OnceCell;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::{ResultExt, Snafu};
use std::{collections::HashMap, convert::From, num::NonZeroUsize, str::FromStr, sync::Mutex};
use tracing::info;

use super::{get_files_from_layer, AnalysisResult, ImageIndex, Layer, Manifest, Platform};
use crate::store::{get_blob_from_file, save_blob_to_file};

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
pub struct DockerManifest {
    pub media_type: String,
    pub schema_version: i64,
    pub config: DockerManifestConfig,
    pub layers: Vec<DockerManifestLayer>,
}

impl From<DockerManifest> for ImageIndex {
    fn from(item: DockerManifest) -> Self {
        let mut manifests = vec![];
        manifests.push(Manifest {
            media_type: item.config.media_type,
            digest: item.config.digest,
            size: item.config.size,
            // TODO 处理platform
            platform: Platform {
                architecture: "amd64".to_string(),
                os: "linux".to_string(),
                ..Default::default()
            },
            ..Default::default()
        });
        // for layer in item.layers {
        //     manifests.push(Manifest {
        //         media_type: layer.media_type,
        //         digest: layer.digest,
        //         size: layer.size,
        //         // TODO 处理platform
        //         ..Default::default()
        //     })
        // }

        ImageIndex {
            media_type: item.media_type,
            schema_version: item.schema_version,
            manifests,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifestConfig {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifestLayer {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifestList {
    pub media_type: String,
    pub schema_version: i64,
    pub manifests: Vec<DockerManifestListManifest>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifestListManifest {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    pub platform: DockerManifestListPlatform,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifestListPlatform {
    pub architecture: String,
    pub os: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerImageConfig {
    pub architecture: String,
    pub created: String,
    pub history: Vec<DockerImageHistory>,
    pub os: String,
    pub rootfs: DockerImageRootfs,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerImageHistory {
    pub created: String,
    #[serde(rename = "created_by")]
    pub created_by: String,
    #[serde(rename = "empty_layer")]
    pub empty_layer: Option<bool>,
    pub comment: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerImageRootfs {
    #[serde(rename = "type")]
    pub type_field: String,
    #[serde(rename = "diff_ids")]
    pub diff_ids: Vec<String>,
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
    for k in key.split(".") {
        let value = root.get(k);
        if value.is_none() {
            return Ok("".to_string());
        }
        root = value.unwrap().to_owned();
    }
    Ok(root.as_str().unwrap_or_else(|| "").to_string())
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
        let url = format!(
            "{}/token?service={}&scope={}",
            self.auth, self.service, scope
        );
        let key = &url.clone();
        if let Some(info) = get_docker_token_from_cache(&url).await {
            if !info.expired() {
                return Ok(info);
            }
        }
        info!(url = url, "Getting token");
        // TODO HTTP请求响应4xx,5xx的处理
        let resp = reqwest::get(url.clone())
            .await
            .context(RequestSnafu { url: url.clone() })?
            .json::<DockerTokenInfo>()
            .await
            .context(JsonSnafu { url: url.clone() })?;
        // 将token缓存，方便后续使用
        set_docker_token_to_cache(key, resp.clone()).await;
        info!(url = url, "Got token");
        Ok(resp)
    }
    // 获取pull时使用的token
    async fn get_pull_token(&self, user: &str, img: &str) -> Result<DockerTokenInfo> {
        let scope = format!("repository:{}/{}:pull", user, img);
        let token = self.get_token(&scope).await?;
        Ok(token)
    }
    // 获取所有的manifest
    pub async fn list_manifest(
        &self,
        user: &str,
        img: &str,
        tag: &str,
    ) -> Result<DockerManifestList> {
        let token = self.get_pull_token(user, img).await?;

        let url = format!("{}/{}/{}/manifests/{}", self.registry, user, img, tag);

        let resp = Client::builder()
            .build()
            .context(BuildSnafu { url: url.clone() })?
            .get(url.clone())
            .header("Authorization", format!("Bearer {}", token.token))
            .header(
                "Accept",
                "application/vnd.docker.distribution.manifest.list.v2+json",
            )
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?
            .json::<DockerManifestList>()
            .await
            .context(JsonSnafu { url })?;

        Ok(resp)
    }
    // 获取manifest
    pub async fn get_manifest(&self, user: &str, img: &str, tag: &str) -> Result<ImageIndex> {
        let token = self.get_pull_token(user, img).await?;

        let url = format!("{}/{}/{}/manifests/{}", self.registry, user, img, tag);
        info!(url = url, "Getting manifest");
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", token.token),
        );
        headers.insert(
            "Accept".to_string(),
            "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.v2+json".to_string(),
        );
        let data = self.get_bytes(url.clone(), headers).await?;
        println!("{:?}", data);
        let media_type = get_value_from_json(&data, "mediaType")?;
        let resp = if media_type.contains("docker") {
            let result: DockerManifest =
                serde_json::from_slice(&data).context(SerdeJsonSnafu {})?;
            result.into()
        } else {
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
    ) -> Result<DockerImageConfig> {
        let manifest = self.get_manifest(user, img, tag).await?;
        // 暂时只获取amd64, linux
        let data = self
            .get_blob(user, img, &manifest.get_config_digest("amd64", "linux"))
            .await?;
        println!("{}", &manifest.get_config_digest("amd64", "linux"));
        println!(">>>>>>>>>>>>>>config");
        println!("{:?}", std::string::String::from_utf8_lossy(&data));
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
        let url = format!("{}/{}/{}/blobs/{}", self.registry, user, img, digest);
        info!(url = url, "Getting blob");
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), format!("Bearer {}", token.token));
        let resp = self.get_bytes(url.clone(), headers).await?;
        // let resp = Client::builder()
        //     .build()
        //     .context(BuildSnafu { url: url.clone() })?
        //     .get(url.clone())
        //     .header("Authorization", format!("Bearer {}", token.token))
        //     .send()
        //     .await
        //     .context(RequestSnafu { url: url.clone() })?
        //     .bytes()
        //     .await
        //     .context(BytesSnafu { url: url.clone() })?;

        // 出错忽略
        // 写入数据失败不影响后续
        let _ = save_blob_to_file(digest, &resp).await;
        info!(url = url, "Got blob");
        Ok(resp.to_vec())
    }
    // 分析镜像
    pub async fn analyze(&self, user: &str, img: &str, tag: &str) -> Result<AnalysisResult> {
        let manifest = self.get_manifest(user, img, tag).await?;
        let config = self.get_image_config(user, img, tag).await?;
        let mut layers = vec![];
        let mut index = 0;
        info!(user = user, img = img, tag = tag, "analyzing image",);
        for history in &config.history {
            let empty = history.empty_layer.unwrap_or_default();
            let mut digest = "".to_string();
            let mut files = vec![];
            let mut size = 0;
            // 只有空的layer需要获取files
            if !empty {
                // if let Some(value) = manifest.layers.get(index) {
                //     size = value.size;
                //     digest = value.digest.clone();
                //     // 判断是否压缩
                //     let buf = self.get_blob(user, img, &digest).await?;

                //     let is_gzip = value.media_type.contains("gzip");
                //     files = get_files_from_layer(&buf, is_gzip)
                //         .await
                //         .context(LayerSnafu {})?;
                // }
                // index += 1;
            }

            layers.push(Layer {
                created: history.created.clone(),
                cmd: history.created_by.clone(),
                empty,
                digest,
                files,
                size,
            });
        }
        info!(user = user, img = img, tag = tag, "analyze image done",);

        Ok(AnalysisResult {
            created: config.created,
            architecture: config.architecture,
            layers,
        })
    }
}
