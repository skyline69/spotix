use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use chrono::Duration as ChronoDuration;
use librespot_core::session::Session as LibrespotSession;
use rspotify::{
    ClientError, ClientResult, Config, Credentials, OAuth, Token,
    clients::{BaseClient, OAuthClient},
    http::HttpClient,
    sync::Mutex,
};
use tokio::sync::Mutex as TokioMutex;

use maybe_async::maybe_async;

pub struct RSpotifyClient {
    creds: Credentials,
    oauth: OAuth,
    config: Config,
    token: Arc<Mutex<Option<Token>>>,
    http: HttpClient,
    session: Arc<TokioMutex<Option<LibrespotSession>>>,
}

impl Default for RSpotifyClient {
    fn default() -> Self {
        Self {
            creds: Credentials::default(),
            oauth: OAuth::default(),
            config: Config {
                token_refreshing: true,
                ..Default::default()
            },
            token: Arc::new(Mutex::new(None)),
            http: HttpClient::default(),
            session: Arc::new(TokioMutex::new(None)),
        }
    }
}

impl RSpotifyClient {
    pub fn new(_proxy_url: Option<&str>, client_id: Option<&str>) -> Self {
        let creds = match client_id {
            Some(id) => Credentials::new_pkce(id),
            None => Credentials::default(),
        };
        Self {
            creds,
            ..Default::default()
        }
    }

    pub async fn set_session(&self, session: LibrespotSession) {
        *self.session.lock().await = Some(session);
    }

    pub async fn set_token(&self, token: Token) {
        *self.token.lock().await.unwrap() = Some(token);
    }

    pub async fn ensure_token(&self) -> bool {
        let mut guard = self.token.lock().await.unwrap();
        if let Some(token) = guard.as_ref()
            && !token.is_expired()
        {
            return true;
        }
        if guard.is_some() {
            *guard = None;
        }
        match self.login5_token().await {
            Ok(Some(token)) => {
                *guard = Some(token);
                true
            }
            Ok(None) => false,
            Err(err) => {
                log::warn!("webapi: failed to fetch login5 token: {err}");
                false
            }
        }
    }

    async fn login5_token(&self) -> ClientResult<Option<Token>> {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        let session = self.session.lock().await.clone();
        let Some(session) = session else {
            return Ok(None);
        };
        if session.is_invalid() {
            return Err(ClientError::InvalidToken);
        }

        let token = match tokio::time::timeout(TIMEOUT, session.login5().auth_token()).await {
            Ok(Ok(token)) => token,
            Ok(Err(err)) => {
                log::warn!("webapi: librespot login5 token failed: {err}");
                return Err(ClientError::InvalidToken);
            }
            Err(_) => {
                log::warn!("webapi: librespot login5 token timed out");
                if !session.is_invalid() {
                    session.shutdown();
                }
                return Err(ClientError::InvalidToken);
            }
        };

        let expires_in =
            ChronoDuration::from_std(token.expires_in).map_err(|_| ClientError::InvalidToken)?;
        let expires_at = chrono::Utc::now() + expires_in;
        Ok(Some(Token {
            access_token: token.access_token,
            expires_in,
            expires_at: Some(expires_at),
            refresh_token: None,
            scopes: HashSet::new(),
        }))
    }
}

impl Clone for RSpotifyClient {
    fn clone(&self) -> Self {
        Self {
            creds: self.creds.clone(),
            oauth: self.oauth.clone(),
            config: self.config.clone(),
            token: Arc::clone(&self.token),
            http: self.http.clone(),
            session: Arc::clone(&self.session),
        }
    }
}

impl fmt::Debug for RSpotifyClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RSpotifyClient")
            .field("creds", &self.creds)
            .field("oauth", &self.oauth)
            .field("config", &self.config)
            .finish()
    }
}

#[maybe_async]
impl BaseClient for RSpotifyClient {
    fn get_http(&self) -> &HttpClient {
        &self.http
    }

    fn get_token(&self) -> Arc<Mutex<Option<Token>>> {
        Arc::clone(&self.token)
    }

    fn get_creds(&self) -> &Credentials {
        &self.creds
    }

    fn get_config(&self) -> &Config {
        &self.config
    }

    async fn refetch_token(&self) -> ClientResult<Option<Token>> {
        self.login5_token().await
    }
}

#[maybe_async]
impl OAuthClient for RSpotifyClient {
    fn get_oauth(&self) -> &OAuth {
        &self.oauth
    }

    async fn request_token(&self, _code: &str) -> ClientResult<()> {
        Err(ClientError::InvalidToken)
    }
}
