use home::home_dir;
use snafu::{ResultExt, Snafu};
use tokio::fs;
#[derive(Debug, Snafu)]
pub enum Error {}
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub async fn get_blob(name: &str) -> Result<Vec<u8>> {
    Ok(vec![])
    // fs::read(path)
}
