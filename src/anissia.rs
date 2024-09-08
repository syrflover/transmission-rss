use std::fmt::Debug;

use bytes::Bytes;
use chrono::{DateTime, Local};
use reqwest::StatusCode;
use serde::Deserialize;
use tap::Pipe;
use tl::ParserOptions;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reqwest: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("tl: {0}")]
    Tl(#[from] tl::ParseError),

    #[error("status: {0} - {1}")]
    Status(StatusCode, String),
}

mod sealed {
    use std::fmt::Debug;

    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub(super) struct ResponseDataInner<T: Debug> {
        pub content: T,
    }
}

#[derive(Debug, Deserialize)]
struct ResponseData<T: Debug> {
    pub data: sealed::ResponseDataInner<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionInfo {
    pub anime_no: u32,
    pub subject: String,
    pub episode: String,
    #[serde(rename = "updDt")]
    pub updated_at: DateTime<Local>,
    pub website: String,
    #[serde(rename = "name")]
    pub translator: String,
}

impl CaptionInfo {
    pub async fn download(&self, selector: &str) -> Result<Option<Bytes>, Error> {
        let website_resp = reqwest::get(&self.website).await?;

        if !website_resp.status().is_success() {
            return Err(Error::Status(
                website_resp.status(),
                website_resp.text().await.unwrap_or_default(),
            ));
        }

        let website = website_resp.text().await?;

        let dom = tl::parse(&website, ParserOptions::default())?;

        let parser = dom.parser();

        let element = match dom
            .query_selector(selector)
            .and_then(|r| r.next()?.get(&parser)?.as_tag()?.pipe(Some))
        {
            Some(r) => r,
            None => return Ok(None),
        };

        let href = match element
            .attributes()
            .get("href")
            .and_then(|r| String::from_utf8(r?.as_bytes().to_vec()).ok())
        {
            Some(r) => r,
            None => return Ok(None),
        };

        let resp = reqwest::get(&href).await?;

        Ok()
    }
}

// TODO: https://github.com/dhku/SMI-Auto-Downloader/blob/main/subs.py

struct GoogleDrive {}

impl GoogleDrive {
    pub async fn download(url: &str) -> Result<Bytes, Error> {}
}

const ANISSIA_URL: &str = "https://api.anissia.net";

// https://api.anissia.net/anime/caption/recent/0
// res_json.data.content

pub async fn get_recent_captions(page: usize) -> Result<Vec<CaptionInfo>, Error> {
    let url = format!("{}/anime/caption/recent/{}", ANISSIA_URL, page);

    let resp = reqwest::get(url).await?;

    if !resp.status().is_success() {
        return Err(Error::Status(
            resp.status(),
            resp.text().await.unwrap_or_default(),
        ));
    }

    let res = resp.json::<ResponseData<Vec<CaptionInfo>>>().await?;

    Ok(res.data.content)
}
