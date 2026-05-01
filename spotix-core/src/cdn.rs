use std::{
    io,
    io::Read,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::session::login5::Login5;
use crate::{
    error::Error,
    item_id::FileId,
    session::{SessionService, client_token::ClientTokenProvider},
    system_info::{OS, SPOTIFY_SEMANTIC_VERSION},
    util::default_ureq_agent_builder,
};
use ureq::http::StatusCode;

pub type CdnHandle = Arc<Cdn>;

pub struct Cdn {
    session: SessionService,
    agent: ureq::Agent,
    login5: Login5,
    client_token_provider: ClientTokenProvider,
}

impl Cdn {
    pub fn new(session: SessionService, proxy_url: Option<&str>) -> Result<CdnHandle, Error> {
        let agent = default_ureq_agent_builder(proxy_url)
            .http_status_as_error(false)
            .build();
        Ok(Arc::new(Self {
            session,
            agent: agent.into(),
            login5: Login5::new(None, proxy_url),
            client_token_provider: ClientTokenProvider::new(proxy_url),
        }))
    }

    pub fn resolve_audio_file_url(&self, id: FileId) -> Result<CdnUrl, Error> {
        const MAX_ATTEMPTS: u8 = 5;
        const BASE_BACKOFF: Duration = Duration::from_millis(500);
        const MAX_BACKOFF: Duration = Duration::from_secs(10);

        let locations_uri = format!(
            "https://api.spotify.com/v1/storage-resolve/files/audio/interactive/{}",
            id.to_base16()
        );

        let access_token = self.login5.get_access_token(&self.session)?;
        let mut attempts = 0;
        let mut backoff = BASE_BACKOFF;

        let response = loop {
            let mut request = self
                .agent
                .get(&locations_uri)
                .query("version", "10000000")
                .query("product", "9")
                .query("platform", "39")
                .query("alt", "json")
                .header(
                    "Authorization",
                    &format!("Bearer {}", access_token.access_token),
                )
                .header("User-Agent", &Self::user_agent());

            match self.client_token_provider.get() {
                Ok(client_token) => {
                    request = request.header("client-token", &client_token);
                }
                Err(err) => {
                    log::warn!("cdn: failed to get client token: {err}");
                }
            }

            let response = request.call();
            let response = match response {
                Ok(resp) => resp,
                Err(err) => {
                    return Err(Error::AudioFetchingError(Box::new(err)));
                }
            };

            match response.status() {
                StatusCode::TOO_MANY_REQUESTS => {
                    let retry_after = response
                        .headers()
                        .get("Retry-After")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let delay = retry_after.unwrap_or(backoff);
                    log::warn!(
                        "cdn: rate limited (HTTP 429), retrying in {}s",
                        delay.as_secs()
                    );
                    if attempts < MAX_ATTEMPTS {
                        thread::sleep(delay);
                        attempts += 1;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                    return Err(Error::AudioFetchingError(Box::new(io::Error::other(
                        format!(
                            "storage-resolve rate limited (HTTP 429), retry in {}s",
                            delay.as_secs()
                        ),
                    ))));
                }
                StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
                    if attempts < MAX_ATTEMPTS {
                        thread::sleep(backoff);
                        attempts += 1;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                    return Err(Error::AudioFetchingError(Box::new(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "storage-resolve timed out",
                    ))));
                }
                status if status.is_server_error() => {
                    if attempts < MAX_ATTEMPTS {
                        thread::sleep(backoff);
                        attempts += 1;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                    return Err(Error::AudioFetchingError(Box::new(io::Error::other(
                        format!("storage-resolve server error: {}", status.as_u16()),
                    ))));
                }
                status if status.is_client_error() => {
                    return Err(Error::AudioFetchingError(Box::new(io::Error::other(
                        format!("storage-resolve error: {}", status.as_u16()),
                    ))));
                }
                _ => break response,
            }
        };

        #[derive(Deserialize)]
        struct AudioFileLocations {
            cdnurl: Vec<String>,
        }

        // Deserialize the response and pick a file URL from the returned CDN list.
        let locations: AudioFileLocations = response.into_body().read_json()?;
        let file_uri = locations
            .cdnurl
            .into_iter()
            // TODO:
            //  Now, we always pick the first URL in the list, figure out a better strategy.
            //  Choosing by random seems wrong.
            .next()
            // TODO: Avoid panicking here.
            .expect("No file URI found");

        let uri = CdnUrl::new(file_uri);
        Ok(uri)
    }

    pub fn fetch_file_range(
        &self,
        uri: &str,
        offset: u64,
        length: u64,
    ) -> Result<(u64, impl Read + use<>), Error> {
        let response = self
            .agent
            .get(uri)
            .header("Range", &range_header(offset, length))
            .call()?;
        let total_length = parse_total_content_length(&response);
        let data_reader = response.into_body().into_reader();
        Ok((total_length, data_reader))
    }
}

#[derive(Clone)]
pub struct CdnUrl {
    pub url: String,
    pub expires: Instant,
}

impl CdnUrl {
    // In case we fail to parse the expiration time from URL, this default is used.
    const DEFAULT_EXPIRATION: Duration = Duration::from_secs(60 * 30);

    // Consider URL expired even before the official expiration time.
    const EXPIRATION_TIME_THRESHOLD: Duration = Duration::from_secs(5);

    fn new(url: String) -> Self {
        let expires_in = parse_expiration(&url).unwrap_or_else(|| {
            log::warn!("failed to parse expiration time from URL {:?}", &url);
            Self::DEFAULT_EXPIRATION
        });
        let expires = Instant::now() + expires_in;
        Self { url, expires }
    }

    pub fn is_expired(&self) -> bool {
        self.expires.saturating_duration_since(Instant::now()) < Self::EXPIRATION_TIME_THRESHOLD
    }
}

impl Cdn {
    fn user_agent() -> String {
        let platform = match OS {
            "macos" => "OSX",
            "windows" => "Win32",
            _ => "Linux",
        };
        format!(
            "Spotify/{} {}/0 (spotix/{})",
            SPOTIFY_SEMANTIC_VERSION,
            platform,
            env!("CARGO_PKG_VERSION")
        )
    }
}

impl From<ureq::Error> for Error {
    fn from(err: ureq::Error) -> Self {
        Error::AudioFetchingError(Box::new(err))
    }
}

/// Constructs a Range header value for given offset and length.
fn range_header(offfset: u64, length: u64) -> String {
    let last_byte = offfset + length - 1; // Offset of the last byte of the range is inclusive.
    format!("bytes={offfset}-{last_byte}")
}

/// Parses a total content length from a Content-Range response header.
///
/// For example, returns 146515 for a response with header
/// "Content-Range: bytes 0-1023/146515".
fn parse_total_content_length(response: &ureq::http::response::Response<ureq::Body>) -> u64 {
    response
        .headers()
        .get("Content-Range")
        .expect("Content-Range header not found")
        .to_str()
        .expect("Failed to parse Content-Range Header")
        .split('/')
        .next_back()
        .expect("Failed to parse Content-Range Header")
        .parse()
        .expect("Failed to parse Content-Range Header")
}

/// Parses an expiration of an audio file URL.
fn parse_expiration(url: &str) -> Option<Duration> {
    let token_exp = url.split("__token__=exp=").nth(1);
    let expires_millis = if let Some(token_exp) = token_exp {
        // Parse from the expiration token param
        token_exp.split('~').next()?
    } else if let Some(verify_exp) = url.split("verify=").nth(1) {
        // Parse from verify parameter (new spotifycdn.com format)
        verify_exp.split('-').next()?
    } else {
        // Parse from the first param
        let first_param = url.split('?').nth(1)?;
        first_param.split('_').next()?
    };
    let expires_millis = expires_millis.parse().ok()?;
    let expires = Duration::from_millis(expires_millis);
    Some(expires)
}
