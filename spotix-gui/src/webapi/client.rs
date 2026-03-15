use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    future::Future,
    io::{self, Read},
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use druid::{
    Data, ImageBuf,
    im::Vector,
    image::{self, ImageFormat},
};

use chrono::{Duration as ChronoDuration, Utc};
use itertools::Itertools;
use librespot_core::{
    Session as LibrespotSession, authentication::Credentials as LibrespotCredentials,
    config::SessionConfig as LibrespotSessionConfig,
};
use log::info;
use parking_lot::{Condvar, Mutex};
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{
    AlbumType as RSpotifyAlbumType, ArtistId, Market, PlayableItem, PlaylistId, SearchType,
    TimeRange,
};
use rspotify::prelude::Id;
use rspotify::{ClientError, Token as RSpotifyToken};
use rspotify_http::HttpError as RSpotifyHttpError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use spotix_core::{
    oauth::{self, OAuthToken},
    session::client_token::ClientTokenProvider,
    session::{SessionService, login5::Login5},
    system_info::{OS, SPOTIFY_SEMANTIC_VERSION},
};
use std::sync::OnceLock;
use time::{Date, Month};
use ureq::{
    Agent, Body,
    http::{Response, StatusCode},
};

use crate::{
    data::{
        self, Album, AlbumType, Artist, ArtistAlbums, ArtistInfo, ArtistLink, ArtistStats,
        AudioAnalysis, Cached, Episode, EpisodeId, EpisodeLink, Image, MixedView, Nav, Page,
        Playlist, PublicUser, Range, Recommendations, RecommendationsRequest, SearchResults,
        SearchTopic, Show, SpotifyUrl, Track, TrackId, TrackLines, UserProfile,
        utils::sanitize_html_string,
    },
    error::Error,
    ui::credits::TrackCredits,
};

use super::rspotify_client::RSpotifyClient;
use super::{cache::WebApiCache, local::LocalTrackManager};
use sanitize_html::{rules::predefined::DEFAULT, sanitize_str};

#[derive(Copy, Clone)]
enum CachePolicy {
    Use,
    Refresh,
}

#[derive(Debug)]
enum RequestError {
    Auth(Error),
    Transport(ureq::Error),
}

pub struct WebApi {
    session: SessionService,
    agent: Agent,
    cache: WebApiCache,
    login5: Login5,
    oauth_token: Mutex<Option<OAuthToken>>,
    client_token_provider: ClientTokenProvider,
    librespot_state: Mutex<Option<LibrespotState>>,
    rspotify: RSpotifyClient,
    rspotify_rt: tokio::runtime::Runtime,
    local_track_manager: Mutex<LocalTrackManager>,
    paginated_limit: usize,
    rate_limiter: Mutex<RateLimiter>,
    request_gate: RequestGate,
    webapi_client_id: String,
}

struct LibrespotState {
    session: LibrespotSession,
    connected: bool,
}

struct RateLimiter {
    next_allowed: Instant,
    cooldown_until: Option<Instant>,
    cooldown_until_wall: Option<SystemTime>,
    consecutive_429: u32,
}

struct RequestGate {
    state: Mutex<RequestGateState>,
    waiters: Condvar,
    max_in_flight: usize,
}

struct RequestGateState {
    in_flight: usize,
}

struct RequestPermit<'a> {
    gate: &'a RequestGate,
}

impl RateLimiter {
    fn from_cache(cache: &WebApiCache) -> Self {
        let mut limiter = Self {
            next_allowed: Instant::now(),
            cooldown_until: None,
            cooldown_until_wall: None,
            consecutive_429: 0,
        };
        if let Some(until_wall) = WebApi::load_persisted_cooldown(cache)
            && let Ok(remaining) = until_wall.duration_since(SystemTime::now())
        {
            limiter.cooldown_until = Some(Instant::now() + remaining);
            limiter.cooldown_until_wall = Some(until_wall);
        }
        limiter
    }
}

