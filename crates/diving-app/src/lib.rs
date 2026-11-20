//! Shared configuration, AI reporting, logging, and multi-channel
//! notification layer used by both `diving` (CLI/TUI) and `diving-web`.

pub mod ai;
pub mod config;
pub mod env_util;
pub mod logging;
pub mod notify;
pub mod summary;
pub mod wecom;
