use std::{env, fmt::Debug, path::PathBuf, str::FromStr};

use serde::Deserialize;

use crate::rule::Rule;

fn env<T>(key: &str) -> T
where
    T: FromStr,
    <T as FromStr>::Err: Debug,
{
    let var = match env::var(key) {
        Ok(r) => r,
        Err(_) => panic!("not set {key}"),
    };

    var.parse().expect("Please set dotenv to valid value")
}

fn env_opt<T>(key: &str) -> Option<T>
where
    T: FromStr,
    <T as FromStr>::Err: Debug,
{
    env::var(key)
        .ok()
        .map(|var| var.parse().expect("Please set dotenv to valid value"))
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub channels_config_url: String,
    pub transmission_url: String,

    pub download_dir: Option<String>,
    pub speed_limit_up: Option<i32>,
    pub speed_limit_down: Option<i32>,
    pub download_queue_size: Option<i32>,
    pub seed_queue_size: Option<i32>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            channels_config_url: env("CHANNELS_CONFIG_URL"),
            transmission_url: env("TRANSMISSION_URL"),

            download_dir: env_opt("DOWNLOAD_DIR"),
            speed_limit_up: env_opt("SPEED_LIMIT_UP"),
            speed_limit_down: env_opt("SPEED_LIMIT_DOWN"),
            download_queue_size: env_opt("DOWNLOAD_QUEUE_SIZE"),
            seed_queue_size: env_opt("SEED_QUEUE_SIZE"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ChannelConfig {
    pub url: String,
    pub directory: PathBuf,
    pub rules: Vec<Rule>,
}