impl WebApi {
    pub fn new(
        session: SessionService,
        proxy_url: Option<&str>,
        cache_base: Option<PathBuf>,
        oauth_token: Option<OAuthToken>,
        paginated_limit: usize,
        webapi_client_id: String,
    ) -> Self {
        let mut agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(5)))
            .http_status_as_error(false);
        if let Some(proxy_url) = proxy_url {
            let proxy = ureq::Proxy::new(proxy_url).ok();
            agent = agent.proxy(proxy);
        }
        let cache = WebApiCache::new(cache_base);
        let rate_limiter = RateLimiter::from_cache(&cache);
        let rspotify_rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .expect("Failed to initialize rspotify runtime");
        let rspotify = RSpotifyClient::new(proxy_url, Some(&webapi_client_id));
        Self {
            session,
            agent: agent.build().into(),
            cache,
            login5: Login5::new(None, proxy_url),
            oauth_token: Mutex::new(oauth_token),
            client_token_provider: ClientTokenProvider::new(proxy_url),
            librespot_state: Mutex::new(None),
            rspotify,
            rspotify_rt,
            local_track_manager: Mutex::new(LocalTrackManager::new()),
            paginated_limit,
            rate_limiter: Mutex::new(rate_limiter),
            request_gate: RequestGate::new(1),
            webapi_client_id,
        }
    }

    // Similar to how librespot does this https://github.com/librespot-org/librespot/blob/dev/core/src/version.rs
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

    fn cache_key(raw: &str) -> String {
        WebApiCache::hash_key(raw)
    }

    fn access_token(&self) -> Result<String, Error> {
        // Try OAuth PKCE first -- standard Web API auth path
        match self.oauth_access_token() {
            Ok(Some(token)) => {
                log::debug!("webapi: using oauth access token");
                return Ok(token);
            }
            Ok(None) => {
                log::debug!("webapi: no oauth token available, trying login5");
            }
            Err(err) => {
                log::warn!("webapi: oauth token error: {err}, trying login5");
            }
        }

        // Fallback to login5
        match self.login5.get_access_token(&self.session) {
            Ok(token) => {
                log::debug!("webapi: using login5 access token (fallback)");
                Ok(token.access_token)
            }
            Err(err) => {
                log::warn!("webapi: login5 token also failed: {err}");
                Err(Error::WebApiError(
                    "Web API authentication failed. Re-authenticate in Preferences.".to_string(),
                ))
            }
        }
    }

    fn oauth_access_token(&self) -> Result<Option<String>, Error> {
        const EXPIRY_BUFFER: Duration = Duration::from_secs(120);
        let mut guard = self.oauth_token.lock();
        let Some(token) = guard.as_mut() else {
            return Ok(None);
        };

        if token.is_expired(EXPIRY_BUFFER) {
            let Some(refresh_token) = token.refresh_token.clone() else {
                log::warn!("webapi: oauth token expired but no refresh token available");
                return Ok(None);
            };
            match oauth::refresh_access_token(&refresh_token, &self.webapi_client_id) {
                Ok(refreshed) => {
                    log::info!("webapi: refreshed oauth access token");
                    *guard = Some(refreshed.clone());
                    return Ok(Some(refreshed.access_token));
                }
                Err(err) => {
                    let message = err.to_string();
                    log::warn!("webapi: oauth refresh failed: {message}");
                    if message.contains("invalid_grant") {
                        *guard = None;
                        log::warn!("webapi: oauth token revoked, clearing stored token");
                    }
                    return Ok(None);
                }
            }
        }

        Ok(Some(token.access_token.clone()))
    }

    fn request(&self, request: &RequestBuilder) -> Result<Response<Body>, Error> {
        let _permit = self.request_gate.acquire();
        self.with_retry(request, || self.request_raw(request))
    }

    fn request_raw(&self, request: &RequestBuilder) -> Result<Response<Body>, RequestError> {
        let token = self.access_token().map_err(RequestError::Auth)?;
        let url = request.build();

        fn configure_request<B>(
            req_builder: ureq::RequestBuilder<B>,
            token: &str,
            headers: &HashMap<String, String>,
        ) -> ureq::RequestBuilder<B> {
            let mut builder = req_builder.header("Authorization", &format!("Bearer {token}"));
            if !headers.contains_key("User-Agent") && !headers.contains_key("user-agent") {
                builder = builder.header("User-Agent", WebApi::user_agent());
            }
            headers
                .iter()
                .fold(builder, |current_req, (k, v)| current_req.header(k, v))
        }

        let mut headers = request.get_headers().clone();
        if request.base_uri == "api.spotify.com" {
            headers.insert("app-platform".to_string(), "WebPlayer".to_string());
            match self.client_token_provider.get() {
                Ok(client_token) => {
                    headers.insert("client-token".to_string(), client_token);
                    log::debug!("webapi: attached client-token header");
                }
                Err(err) => {
                    log::warn!("webapi: failed to get client token: {err}");
                }
            }
        }
        match request.get_method() {
            Method::Get => configure_request(self.agent.get(&url), &token, &headers)
                .call()
                .map_err(RequestError::Transport),
            Method::Post => configure_request(self.agent.post(&url), &token, &headers)
                .send_json(request.get_body())
                .map_err(RequestError::Transport),
            Method::Put => configure_request(self.agent.put(&url), &token, &headers)
                .send_json(request.get_body())
                .map_err(RequestError::Transport),
            Method::Delete => configure_request(self.agent.delete(&url), &token, &headers)
                .force_send_body()
                .send_json(request.get_body())
                .map_err(RequestError::Transport),
        }
    }

    fn with_retry(
        &self,
        request: &RequestBuilder,
        f: impl Fn() -> Result<Response<Body>, RequestError>,
    ) -> Result<Response<Body>, Error> {
        const MAX_ATTEMPTS: u8 = 5;
        const BASE_BACKOFF: Duration = Duration::from_millis(500);
        const MAX_BACKOFF: Duration = Duration::from_secs(10);
        const MIN_429_DELAY: Duration = Duration::from_secs(60);
        let mut attempts = 0;
        let mut backoff = BASE_BACKOFF;

        loop {
            self.wait_for_rate_limit(request.base_uri.as_str())?;
            match f() {
                Ok(response) => match response.status() {
                    StatusCode::TOO_MANY_REQUESTS => {
                        let retry_after_header = response
                            .headers()
                            .get("Retry-After")
                            .and_then(|secs| secs.to_str().ok())
                            .map(str::to_string);
                        if let Some(ref value) = retry_after_header {
                            log::warn!("webapi: 429 Retry-After header = {value}");
                        } else {
                            log::warn!("webapi: 429 without Retry-After header");
                        }
                        log::warn!(
                            "webapi: 429 on {:?} {}",
                            request.get_method(),
                            request.build()
                        );
                        let retry_after_secs = retry_after_header
                            .as_deref()
                            .and_then(|secs| secs.parse::<u64>().ok());
                        let response_delay = self
                            .register_429(retry_after_secs.map(Duration::from_secs), MIN_429_DELAY);
                        if attempts < MAX_ATTEMPTS {
                            attempts += 1;
                            continue;
                        }
                        break Err(Error::WebApiError(format!(
                            "rate limited (HTTP 429), retry in {}s",
                            response_delay.as_secs()
                        )));
                    }
                    StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => {
                        if attempts >= MAX_ATTEMPTS {
                            break Err(Error::WebApiError(
                                "request timed out (HTTP 408/504)".to_string(),
                            ));
                        }
                        thread::sleep(backoff);
                        attempts += 1;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                    status if status.is_client_error() || status.is_server_error() => {
                        break Err(Error::WebApiError(format!(
                            "https status: {}",
                            status.as_u16()
                        )));
                    }
                    _ => {
                        self.clear_rate_limit();
                        break Ok(response);
                    }
                },
                Err(RequestError::Auth(err)) => break Err(err),
                Err(RequestError::Transport(err)) => {
                    if let ureq::Error::StatusCode(code) = &err
                        && *code == 429
                    {
                        let response_delay = self.register_429(None, MIN_429_DELAY);
                        if attempts < MAX_ATTEMPTS {
                            attempts += 1;
                            continue;
                        }
                        break Err(Error::WebApiError(format!(
                            "rate limited (HTTP 429), retry in {}s",
                            response_delay.as_secs()
                        )));
                    }
                    let should_retry = Self::is_retryable_error(&err);
                    if should_retry && attempts < MAX_ATTEMPTS {
                        thread::sleep(backoff);
                        attempts += 1;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                    break Err(Error::from(err));
                }
            }
        }
    }

    fn is_retryable_error(err: &ureq::Error) -> bool {
        match err {
            ureq::Error::Timeout(_) => true,
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => true,
            ureq::Error::Io(err) => matches!(
                err.kind(),
                io::ErrorKind::TimedOut
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::Interrupted
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionRefused
            ),
            ureq::Error::StatusCode(code) => matches!(*code, 408 | 429 | 504),
            _ => false,
        }
    }

    fn wait_for_rate_limit(&self, _base_uri: &str) -> Result<(), Error> {
        const MIN_API_INTERVAL: Duration = Duration::from_millis(900);
        let mut delay = None;
        {
            let mut limiter = self.rate_limiter.lock();
            let now = Instant::now();
            if let Some(until) = limiter.cooldown_until {
                if until > now {
                    delay = Some(until - now);
                } else {
                    limiter.cooldown_until = None;
                }
            }
            if delay.is_none()
                && let Some(until_wall) = limiter.cooldown_until_wall
            {
                if let Ok(remaining) = until_wall.duration_since(SystemTime::now()) {
                    delay = Some(remaining);
                } else {
                    limiter.cooldown_until_wall = None;
                }
            }
            if delay.is_none() {
                if limiter.next_allowed <= now {
                    limiter.next_allowed = now + MIN_API_INTERVAL;
                } else {
                    delay = Some(limiter.next_allowed - now);
                    limiter.next_allowed += MIN_API_INTERVAL;
                }
            }
        }
        if let Some(delay) = delay {
            log::warn!("webapi: blocked by cooldown, retry in {}s", delay.as_secs());
            thread::sleep(delay);
            return Ok(());
        }
        Ok(())
    }

    fn register_429(&self, retry_after: Option<Duration>, min_delay: Duration) -> Duration {
        const MAX_DELAY_SECS: u64 = 60 * 60;
        let mut limiter = self.rate_limiter.lock();
        limiter.consecutive_429 = limiter.consecutive_429.saturating_add(1);
        let exp = (limiter.consecutive_429.saturating_sub(1)).min(6);
        let base_secs = min_delay.as_secs().max(60);
        let mut delay_secs = base_secs.saturating_mul(1u64 << exp);
        if let Some(retry) = retry_after {
            delay_secs = delay_secs.max(retry.as_secs());
        }
        delay_secs = delay_secs.min(MAX_DELAY_SECS);
        let delay = Duration::from_secs(delay_secs);
        let target = Instant::now() + delay;
        if limiter.next_allowed < target {
            limiter.next_allowed = target;
        }
        limiter.cooldown_until = Some(target);
        let wall_target = SystemTime::now() + delay;
        limiter.cooldown_until_wall = Some(wall_target);
        Self::persist_cooldown(&self.cache, wall_target);
        log::warn!(
            "webapi: HTTP 429 cooldown {}s (consecutive={})",
            delay_secs,
            limiter.consecutive_429
        );
        delay
    }

    fn clear_rate_limit(&self) {
        let mut limiter = self.rate_limiter.lock();
        limiter.consecutive_429 = 0;
        limiter.cooldown_until = None;
        limiter.cooldown_until_wall = None;
        Self::clear_persisted_cooldown(&self.cache);
    }

    fn persist_cooldown(cache: &WebApiCache, until: SystemTime) {
        let Ok(secs) = until.duration_since(SystemTime::UNIX_EPOCH) else {
            return;
        };
        let payload = json!({ "until_unix": secs.as_secs() });
        if let Ok(bytes) = serde_json::to_vec(&payload) {
            cache.set("rate-limit", "cooldown.json", &bytes);
        }
    }

    fn clear_persisted_cooldown(cache: &WebApiCache) {
        cache.remove("rate-limit", "cooldown.json");
    }

    fn load_persisted_cooldown(cache: &WebApiCache) -> Option<SystemTime> {
        let file = cache.get("rate-limit", "cooldown.json")?;
        let payload: serde_json::Value = serde_json::from_reader(file).ok()?;
        let secs = payload.get("until_unix")?.as_u64()?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Send a request with an empty JSON object, throw away the response body.
    /// Use for POST/PUT/DELETE requests.
    fn send_empty_json(&self, request: &RequestBuilder) -> Result<(), Error> {
        self.request(request).map(|_| ())
    }

    /// Send a request using `self.load()`, but only if it isn't already present
    /// in cache.
    fn load_cached<T: Data + DeserializeOwned>(
        &self,
        request: &RequestBuilder,
        bucket: &str,
        key: &str,
    ) -> Result<Cached<T>, Error> {
        self.load_cached_with(request, bucket, key, CachePolicy::Use)
    }

    fn load_cached_with<T: Data + DeserializeOwned>(
        &self,
        request: &RequestBuilder,
        bucket: &str,
        key: &str,
        policy: CachePolicy,
    ) -> Result<Cached<T>, Error> {
        let (value, cached_at) = self.load_cached_value(request, bucket, key, policy)?;
        Ok(match cached_at {
            Some(at) => Cached::new(value, at),
            None => Cached::fresh(value),
        })
    }

    fn load_cached_value<T: DeserializeOwned>(
        &self,
        request: &RequestBuilder,
        bucket: &str,
        key: &str,
        policy: CachePolicy,
    ) -> Result<(T, Option<SystemTime>), Error> {
        if matches!(policy, CachePolicy::Use)
            && let Some(file) = self.cache.get(bucket, key)
        {
            let cached_at = file.metadata()?.modified()?;
            let value = serde_json::from_reader(file)?;
            Ok((value, Some(cached_at)))
        } else {
            let response = self.request(request)?;
            let body = {
                let mut reader = response.into_body().into_reader();
                let mut body = Vec::new();
                reader.read_to_end(&mut body)?;
                body
            };
            let value = serde_json::from_slice(&body)?;
            self.cache.set(bucket, key, &body);
            Ok((value, None))
        }
    }

    fn load_cached_value_rspotify<T: DeserializeOwned + Serialize>(
        &self,
        bucket: &str,
        key: &str,
        policy: CachePolicy,
        fetch: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        if matches!(policy, CachePolicy::Use)
            && let Some(file) = self.cache.get(bucket, key)
        {
            match serde_json::from_reader(file) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    log::warn!("webapi: invalid cache entry for {bucket}/{key}, refetching: {err}");
                    self.cache.remove(bucket, key);
                }
            }
        }

        let value = fetch()?;
        if let Ok(bytes) = serde_json::to_vec(&value) {
            self.cache.set(bucket, key, &bytes);
        }
        Ok(value)
    }

    fn rspotify_to<T: DeserializeOwned, U: Serialize>(&self, value: &U) -> Result<T, Error> {
        let json = serde_json::to_value(value)?;
        Ok(serde_json::from_value(json)?)
    }

    fn rspotify_vec<T: DeserializeOwned + Clone, U: Serialize>(
        &self,
        items: impl IntoIterator<Item = U>,
    ) -> Vector<T> {
        items
            .into_iter()
            .filter_map(|item| self.rspotify_to(&item).ok())
            .collect()
    }

    fn oauth_token_for_rspotify(&self) -> Result<Option<RSpotifyToken>, Error> {
        const EXPIRY_BUFFER: Duration = Duration::from_secs(120);
        let mut guard = self.oauth_token.lock();
        let Some(token) = guard.as_mut() else {
            return Ok(None);
        };

        if token.is_expired(EXPIRY_BUFFER) {
            let Some(refresh_token) = token.refresh_token.clone() else {
                log::warn!("webapi: oauth token expired but no refresh token available");
                return Ok(None);
            };
            match oauth::refresh_access_token(&refresh_token, &self.webapi_client_id) {
                Ok(refreshed) => {
                    log::info!("webapi: refreshed oauth access token");
                    *guard = Some(refreshed.clone());
                }
                Err(err) => {
                    let message = err.to_string();
                    log::warn!("webapi: oauth refresh failed: {message}");
                    if message.contains("invalid_grant") {
                        *guard = None;
                        log::warn!("webapi: oauth token revoked, clearing stored token");
                    }
                    return Ok(None);
                }
            }
        }

        let token = guard.as_ref().expect("oauth token must exist");
        let now_unix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let expires_in_secs = token
            .expires_at_unix
            .and_then(|expires_at| expires_at.checked_sub(now_unix))
            .unwrap_or(3600);
        let expires_in = ChronoDuration::from_std(Duration::from_secs(expires_in_secs))
            .unwrap_or_else(|_| ChronoDuration::seconds(3600));
        let expires_at = Some(Utc::now() + expires_in);

        Ok(Some(RSpotifyToken {
            access_token: token.access_token.clone(),
            expires_in,
            expires_at,
            refresh_token: token.refresh_token.clone(),
            scopes: HashSet::new(),
        }))
    }

    fn ensure_librespot_session(&self) -> Result<Option<LibrespotSession>, Error> {
        let creds = match self.session.credentials() {
            Some(creds) => creds,
            None => return Ok(None),
        };

        let libre_creds = LibrespotCredentials {
            username: creds.username.clone(),
            auth_type: creds.auth_type,
            auth_data: creds.auth_data.clone(),
        };

        let session = {
            let mut guard = self.librespot_state.lock();
            let needs_new = match guard.as_ref() {
                Some(state) => state.session.is_invalid(),
                None => true,
            };

            if needs_new {
                let session = {
                    let _guard = self.rspotify_rt.enter();
                    LibrespotSession::new(LibrespotSessionConfig::default(), None)
                };
                *guard = Some(LibrespotState {
                    session: session.clone(),
                    connected: false,
                });
            }

            guard
                .as_ref()
                .expect("librespot session must be present")
                .session
                .clone()
        };

        let connected = self
            .librespot_state
            .lock()
            .as_ref()
            .map(|state| state.connected)
            .unwrap_or(false);

        if !connected {
            let connect_result = self
                .rspotify_rt
                .block_on(session.connect(libre_creds, true));
            if let Err(err) = connect_result {
                log::warn!("webapi: librespot session connect failed: {err}");
                return Ok(None);
            }
            if let Some(state) = self.librespot_state.lock().as_mut() {
                state.connected = true;
            }
        }

        Ok(Some(session))
    }

    fn rspotify_call<T, F>(&self, f: impl FnOnce() -> F) -> Result<T, Error>
    where
        F: Future<Output = rspotify::ClientResult<T>>,
    {
        const MIN_429_DELAY: Duration = Duration::from_secs(60);
        let _permit = self.request_gate.acquire();
        self.wait_for_rate_limit("api.spotify.com")?;

        // Try OAuth token first (preferred for Web API)
        let mut has_token = false;
        if let Ok(Some(token)) = self.oauth_token_for_rspotify() {
            self.rspotify_rt
                .block_on(async { self.rspotify.set_token(token).await });
            has_token = true;
        }

        // Fallback to librespot session login5 token
        if !has_token {
            let libre_session = self.ensure_librespot_session()?;
            has_token = self.rspotify_rt.block_on(async {
                if let Some(session) = libre_session {
                    self.rspotify.set_session(session).await;
                }
                self.rspotify.ensure_token().await
            });
        }

        if !has_token {
            return Err(Error::WebApiError(
                "Web API authentication failed. Re-authenticate in Preferences.".to_string(),
            ));
        }

        let result = self.rspotify_rt.block_on(async { f().await });

        match result {
            Ok(value) => {
                self.clear_rate_limit();
                Ok(value)
            }
            Err(err) => {
                if let ClientError::Http(http_err) = &err
                    && let RSpotifyHttpError::StatusCode(resp) = http_err.as_ref()
                    && resp.status().as_u16() == 429
                {
                    let retry_after = resp
                        .headers()
                        .get("Retry-After")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let delay = self.register_429(retry_after, MIN_429_DELAY);
                    log::warn!("webapi: HTTP 429 cooldown {}s (rspotify)", delay.as_secs());
                    }
                Err(Error::WebApiError(err.to_string()))
            }
        }
    }

    fn artist_from_full(&self, artist: rspotify::model::FullArtist) -> Artist {
        Artist {
            id: Arc::from(artist.id.id()),
            name: Arc::from(artist.name),
            images: artist
                .images
                .into_iter()
                .map(|image| Image {
                    url: Arc::from(image.url),
                    width: image.width.map(|width| width as usize),
                    height: image.height.map(|height| height as usize),
                })
                .collect(),
        }
    }

    fn artist_links_from_simplified(
        &self,
        artists: Vec<rspotify::model::SimplifiedArtist>,
    ) -> Vector<ArtistLink> {
        artists
            .into_iter()
            .filter_map(|artist| {
                let id = artist.id?;
                Some(ArtistLink {
                    id: Arc::from(id.id()),
                    name: Arc::from(artist.name),
                })
            })
            .collect()
    }

    fn images_from_rspotify(&self, images: Vec<rspotify::model::Image>) -> Vector<Image> {
        images
            .into_iter()
            .map(|image| Image {
                url: Arc::from(image.url),
                width: image.width.map(|width| width as usize),
                height: image.height.map(|height| height as usize),
            })
            .collect()
    }

    fn public_user_from_rspotify(&self, user: rspotify::model::PublicUser) -> PublicUser {
        PublicUser {
            id: Arc::from(user.id.id()),
            display_name: Arc::from(user.display_name.unwrap_or_default()),
        }
    }

    fn user_profile_from_rspotify(&self, user: rspotify::model::PrivateUser) -> UserProfile {
        UserProfile {
            display_name: Arc::from(user.display_name.unwrap_or_default()),
            email: Arc::from(user.email.unwrap_or_default()),
            id: Arc::from(user.id.id()),
        }
    }

    fn playlist_from_simplified(&self, playlist: rspotify::model::SimplifiedPlaylist) -> Playlist {
        Playlist {
            id: Arc::from(playlist.id.id()),
            name: Arc::from(playlist.name),
            images: Some(self.images_from_rspotify(playlist.images)),
            description: Arc::from(""),
            track_count: Some(playlist.tracks.total as usize),
            owner: self.public_user_from_rspotify(playlist.owner),
            collaborative: playlist.collaborative,
            public: playlist.public,
        }
    }

    fn playlist_from_full(&self, playlist: rspotify::model::FullPlaylist) -> Playlist {
        Playlist {
            id: Arc::from(playlist.id.id()),
            name: Arc::from(playlist.name),
            images: Some(self.images_from_rspotify(playlist.images)),
            description: sanitize_html_string(playlist.description.as_deref().unwrap_or_default()),
            track_count: Some(playlist.tracks.total as usize),
            owner: self.public_user_from_rspotify(playlist.owner),
            collaborative: playlist.collaborative,
            public: playlist.public,
        }
    }

    fn album_type_from_meta(&self, album: &rspotify::model::SimplifiedAlbum) -> AlbumType {
        let group = album
            .album_group
            .as_deref()
            .or(album.album_type.as_deref());
        match group {
            Some("single") => AlbumType::Single,
            Some("compilation") => AlbumType::Compilation,
            Some("appears_on") => AlbumType::AppearsOn,
            _ => AlbumType::Album,
        }
    }

    fn parse_release_date(&self, raw: Option<&str>) -> Option<Date> {
        let raw = raw?;
        let mut parts = raw.splitn(3, '-');
        let year = parts.next()?.parse::<i32>().ok()?;
        let month = parts
            .next()
            .and_then(|part| part.parse::<u8>().ok())
            .unwrap_or(1);
        let day = parts
            .next()
            .and_then(|part| part.parse::<u8>().ok())
            .unwrap_or(1);
        let month = Month::try_from(month).ok()?;
        Date::from_calendar_date(year, month, day).ok()
    }

    fn album_from_simplified(&self, album: rspotify::model::SimplifiedAlbum) -> Option<Album> {
        let id = album.id.as_ref()?.id().to_string();
        let album_type = self.album_type_from_meta(&album);
        let release_date = self.parse_release_date(album.release_date.as_deref());
        let name = album.name;
        Some(Album {
            id: Arc::from(id),
            name: Arc::from(name),
            album_type,
            images: album
                .images
                .into_iter()
                .map(|image| Image {
                    url: Arc::from(image.url),
                    width: image.width.map(|width| width as usize),
                    height: image.height.map(|height| height as usize),
                })
                .collect(),
            artists: self.artist_links_from_simplified(album.artists),
            copyrights: Vector::new(),
            label: "".into(),
            tracks: Vector::new(),
            release_date,
            release_date_precision: None,
        })
    }

    fn for_all_pages_cached<T: DeserializeOwned + Clone>(
        &self,
        request: &RequestBuilder,
        bucket: &str,
        key: &str,
        policy: CachePolicy,
        mut func: impl FnMut(Page<T>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut limit = 50;
        let mut offset = 0;
        loop {
            let req = request
                .clone()
                .query("limit".to_string(), limit.to_string())
                .query("offset".to_string(), offset.to_string());
            let page_key = format!("{key}-o{offset}-l{limit}");
            let (page, _) = self.load_cached_value::<Page<T>>(&req, bucket, &page_key, policy)?;

            let page_total = page.total;
            let page_offset = page.offset;
            let page_limit = page.limit;
            func(page)?;

            if page_total > offset && offset < self.paginated_limit {
                limit = page_limit;
                offset = page_offset + page_limit;
            } else {
                break Ok(());
            }
        }
    }

    fn load_all_pages_cached<T: DeserializeOwned + Clone>(
        &self,
        request: &RequestBuilder,
        bucket: &str,
        key: &str,
        policy: CachePolicy,
    ) -> Result<Vector<T>, Error> {
        let mut results = Vector::new();

        self.for_all_pages_cached(request, bucket, key, policy, |page| {
            results.append(page.items);
            Ok(())
        })?;

        Ok(results)
    }

    /// Load local track files from the official client's database.
    pub fn load_local_tracks(&self, username: &str) {
        if let Err(err) = self
            .local_track_manager
            .lock()
            .load_tracks_for_user(username)
        {
            log::error!("failed to read local tracks: {err}");
        }
    }

    fn load_and_return_home_section(
        &self,
        request: &RequestBuilder,
        cache_key: &str,
        policy: CachePolicy,
    ) -> Result<MixedView, Error> {
        #[derive(Deserialize)]
        pub struct Welcome {
            data: WelcomeData,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct WelcomeData {
            home_sections: HomeSections,
        }

        #[derive(Deserialize)]
        pub struct HomeSections {
            sections: Vec<Section>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Section {
            data: SectionData,
            section_items: SectionItems,
        }

        #[derive(Deserialize)]
        pub struct SectionData {
            title: Title,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Title {
            text: String,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct SectionItems {
            items: Vec<Item>,
        }

        #[derive(Deserialize)]
        pub struct Item {
            content: Content,
        }

        #[derive(Deserialize)]
        pub struct Content {
            data: ContentData,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct ContentData {
            #[serde(rename = "__typename")]
            typename: DataTypename,
            name: Option<String>,
            uri: Option<String>,

            // Playlist-specific fields
            attributes: Option<Vec<Attribute>>,
            description: Option<String>,
            images: Option<Images>,
            owner_v2: Option<OwnerV2>,

            // Artist-specific fields
            artists: Option<Artists>,
            profile: Option<Profile>,
            visuals: Option<Visuals>,

            // Show-specific fields
            cover_art: Option<CoverArt>,
            publisher: Option<Publisher>,
            total_episodes: Option<usize>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Visuals {
            avatar_image: CoverArt,
        }

        #[derive(Deserialize)]
        pub struct Artists {
            items: Vec<ArtistsItem>,
        }

        #[derive(Deserialize)]
        pub struct ArtistsItem {
            profile: Profile,
            uri: String,
        }

        #[derive(Deserialize)]
        pub struct Profile {
            name: String,
        }

        #[derive(Deserialize)]
        pub struct Attribute {
            key: String,
            value: String,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct CoverArt {
            sources: Vec<Source>,
        }

        #[derive(Deserialize)]
        pub struct Source {
            url: String,
        }

        #[derive(Deserialize)]
        #[allow(dead_code)]
        pub enum MediaType {
            #[serde(rename = "AUDIO")]
            Audio,
            #[serde(rename = "MIXED")]
            Mixed,
        }

        #[derive(Deserialize)]
        pub struct Publisher {
            name: String,
        }

        #[derive(Deserialize)]
        pub enum DataTypename {
            Podcast,
            Playlist,
            Artist,
            Album,
            NotFound,
        }

        #[derive(Deserialize)]
        pub struct Images {
            items: Vec<ImagesItem>,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct ImagesItem {
            sources: Vec<Source>,
        }

        #[derive(Deserialize)]
        pub struct OwnerV2 {
            data: OwnerV2Data,
        }

        #[derive(Deserialize)]
        pub struct OwnerV2Data {
            #[serde(rename = "__typename")]
            name: String,
        }

        // Extract the playlists
        let result: Welcome = match self
            .load_cached_value(request, "home-section", cache_key, policy)
            .map(|(value, _)| value)
        {
            Ok(res) => res,
            Err(e) => {
                info!("Error loading home section: {e}");
                return Err(e);
            }
        };

        let mut title: Arc<str> = Arc::from("");
        let mut playlist: Vector<Playlist> = Vector::new();
        let mut album: Vector<Arc<Album>> = Vector::new();
        let mut artist: Vector<Artist> = Vector::new();
        let mut show: Vector<Arc<Show>> = Vector::new();

        result
            .data
            .home_sections
            .sections
            .iter()
            .for_each(|section| {
                title = section.data.title.text.clone().into();

                section.section_items.items.iter().for_each(|item| {
                    let Some(uri) = &item.content.data.uri else {
                        return;
                    };
                    let id = uri.split(':').next_back().unwrap_or("").to_string();

                    match item.content.data.typename {
                        DataTypename::Playlist => {
                            playlist.push_back(Playlist {
                                id: id.into(),
                                name: Arc::from(item.content.data.name.clone().unwrap()),
                                images: Some(item.content.data.images.as_ref().map_or_else(
                                    Vector::new,
                                    |images| {
                                        images
                                            .items
                                            .iter()
                                            .map(|img| data::utils::Image {
                                                url: Arc::from(
                                                    img.sources
                                                        .first()
                                                        .map(|s| s.url.as_str())
                                                        .unwrap_or_default(),
                                                ),
                                                width: None,
                                                height: None,
                                            })
                                            .collect()
                                    },
                                )),
                                description: {
                                    let desc = sanitize_html_string(
                                        item.content
                                            .data
                                            .description
                                            .as_deref()
                                            .unwrap_or_default(),
                                    );

                                    // This is roughly 3 lines of description, truncated if too long
                                    if desc.chars().count() > 55 {
                                        Arc::from(desc.chars().take(52).collect::<String>() + "...")
                                    } else {
                                        desc
                                    }
                                },
                                track_count: item.content.data.attributes.as_ref().and_then(
                                    |attrs| {
                                        attrs
                                            .iter()
                                            .find(|attr| attr.key == "track_count")
                                            .and_then(|attr| attr.value.parse().ok())
                                    },
                                ),
                                owner: PublicUser {
                                    id: Arc::from(""),
                                    display_name: Arc::from(
                                        item.content
                                            .data
                                            .owner_v2
                                            .as_ref()
                                            .map(|owner| owner.data.name.as_str())
                                            .unwrap_or_default(),
                                    ),
                                },
                                collaborative: false,
                                public: None,
                            });
                        }
                        DataTypename::Artist => artist.push_back(Artist {
                            id: id.into(),
                            name: Arc::from(
                                item.content.data.profile.as_ref().unwrap().name.clone(),
                            ),
                            images: item.content.data.visuals.as_ref().map_or_else(
                                Vector::new,
                                |images| {
                                    images
                                        .avatar_image
                                        .sources
                                        .iter()
                                        .map(|img| data::utils::Image {
                                            url: Arc::from(img.url.as_str()),
                                            width: None,
                                            height: None,
                                        })
                                        .collect()
                                },
                            ),
                        }),
                        DataTypename::Album => album.push_back(Arc::new(Album {
                            id: id.into(),
                            name: Arc::from(item.content.data.name.clone().unwrap()),
                            album_type: AlbumType::Album,
                            images: item.content.data.cover_art.as_ref().map_or_else(
                                Vector::new,
                                |images| {
                                    images
                                        .sources
                                        .iter()
                                        .map(|src| data::utils::Image {
                                            url: Arc::from(src.url.clone()),
                                            width: None,
                                            height: None,
                                        })
                                        .collect()
                                },
                            ),
                            artists: item.content.data.artists.as_ref().map_or_else(
                                Vector::new,
                                |artists| {
                                    artists
                                        .items
                                        .iter()
                                        .map(|artist| ArtistLink {
                                            id: Arc::from(
                                                artist
                                                    .uri
                                                    .split(':')
                                                    .next_back()
                                                    .unwrap_or("")
                                                    .to_string(),
                                            ),
                                            name: Arc::from(artist.profile.name.clone()),
                                        })
                                        .collect()
                                },
                            ),
                            copyrights: Vector::new(),
                            label: "".into(),
                            tracks: Vector::new(),
                            release_date: None,
                            release_date_precision: None,
                        })),
                        DataTypename::Podcast => show.push_back(Arc::new(Show {
                            id: id.into(),
                            name: Arc::from(item.content.data.name.clone().unwrap()),
                            images: item.content.data.cover_art.as_ref().map_or_else(
                                Vector::new,
                                |images| {
                                    images
                                        .sources
                                        .iter()
                                        .map(|src| data::utils::Image {
                                            url: Arc::from(src.url.clone()),
                                            width: None,
                                            height: None,
                                        })
                                        .collect()
                                },
                            ),
                            publisher: Arc::from(
                                item.content
                                    .data
                                    .publisher
                                    .as_ref()
                                    .map(|p| p.name.as_str())
                                    .unwrap_or(""),
                            ),
                            description: Arc::from(
                                item.content.data.description.as_deref().unwrap_or(""),
                            ),
                            total_episodes: item.content.data.total_episodes,
                        })),
                        // For section items we don't cover yet
                        DataTypename::NotFound => {}
                    }
                });
            });

        Ok(MixedView {
            title,
            playlists: playlist,
            artists: artist,
            albums: album,
            shows: show,
        })
    }
}

static GLOBAL_WEBAPI: OnceLock<Arc<WebApi>> = OnceLock::new();

/// Global instance.
impl WebApi {
    pub fn install_as_global(self) {
        GLOBAL_WEBAPI
            .set(Arc::new(self))
            .map_err(|_| "Cannot install more than once")
            .unwrap()
    }

    pub fn global() -> Arc<Self> {
        GLOBAL_WEBAPI.get().unwrap().clone()
    }

    pub fn rate_limit_delay(&self) -> Option<Duration> {
        let limiter = self.rate_limiter.lock();
        let now = Instant::now();
        if let Some(until) = limiter.cooldown_until
            && until > now
        {
            return Some(until - now);
        }
        if let Some(until_wall) = limiter.cooldown_until_wall
            && let Ok(remaining) = until_wall.duration_since(SystemTime::now())
            && !remaining.is_zero()
        {
            return Some(remaining);
        }
        None
    }

    /// Clears the persisted rate-limit state unconditionally.
    /// Use after re-authentication when fresh credentials make the old
    /// cooldown irrelevant.
    pub fn clear_rate_limit_state(&self) {
        self.clear_rate_limit();
    }

    /// Clears the persisted rate-limit state only if the cooldown has expired.
    /// Useful for garbage-collecting stale cooldown files without violating an
    /// active server-side rate limit.
    #[allow(dead_code)]
    pub fn clear_expired_rate_limit_state(&self) {
        let still_active = {
            let limiter = self.rate_limiter.lock();
            limiter
                .cooldown_until
                .is_some_and(|until| until > Instant::now())
        };
        if !still_active {
            self.clear_rate_limit();
        }
    }

    pub fn set_oauth_token(&self, token: OAuthToken) {
        *self.oauth_token.lock() = Some(token);
    }

    pub fn webapi_client_id(&self) -> &str {
        &self.webapi_client_id
    }

    pub fn is_rate_limited(&self) -> bool {
        self.rate_limit_delay().is_some()
    }
}

/// User endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-users-profile
    pub fn get_user_profile(&self) -> Result<UserProfile, Error> {
        let result: rspotify::model::PrivateUser =
            self.load_cached_value_rspotify("user-profile", "me", CachePolicy::Use, || {
                self.rspotify_call(|| self.rspotify.current_user())
            })?;
        Ok(self.user_profile_from_rspotify(result))
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-users-top-artists-and-tracks
    pub fn get_user_top_tracks(&self) -> Result<Vector<Arc<Track>>, Error> {
        let cache_key = "all";
        let result: rspotify::model::Page<rspotify::model::FullTrack> = self
            .load_cached_value_rspotify("user-top-tracks", cache_key, CachePolicy::Use, || {
                self.rspotify_call(|| {
                    self.rspotify.current_user_top_tracks_manual(
                        Some(TimeRange::MediumTerm),
                        Some(30),
                        None,
                    )
                })
            })?;
        Ok(self
            .rspotify_vec::<Track, _>(result.items)
            .into_iter()
            .map(Arc::new)
            .collect())
    }

    pub fn get_user_top_artist(&self) -> Result<Vector<Artist>, Error> {
        let cache_key = "all";
        let result: rspotify::model::Page<rspotify::model::FullArtist> = self
            .load_cached_value_rspotify("user-top-artists", cache_key, CachePolicy::Use, || {
                self.rspotify_call(|| {
                    self.rspotify.current_user_top_artists_manual(
                        Some(TimeRange::MediumTerm),
                        Some(10),
                        None,
                    )
                })
            })?;
        Ok(result
            .items
            .into_iter()
            .map(|artist| self.artist_from_full(artist))
            .collect())
    }
}

/// Artist endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-artist/
    pub fn get_artist(&self, id: &str) -> Result<Artist, Error> {
        let artist_id = ArtistId::from_id_or_uri(id)
            .map_err(|_| Error::WebApiError("Invalid artist id".to_string()))?;
        let result: rspotify::model::FullArtist =
            self.load_cached_value_rspotify("artist", id, CachePolicy::Use, || {
                self.rspotify_call(|| self.rspotify.artist(artist_id.as_ref()))
            })?;
        Ok(self.artist_from_full(result))
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-an-artists-albums/
    pub fn get_artist_albums(&self, id: &str) -> Result<ArtistAlbums, Error> {
        self.get_artist_albums_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_artist_albums(&self, id: &str) -> Result<ArtistAlbums, Error> {
        self.get_artist_albums_with_policy(id, CachePolicy::Refresh)
    }

    fn get_artist_albums_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<ArtistAlbums, Error> {
        let artist_id = ArtistId::from_id_or_uri(id)
            .map_err(|_| Error::WebApiError("Invalid artist id".to_string()))?
            .into_static();
        let result: Vec<rspotify::model::SimplifiedAlbum> =
            self.load_cached_value_rspotify("artist-albums", id, policy, || {
                let mut all = Vec::new();
                let mut offset = 0u32;
                let limit = 50u32;
                loop {
                    let page = self.rspotify_call(|| {
                        self.rspotify.artist_albums_manual(
                            artist_id.as_ref(),
                            [
                                RSpotifyAlbumType::Album,
                                RSpotifyAlbumType::Single,
                                RSpotifyAlbumType::Compilation,
                                RSpotifyAlbumType::AppearsOn,
                            ],
                            Some(Market::FromToken),
                            Some(limit),
                            Some(offset),
                        )
                    })?;
                    if page.items.is_empty() {
                        break;
                    }
                    offset = page.offset + page.limit;
                    all.extend(page.items);
                    if offset >= page.total || offset as usize >= self.paginated_limit {
                        break;
                    }
                }
                Ok(all)
            })?;

        let mut artist_albums = ArtistAlbums {
            albums: Vector::new(),
            singles: Vector::new(),
            compilations: Vector::new(),
            appears_on: Vector::new(),
        };

        for album in result {
            let Some(album) = self.album_from_simplified(album) else {
                continue;
            };
            match album.album_type {
                AlbumType::Album => artist_albums.albums.push_back(Arc::new(album)),
                AlbumType::Single => artist_albums.singles.push_back(Arc::new(album)),
                AlbumType::Compilation => artist_albums.compilations.push_back(Arc::new(album)),
                AlbumType::AppearsOn => artist_albums.appears_on.push_back(Arc::new(album)),
            }
        }

        Ok(artist_albums)
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-an-artists-top-tracks
    pub fn get_artist_top_tracks(&self, id: &str) -> Result<Vector<Arc<Track>>, Error> {
        self.get_artist_top_tracks_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_artist_top_tracks(&self, id: &str) -> Result<Vector<Arc<Track>>, Error> {
        self.get_artist_top_tracks_with_policy(id, CachePolicy::Refresh)
    }

    fn get_artist_top_tracks_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<Vector<Arc<Track>>, Error> {
        let artist_id = ArtistId::from_id_or_uri(id)
            .map_err(|_| Error::WebApiError("Invalid artist id".to_string()))?;
        let result: Vec<rspotify::model::FullTrack> =
            self.load_cached_value_rspotify("artist-top-tracks", id, policy, || {
                self.rspotify_call(|| {
                    self.rspotify
                        .artist_top_tracks(artist_id.as_ref(), Some(Market::FromToken))
                })
            })?;
        Ok(self
            .rspotify_vec::<Track, _>(result)
            .into_iter()
            .map(Arc::new)
            .collect())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-an-artists-related-artists
    pub fn get_artist_info(&self, id: &str) -> Result<Cached<ArtistInfo>, Error> {
        self.get_artist_info_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_artist_info(&self, id: &str) -> Result<Cached<ArtistInfo>, Error> {
        self.get_artist_info_with_policy(id, CachePolicy::Refresh)
    }

    fn get_artist_info_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<Cached<ArtistInfo>, Error> {
        #[derive(Clone, Data, Deserialize)]
        pub struct Welcome {
            data: Data1,
        }

        #[derive(Clone, Data, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Data1 {
            artist_union: ArtistUnion,
        }

        #[derive(Clone, Data, Deserialize)]
        pub struct ArtistUnion {
            profile: Profile,
            stats: Stats,
            visuals: Visuals,
        }

        #[derive(Clone, Data, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Profile {
            biography: Biography,
            external_links: ExternalLinks,
        }

        #[derive(Clone, Data, Deserialize)]
        pub struct Biography {
            text: String,
        }

        #[derive(Clone, Data, Deserialize)]
        pub struct ExternalLinks {
            items: Vector<ExternalLinksItem>,
        }

        #[derive(Clone, Data, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Visuals {
            avatar_image: AvatarImage,
        }
        #[derive(Clone, Data, Deserialize)]
        pub struct AvatarImage {
            sources: Vector<Image>,
        }
        #[derive(Clone, Data, Deserialize)]
        pub struct ExternalLinksItem {
            url: String,
        }

        #[derive(Clone, Data, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct Stats {
            followers: i64,
            monthly_listeners: i64,
            world_rank: i64,
        }

        let variables = json!( {
            "locale": "",
            "uri": format!("spotify:artist:{}", id),
        });
        let json = json!({
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "1ac33ddab5d39a3a9c27802774e6d78b9405cc188c6f75aed007df2a32737c72"
                }
            },
            "operationName": "queryArtistOverview",
            "variables": variables,
        });

        let request =
            &RequestBuilder::new("pathfinder/v2/query".to_string(), Method::Post, Some(json))
                .set_base_uri("api-partner.spotify.com")
                .header("User-Agent", Self::user_agent());
        let result: Cached<Welcome> = self.load_cached_with(request, "artist-info", id, policy)?;

        Ok(result.map(|result| {
            let hrefs: Vector<String> = result
                .data
                .artist_union
                .profile
                .external_links
                .items
                .iter()
                .map(|link| link.url.clone())
                .collect();

            ArtistInfo {
                artist_id: id.into(),
                main_image: Arc::from(
                    result.data.artist_union.visuals.avatar_image.sources[0]
                        .url
                        .to_string(),
                ),
                stats: ArtistStats {
                    followers: result.data.artist_union.stats.followers,
                    monthly_listeners: result.data.artist_union.stats.monthly_listeners,
                    world_rank: result.data.artist_union.stats.world_rank,
                },
                bio: {
                    let sanitized_bio =
                        sanitize_str(&DEFAULT, &result.data.artist_union.profile.biography.text)
                            .unwrap_or_default();
                    sanitized_bio.replace("&amp;", "&")
                },
                artist_links: hrefs,
            }
        }))
    }
}

/// Album endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-an-album/
    pub fn get_album(&self, id: &str) -> Result<Cached<Arc<Album>>, Error> {
        self.get_album_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_album(&self, id: &str) -> Result<Cached<Arc<Album>>, Error> {
        self.get_album_with_policy(id, CachePolicy::Refresh)
    }

    fn get_album_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<Cached<Arc<Album>>, Error> {
        let request = &RequestBuilder::new(format!("v1/albums/{id}"), Method::Get, None)
            .query("market", "from_token");
        let result = self.load_cached_with(request, "album", id, policy)?;
        Ok(result)
    }
}

/// Show endpoints. (Podcasts)
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-a-show/Add commentMore actions
    pub fn get_show(&self, id: &str) -> Result<Cached<Arc<Show>>, Error> {
        self.get_show_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_show(&self, id: &str) -> Result<Cached<Arc<Show>>, Error> {
        self.get_show_with_policy(id, CachePolicy::Refresh)
    }

    fn get_show_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<Cached<Arc<Show>>, Error> {
        let request = &RequestBuilder::new(format!("v1/shows/{id}"), Method::Get, None)
            .query("market", "from_token");

        let result = self.load_cached_with(request, "show", id, policy)?;

        Ok(result)
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-multiple-episodes
    fn get_episodes_with_policy(
        &self,
        ids: impl IntoIterator<Item = EpisodeId>,
        policy: CachePolicy,
    ) -> Result<Vector<Arc<Episode>>, Error> {
        #[derive(Deserialize)]
        struct Episodes {
            episodes: Vector<Arc<Episode>>,
        }

        let ids: Vec<EpisodeId> = ids.into_iter().collect();
        let id_list = ids.iter().map(|id| id.0.to_base62()).join(",");
        let cache_key = Self::cache_key(&id_list);
        let request = &RequestBuilder::new("v1/episodes", Method::Get, None)
            .query("ids", &id_list)
            .query("market", "from_token");
        let (result, _) =
            self.load_cached_value::<Episodes>(request, "episodes", &cache_key, policy)?;
        Ok(result.episodes)
    }

    pub fn get_episode(&self, id: &str) -> Result<Arc<Episode>, Error> {
        let request = &RequestBuilder::new(format!("v1/episodes/{id}"), Method::Get, None)
            .query("market", "from_token");
        let result: Cached<Arc<Episode>> = self.load_cached(request, "episode", id)?;
        Ok(result.data)
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-information-about-the-users-current-playback

    // https://developer.spotify.com/documentation/web-api/reference/get-a-shows-episodes
    pub fn get_show_episodes(&self, id: &str) -> Result<Vector<Arc<Episode>>, Error> {
        self.get_show_episodes_with_policy(id, CachePolicy::Use)
    }

    pub fn refresh_show_episodes(&self, id: &str) -> Result<Vector<Arc<Episode>>, Error> {
        self.get_show_episodes_with_policy(id, CachePolicy::Refresh)
    }

    fn get_show_episodes_with_policy(
        &self,
        id: &str,
        policy: CachePolicy,
    ) -> Result<Vector<Arc<Episode>>, Error> {
        let request = &RequestBuilder::new(format!("v1/shows/{id}/episodes"), Method::Get, None)
            .query("market", "from_token");

        let mut results = Vector::new();
        self.for_all_pages_cached(
            request,
            "show-episodes",
            id,
            policy,
            |page: Page<Option<EpisodeLink>>| {
                if !page.items.is_empty() {
                    let ids = page
                        .items
                        .into_iter()
                        .filter_map(|link| link.map(|link| link.id));
                    let episodes = self.get_episodes_with_policy(ids, policy)?;
                    results.append(episodes);
                }
                Ok(())
            },
        )?;

        Ok(results)
    }
}

/// Track endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-track
    pub fn get_track(&self, id: &str) -> Result<Arc<Track>, Error> {
        let request = &RequestBuilder::new(format!("v1/tracks/{id}"), Method::Get, None)
            .query("market", "from_token");
        let result = self.load_cached(request, "track", id)?;
        Ok(result.data)
    }

    pub fn get_track_credits(&self, track_id: &str) -> Result<TrackCredits, Error> {
        let request = &RequestBuilder::new(
            format!("track-credits-view/v0/experimental/{track_id}/credits"),
            Method::Get,
            None,
        )
        .set_base_uri("spclient.wg.spotify.com");
        let result = self.load_cached(request, "track-credits", track_id)?;
        Ok(result.data)
    }

    pub fn get_lyrics(&self, track_id: String) -> Result<Vector<TrackLines>, Error> {
        #[derive(Default, Debug, Clone, PartialEq, Deserialize, Data)]
        #[serde(rename_all = "camelCase")]
        pub struct Root {
            pub lyrics: Lyrics,
        }

        #[derive(Default, Debug, Clone, PartialEq, Deserialize, Data)]
        #[serde(rename_all = "camelCase")]
        pub struct Lyrics {
            pub lines: Vector<TrackLines>,
            pub provider: String,
            pub provider_lyrics_id: String,
        }

        let request = &RequestBuilder::new(
            format!("color-lyrics/v2/track/{track_id}"),
            Method::Get,
            None,
        )
        .set_base_uri("spclient.wg.spotify.com")
        .query("format", "json")
        .query("vocalRemoval", "false")
        .query("market", "from_token")
        .header("app-platform", "WebPlayer");

        let lyrics: Cached<Root> = self.load_cached(request, "lyrics", &track_id)?;
        Ok(lyrics.data.lyrics.lines)
    }
}

/// Library endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-users-saved-albums/
    pub fn get_saved_albums(&self) -> Result<Vector<Arc<Album>>, Error> {
        #[derive(Clone, Deserialize)]
        struct SavedAlbum {
            album: Arc<Album>,
        }

        let request =
            &RequestBuilder::new("v1/me/albums", Method::Get, None).query("market", "from_token");

        Ok(self
            .load_all_pages_cached(request, "saved-albums", "all", CachePolicy::Use)?
            .into_iter()
            .map(|item: SavedAlbum| item.album)
            .collect())
    }

    // https://developer.spotify.com/documentation/web-api/reference/save-albums-user/
    pub fn save_album(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/albums", Method::Put, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-albums");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/remove-albums-user/
    pub fn unsave_album(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/albums", Method::Delete, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-albums");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-users-saved-tracks/
    pub fn get_saved_tracks(&self) -> Result<Vector<Arc<Track>>, Error> {
        #[derive(Clone, Deserialize)]
        struct SavedTrack {
            track: Arc<Track>,
        }
        let request =
            &RequestBuilder::new("v1/me/tracks", Method::Get, None).query("market", "from_token");
        Ok(self
            .load_all_pages_cached(request, "saved-tracks", "all", CachePolicy::Use)?
            .into_iter()
            .map(|item: SavedTrack| item.track)
            .collect())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-users-saved-shows
    pub fn get_saved_shows(&self) -> Result<Vector<Arc<Show>>, Error> {
        #[derive(Clone, Deserialize)]
        struct SavedShow {
            show: Arc<Show>,
        }

        let request =
            &RequestBuilder::new("v1/me/shows", Method::Get, None).query("market", "from_token");

        Ok(self
            .load_all_pages_cached(request, "saved-shows", "all", CachePolicy::Use)?
            .into_iter()
            .map(|item: SavedShow| item.show)
            .collect())
    }

    // https://developer.spotify.com/documentation/web-api/reference/save-tracks-user/
    pub fn save_track(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/tracks", Method::Put, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-tracks");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/remove-tracks-user/
    pub fn unsave_track(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/tracks", Method::Delete, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-tracks");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/save-shows-user
    pub fn save_show(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/shows", Method::Put, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-shows");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/remove-shows-user
    pub fn unsave_show(&self, id: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new("v1/me/shows", Method::Delete, None).query("ids", id);
        self.send_empty_json(request)?;
        self.cache.clear_bucket("saved-shows");
        Ok(())
    }
}

/// View endpoints.
impl WebApi {
    pub fn get_user_info(&self) -> Result<(String, String), Error> {
        #[derive(Deserialize, Clone, Data)]
        struct User {
            region: String,
            timezone: String,
        }
        let token = self.access_token()?;

        let request = &RequestBuilder::new("json".to_string(), Method::Get, None)
            .set_protocol("http")
            .set_base_uri("ip-api.com")
            .query("fields", "260")
            .header("Authorization", format!("Bearer {token}"));

        let result: Cached<User> = self.load_cached(request, "user-info", "usrinfo")?;

        Ok((result.data.region, result.data.timezone))
    }

    pub fn get_section(&self, section_uri: &str) -> Result<MixedView, Error> {
        let (country, time_zone) = self.get_user_info()?;
        let access_token = self.access_token()?;

        let json = json!({
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "eb3fba2d388cf4fc4d696b1757a58584e9538a3b515ea742e9cc9465807340be"
                }
            },
            "operationName": "homeSection",
            "variables":  {
                "sectionItemsLimit": 20,
                "sectionItemsOffset": 0,
                "sp_t": access_token,
                "timeZone": time_zone,
                "country": country,
                "uri": section_uri
            },
        });

        let request =
            &RequestBuilder::new("pathfinder/v2/query".to_string(), Method::Post, Some(json))
                .set_base_uri("api-partner.spotify.com")
                .header("User-Agent", Self::user_agent());

        // Extract the playlists
        let cache_key = Self::cache_key(&format!("{section_uri}:{country}:{time_zone}"));
        self.load_and_return_home_section(request, &cache_key, CachePolicy::Use)
    }

    pub fn get_made_for_you(&self) -> Result<MixedView, Error> {
        // 0JQ5DAUnp4wcj0bCb3wh3S -> Made for you
        self.get_section("spotify:section:0JQ5DAUnp4wcj0bCb3wh3S")
    }

    pub fn get_top_mixes(&self) -> Result<MixedView, Error> {
        // 0JQ5DAnM3wGh0gz1MXnu89 -> Top mixes
        self.get_section("spotify:section:0JQ5DAnM3wGh0gz1MXnu89")
    }

    pub fn recommended_stations(&self) -> Result<MixedView, Error> {
        // 0JQ5DAnM3wGh0gz1MXnu3R -> Recommended stations
        self.get_section("spotify:section:0JQ5DAnM3wGh0gz1MXnu3R")
    }

    pub fn uniquely_yours(&self) -> Result<MixedView, Error> {
        // 0JQ5DAUnp4wcj0bCb3wh3S -> Uniquely yours
        self.get_section("spotify:section:0JQ5DAUnp4wcj0bCb3wh3S")
    }

    pub fn best_of_artists(&self) -> Result<MixedView, Error> {
        // 0JQ5DAnM3wGh0gz1MXnu3n -> Best of artists
        self.get_section("spotify:section:0JQ5DAnM3wGh0gz1MXnu3n")
    }

    // Need to make a mix of it!
    pub fn jump_back_in(&self) -> Result<MixedView, Error> {
        // 0JQ5DAIiKWzVFULQfUm85X -> Jump back in
        self.get_section("spotify:section:0JQ5DAIiKWzVFULQfUm85X")
    }

    // Shows
    pub fn your_shows(&self) -> Result<MixedView, Error> {
        // 0JQ5DAnM3wGh0gz1MXnu3N -> Your shows
        self.get_section("spotify:section:0JQ5DAnM3wGh0gz1MXnu3N")
    }

    pub fn shows_that_you_might_like(&self) -> Result<MixedView, Error> {
        // 0JQ5DAnM3wGh0gz1MXnu3P -> Shows that you might like
        self.get_section("spotify:section:0JQ5DAnM3wGh0gz1MXnu3P")
    }
}

/// Playlist endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-a-list-of-current-users-playlists
    pub fn get_playlists(&self) -> Result<Vector<Playlist>, Error> {
        let result: rspotify::model::Page<rspotify::model::SimplifiedPlaylist> = self
            .load_cached_value_rspotify("playlists", "all", CachePolicy::Use, || {
                self.rspotify_call(|| self.rspotify.current_user_playlists_manual(Some(50), None))
            })?;
        Ok(result
            .items
            .into_iter()
            .map(|playlist| self.playlist_from_simplified(playlist))
            .collect())
    }

    pub fn follow_playlist(&self, id: &str) -> Result<(), Error> {
        let request =
            &RequestBuilder::new(format!("v1/playlists/{id}/followers"), Method::Put, None)
                .set_body(Some(json!({"public": false})));
        self.request(request)?;
        self.cache.clear_bucket("playlists");
        Ok(())
    }

    pub fn unfollow_playlist(&self, id: &str) -> Result<(), Error> {
        let request =
            &RequestBuilder::new(format!("v1/playlists/{id}/followers"), Method::Delete, None);
        self.request(request)?;
        self.cache.clear_bucket("playlists");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-playlist
    pub fn get_playlist(&self, id: &str) -> Result<Playlist, Error> {
        let playlist_id = PlaylistId::from_id_or_uri(id)
            .map_err(|_| Error::WebApiError("Invalid playlist id".to_string()))?;
        let result: rspotify::model::FullPlaylist =
            self.load_cached_value_rspotify("playlist", id, CachePolicy::Use, || {
                self.rspotify_call(|| self.rspotify.playlist(playlist_id.as_ref(), None, None))
            })?;
        Ok(self.playlist_from_full(result))
    }

    // https://developer.spotify.com/documentation/web-api/reference/get-playlists-tracks
    pub fn get_playlist_tracks_page(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Arc<Track>>, Error> {
        let playlist_id = PlaylistId::from_id_or_uri(id)
            .map_err(|_| Error::WebApiError("Invalid playlist id".to_string()))?;
        let page_key = format!("{id}-o{offset}-l{limit}");
        let page: rspotify::model::Page<rspotify::model::PlaylistItem> = self
            .load_cached_value_rspotify("playlist-tracks", &page_key, CachePolicy::Use, || {
                self.rspotify_call(|| {
                    self.rspotify.playlist_items_manual(
                        playlist_id.as_ref(),
                        None,
                        Some(Market::FromToken),
                        Some(limit as u32),
                        Some(offset as u32),
                    )
                })
            })?;

        let local_track_manager = self.local_track_manager.lock();
        let items = page
            .items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let mut track = match item.track {
                    Some(PlayableItem::Track(track)) => {
                        let track = self.rspotify_to::<Track, _>(&track).ok()?;
                        Arc::new(track)
                    }
                    Some(PlayableItem::Unknown(json)) => {
                        local_track_manager.find_local_track(json)?
                    }
                    _ => return None,
                };
                Arc::make_mut(&mut track).track_pos = page.offset as usize + index;
                Some(track)
            })
            .collect();

        Ok(Page {
            items,
            limit: page.limit as usize,
            offset: page.offset as usize,
            total: page.total as usize,
        })
    }

    pub fn get_playlist_tracks_all(&self, id: &str) -> Result<Vector<Arc<Track>>, Error> {
        let mut all = Vector::new();
        let mut offset = 0usize;
        loop {
            let page = self.get_playlist_tracks_page(id, offset, 100)?;
            offset = page.offset + page.limit;
            all.append(page.items);
            if offset >= page.total {
                break;
            }
        }
        Ok(all)
    }

    pub fn change_playlist_details(&self, id: &str, name: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new(format!("v1/playlists/{id}/tracks"), Method::Get, None)
            .set_body(Some(json!({ "name": name })));
        self.request(request)?;
        self.cache.remove("playlist", id);
        self.cache.clear_bucket("playlists");
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/add-tracks-to-playlist
    pub fn add_track_to_playlist(&self, playlist_id: &str, track_uri: &str) -> Result<(), Error> {
        let request = &RequestBuilder::new(
            format!("v1/playlists/{playlist_id}/tracks"),
            Method::Post,
            None,
        )
        .query("uris", track_uri);
        self.request(request)?;
        self.cache.clear_bucket("playlist-tracks");
        self.cache.remove("playlist", playlist_id);
        Ok(())
    }

    // https://developer.spotify.com/documentation/web-api/reference/remove-tracks-playlist
    pub fn remove_track_from_playlist(
        &self,
        playlist_id: &str,
        track_id: TrackId,
        track_pos: usize,
    ) -> Result<(), Error> {
        self.remove_tracks_from_playlist(playlist_id, &[(track_id, track_pos)])
    }

    // https://developer.spotify.com/documentation/web-api/reference/remove-tracks-playlist
    pub fn remove_tracks_from_playlist(
        &self,
        playlist_id: &str,
        items: &[(TrackId, usize)],
    ) -> Result<(), Error> {
        if items.is_empty() {
            return Ok(());
        }

        let mut uri_positions: HashMap<String, Vec<usize>> = HashMap::new();
        let mut position_only: Vec<usize> = Vec::new();

        for (track_id, pos) in items {
            if let Some(uri) = track_id.0.to_uri() {
                uri_positions.entry(uri).or_default().push(*pos);
            } else {
                position_only.push(*pos);
            }
        }

        if !uri_positions.is_empty() {
            let mut tracks = Vec::with_capacity(uri_positions.len());
            for (uri, mut positions) in uri_positions {
                positions.sort_unstable();
                tracks.push(json!({
                    "uri": uri,
                    "positions": positions,
                }));
            }

            let request = &RequestBuilder::new(
                format!("v1/playlists/{playlist_id}/tracks"),
                Method::Delete,
                None,
            )
            .set_body(Some(json!({ "tracks": tracks })));
            self.request(request)?;
            self.cache.clear_bucket("playlist-tracks");
            self.cache.remove("playlist", playlist_id);
        }

        if !position_only.is_empty() {
            position_only.sort_unstable_by(|a, b| b.cmp(a));
            self.remove_track_positions_request(playlist_id, &position_only)?;
        }
        Ok(())
    }

    fn remove_track_positions_request(
        &self,
        playlist_id: &str,
        track_positions: &[usize],
    ) -> Result<(), Error> {
        let request = &RequestBuilder::new(
            format!("v1/playlists/{playlist_id}/tracks"),
            Method::Delete,
            None,
        )
        .set_body(Some(json!({ "positions": track_positions })));
        self.request(request)?;
        self.cache.clear_bucket("playlist-tracks");
        self.cache.remove("playlist", playlist_id);
        Ok(())
    }
}

/// Search endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/search/
    pub fn search(
        &self,
        query: &str,
        topics: &[SearchTopic],
        limit: usize,
    ) -> Result<SearchResults, Error> {
        let type_query_param = topics.iter().map(SearchTopic::as_str).join(",");
        let cache_key = Self::cache_key(&format!("{query}:{type_query_param}:{limit}"));
        let result: rspotify::model::SearchMultipleResult =
            self.load_cached_value_rspotify("search", &cache_key, CachePolicy::Use, || {
                let types = topics.iter().map(|topic| match topic {
                    SearchTopic::Artist => SearchType::Artist,
                    SearchTopic::Album => SearchType::Album,
                    SearchTopic::Track => SearchType::Track,
                    SearchTopic::Playlist => SearchType::Playlist,
                    SearchTopic::Show => SearchType::Show,
                });
                self.rspotify_call(|| {
                    self.rspotify.search_multiple(
                        query,
                        types,
                        Some(Market::FromToken),
                        None,
                        Some(limit as u32),
                        None,
                    )
                })
            })?;

        let artists = result
            .artists
            .map_or_else(Vector::new, |page| self.rspotify_vec(page.items));
        let albums = result.albums.map_or_else(Vector::new, |page| {
            self.rspotify_vec::<Album, _>(page.items)
                .into_iter()
                .map(Arc::new)
                .collect()
        });
        let tracks = result.tracks.map_or_else(Vector::new, |page| {
            self.rspotify_vec::<Track, _>(page.items)
                .into_iter()
                .map(Arc::new)
                .collect()
        });
        let playlists = result
            .playlists
            .map_or_else(Vector::new, |page| self.rspotify_vec(page.items));
        let shows = result.shows.map_or_else(Vector::new, |page| {
            self.rspotify_vec::<Show, _>(page.items)
                .into_iter()
                .map(Arc::new)
                .collect()
        });
        let topic = (topics.len() == 1).then_some(topics[0]);

        Ok(SearchResults {
            query: query.into(),
            topic,
            artists,
            albums,
            tracks,
            playlists,
            shows,
        })
    }

    pub fn load_spotify_link(&self, link: &SpotifyUrl) -> Result<Nav, Error> {
        let nav = match link {
            SpotifyUrl::Playlist(id) => Nav::PlaylistDetail(self.get_playlist(id)?.link()),
            SpotifyUrl::Artist(id) => Nav::ArtistDetail(self.get_artist(id)?.link()),
            SpotifyUrl::Album(id) => Nav::AlbumDetail(self.get_album(id)?.data.link(), None),
            SpotifyUrl::Show(id) => Nav::ShowDetail(self.get_show(id)?.data.link()),
            SpotifyUrl::Track(id) => {
                let track = self.get_track(id)?;
                let album = track.album.clone().ok_or_else(|| {
                    Error::WebApiError("Track was found but has no album".to_string())
                })?;
                Nav::AlbumDetail(album, Some(track.id))
            }
        };
        Ok(nav)
    }
}

/// Recommendation endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-recommendations
    pub fn get_recommendations(
        &self,
        data: Arc<RecommendationsRequest>,
    ) -> Result<Recommendations, Error> {
        let seed_artists = data.seed_artists.iter().map(|link| &link.id).join(", ");
        let seed_tracks = data
            .seed_tracks
            .iter()
            .map(|track| track.0.to_base62())
            .join(", ");

        let mut request = RequestBuilder::new("v1/recommendations", Method::Get, None)
            .query("marker", "from_token")
            .query("limit", "100")
            .query("seed_artists", &seed_artists)
            .query("seed_tracks", &seed_tracks);

        fn add_range_param(
            req: RequestBuilder,
            r: Range<impl ToString>,
            s: &str,
        ) -> RequestBuilder {
            let mut req = req;
            if let Some(v) = r.min {
                req = req.query(format!("min_{s}"), v.to_string());
            }
            if let Some(v) = r.max {
                req = req.query(format!("max_{s}"), v.to_string());
            }
            if let Some(v) = r.target {
                req = req.query(format!("target_{s}"), v.to_string());
            }
            req
        }

        request = add_range_param(request, data.params.duration_ms, "duration_ms");
        request = add_range_param(request, data.params.popularity, "popularity");
        request = add_range_param(request, data.params.key, "key");
        request = add_range_param(request, data.params.mode, "mode");
        request = add_range_param(request, data.params.tempo, "tempo");
        request = add_range_param(request, data.params.time_signature, "time_signature");
        request = add_range_param(request, data.params.acousticness, "acousticness");
        request = add_range_param(request, data.params.danceability, "danceability");
        request = add_range_param(request, data.params.energy, "energy");
        request = add_range_param(request, data.params.instrumentalness, "instrumentalness");
        request = add_range_param(request, data.params.liveness, "liveness");
        request = add_range_param(request, data.params.loudness, "loudness");
        request = add_range_param(request, data.params.speechiness, "speechiness");
        request = add_range_param(request, data.params.valence, "valence");

        let cache_key = Self::cache_key(&request.build());
        let result: Cached<Recommendations> =
            self.load_cached_with(&request, "recommendations", &cache_key, CachePolicy::Use)?;
        let mut result = result.data;
        result.request = data;
        Ok(result)
    }
}

/// Track endpoints.
impl WebApi {
    // https://developer.spotify.com/documentation/web-api/reference/get-audio-analysis/
    pub fn _get_audio_analysis(&self, track_id: &str) -> Result<AudioAnalysis, Error> {
        let request =
            &RequestBuilder::new(format!("v1/audio-analysis/{track_id}"), Method::Get, None);
        let result = self.load_cached(request, "audio-analysis", track_id)?;
        Ok(result.data)
    }
}

/// Image endpoints.
impl WebApi {
    pub fn get_cached_image(&self, uri: &Arc<str>) -> Option<ImageBuf> {
        self.cache.get_image(uri)
    }

    pub fn get_image(&self, uri: Arc<str>) -> Result<ImageBuf, Error> {
        if let Some(cached_image) = self.cache.get_image(&uri) {
            return Ok(cached_image);
        }

        if let Some(disk_cached_image) = self.cache.get_image_from_disk(&uri) {
            self.cache.set_image(uri.clone(), disk_cached_image.clone());
            return Ok(disk_cached_image);
        }

        // Split the URI into its components
        let uri_clone = uri.clone();
        let parsed = url::Url::parse(&uri_clone).unwrap();

        let protocol = parsed.scheme();
        let base_uri = parsed.host_str().unwrap();
        let path = parsed.path().trim_start_matches('/');

        let mut queries = std::collections::HashMap::new();
        for (k, v) in parsed.query_pairs() {
            queries.insert(k.to_string(), v.to_string());
        }

        let request = RequestBuilder::new(path, Method::Get, None)
            .set_protocol(protocol)
            .set_base_uri(base_uri);

        let response = self.request(&request)?;
        let mut body = Vec::new();
        response.into_body().into_reader().read_to_end(&mut body)?;

        let format = match infer::get(body.as_slice()) {
            Some(kind) if kind.mime_type() == "image/jpeg" => Some(ImageFormat::Jpeg),
            Some(kind) if kind.mime_type() == "image/png" => Some(ImageFormat::Png),
            Some(kind) if kind.mime_type() == "image/webp" => Some(ImageFormat::WebP),
            _ => None,
        };

        // Save raw image data to disk cache
        self.cache.save_image_to_disk(&uri, &body);

        let image = if let Some(format) = format {
            image::load_from_memory_with_format(&body, format)?
        } else {
            image::load_from_memory(&body)?
        };
        let image_buf = ImageBuf::from_dynamic_image(image);
        self.cache.set_image(uri, image_buf.clone());
        Ok(image_buf)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::WebApiError(err.to_string())
    }
}

impl From<ureq::Error> for Error {
    fn from(err: ureq::Error) -> Self {
        Error::WebApiError(err.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::WebApiError(err.to_string())
    }
}

impl From<image::ImageError> for Error {
    fn from(err: image::ImageError) -> Self {
        Error::WebApiError(err.to_string())
    }
}

#[derive(Debug, Clone)]
enum Method {
    Post,
    Put,
    Delete,
    Get,
}

// Creating a new URI builder so aid in the creation of uris with extendable queries.
#[derive(Debug, Clone)]
struct RequestBuilder {
    protocol: String,
    base_uri: String,
    path: String,
    queries: HashMap<String, String>,
    headers: HashMap<String, String>,
    method: Method,
    body: Option<serde_json::Value>,
}

impl RequestBuilder {
    // By default, we use https and the api.spotify.com
    fn new(path: impl Display, method: Method, body: Option<serde_json::Value>) -> Self {
        Self {
            protocol: "https".to_string(),
            base_uri: "api.spotify.com".to_string(),
            path: path.to_string(),
            queries: HashMap::new(),
            headers: HashMap::new(),
            method,
            body,
        }
    }

    fn query(mut self, key: impl Display, value: impl Display) -> Self {
        self.queries.insert(key.to_string(), value.to_string());
        self
    }

    fn header(mut self, key: impl Display, value: impl Display) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    fn set_protocol(mut self, protocol: impl Display) -> Self {
        self.protocol = protocol.to_string();
        self
    }
    fn get_headers(&self) -> &HashMap<String, String> {
        &self.headers
    }
    fn get_body(&self) -> Option<&serde_json::Value> {
        self.body.as_ref()
    }
    fn set_body(mut self, body: Option<serde_json::Value>) -> Self {
        self.body = body;
        self
    }
    fn get_method(&self) -> &Method {
        &self.method
    }
    #[allow(dead_code)]
    fn set_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }
    fn set_base_uri(mut self, url: impl Display) -> Self {
        self.base_uri = url.to_string();
        self
    }
    fn build(&self) -> String {
        let mut url = format!("{}://{}/{}", self.protocol, self.base_uri, self.path);
        if !self.queries.is_empty() {
            url.push('?');
            url.push_str(
                &self
                    .queries
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        url
    }
}

impl RequestGate {
    fn new(max_in_flight: usize) -> Self {
        Self {
            state: Mutex::new(RequestGateState { in_flight: 0 }),
            waiters: Condvar::new(),
            max_in_flight,
        }
    }

    fn acquire(&self) -> RequestPermit<'_> {
        let mut state = self.state.lock();
        while state.in_flight >= self.max_in_flight {
            self.waiters.wait(&mut state);
        }
        state.in_flight += 1;
        RequestPermit { gate: self }
    }
}

impl Drop for RequestPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock();
        state.in_flight = state.in_flight.saturating_sub(1);
        self.gate.waiters.notify_one();
    }
}
