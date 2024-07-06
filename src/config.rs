use std::{env, fmt::Debug, str::FromStr};

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

#[derive(Debug, Deserialize)]
pub struct Config {
    pub channels_config_url: String,
    pub transmission_url: String,
}

impl Config {
    pub fn new() -> Self {
        Self {
            channels_config_url: env("CHANNELS_CONFIG_URL"),
            transmission_url: env("TRANSMISSION_URL"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ChannelConfig {
    pub url: String,
    pub rules: Vec<Rule>,
}
