use bytes::Bytes;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};

static REGISTRY: &str = "https://index.docker.io/v2";
static AUTH: &str = "https://auth.docker.io";
static SERVICE: &str = "registry.docker.io";

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
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

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

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerManifest {
    pub media_type: String,
    pub schema_version: i64,
    pub config: DockerManifestConfig,
    pub layers: Vec<DockerManifestLayer>,
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

impl DockerClient {
    pub fn new() -> Self {
        DockerClient {
            registry: REGISTRY.to_string(),
            auth: AUTH.to_string(),
            service: SERVICE.to_string(),
        }
    }
    async fn get_token(&self, scope: &String) -> Result<DockerTokenInfo> {
        let url = format!(
            "{}/token?service={}&scope={}",
            self.auth, self.service, scope
        );
        // TODO HTTP请求响应4xx,5xx的处理
        let resp = reqwest::get(url.clone())
            .await
            .context(RequestSnafu { url: url.clone() })?
            .json::<DockerTokenInfo>()
            .await
            .context(JsonSnafu { url })?;
        Ok(resp)
    }
    async fn get_pull_token(&self, user: &str, img: &str) -> Result<DockerTokenInfo> {
        let scope = format!("repository:{}/{}:pull", user, img);
        let token = self.get_token(&scope).await?;
        Ok(token)
    }

    pub async fn get_manifest(&self, user: &str, img: &str, tag: &str) -> Result<DockerManifest> {
        let token = self.get_pull_token(user, img).await?;

        let url = format!("{}/{}/{}/manifests/{}", self.registry, user, img, tag);

        let resp = Client::builder()
            .build()
            .context(BuildSnafu { url: url.clone() })?
            .get(url.clone())
            .header("Authorization", format!("Bearer {}", token.token))
            .header(
                "Accept",
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?
            .json::<DockerManifest>()
            .await
            .context(JsonSnafu { url })?;
        Ok(resp)
    }
    pub async fn get_blob(&self, user: &str, img: &str, digest: &str) -> Result<Bytes> {
        let token = self.get_pull_token(user, img).await?;
        let url = format!("{}/{}/{}/blobs/{}", self.registry, user, img, digest);
        let resp = Client::builder()
            .build()
            .context(BuildSnafu { url: url.clone() })?
            .get(url.clone())
            .header("Authorization", format!("Bearer {}", token.token))
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?
            .bytes()
            .await
            .context(BytesSnafu { url: url.clone() })?;

        Ok(resp)
    }
}
