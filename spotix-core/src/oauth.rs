use crate::error::Error;
use oauth2::{
    basic::BasicClient, reqwest, AuthUrl, AuthorizationCode, ClientId, CsrfToken, EndpointNotSet,
    EndpointSet, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, Scope,
    TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;

pub fn listen_for_callback_parameter(
    socket_address: SocketAddr,
    timeout: Duration,
    parameter_name: &'static str,
) -> Result<String, Error> {
    log::info!("starting callback listener for '{parameter_name}' on {socket_address:?}",);

    // Create a simpler, linear flow
    // 1. Bind the listener
    let listener = match TcpListener::bind(socket_address) {
        Ok(l) => {
            log::info!("listener bound successfully");
            l
        }
        Err(e) => {
            log::error!("Failed to bind listener: {e}");
            return Err(Error::IoError(e));
        }
    };

    // 2. Set up the channel for communication
    let (tx, rx) = mpsc::channel::<Result<String, Error>>();

    // 3. Spawn the thread
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            handle_callback_connection(&mut stream, &tx, parameter_name);
        } else {
            log::error!("Failed to accept connection on callback listener");
            let _ = tx.send(Err(Error::IoError(std::io::Error::other(
                "Failed to accept connection",
            ))));
        }
    });

    // 4. Wait for the result with timeout
    let result = match rx.recv_timeout(timeout) {
        Ok(r) => r,
        Err(e) => {
            log::error!("Timed out or channel error: {e}");
            return Err(Error::from(e));
        }
    };

    // 5. Wait for thread completion
    if handle.join().is_err() {
        log::warn!("thread join failed, but continuing with result");
    }

    // 6. Return the result
    result
}

/// Handles an incoming TCP connection for a generic OAuth callback.
fn handle_callback_connection(
    stream: &mut TcpStream,
    tx: &mpsc::Sender<Result<String, Error>>,
    parameter_name: &'static str,
) {
    let mut reader = BufReader::new(&mut *stream);
    let mut request_line = String::new();

    if reader.read_line(&mut request_line).is_ok() {
        match extract_parameter_from_request(&request_line, parameter_name) {
            Some(value) => {
                log::info!("received callback parameter '{parameter_name}'.");
                send_success_response(stream);
                let _ = tx.send(Ok(value));
            }
            None => {
                let err_msg = format!(
                    "Failed to extract parameter '{parameter_name}' from request: {request_line}",
                );
                log::error!("{err_msg}");
                let _ = tx.send(Err(Error::OAuthError(err_msg)));
            }
        }
    } else {
        log::error!("Failed to read request line from callback.");
        let _ = tx.send(Err(Error::IoError(std::io::Error::other(
            "Failed to read request line",
        ))));
    }
}

/// Extracts a specified query parameter from an HTTP request line.
fn extract_parameter_from_request(request_line: &str, parameter_name: &str) -> Option<String> {
    request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| Url::parse(&format!("http://localhost{path}")).ok())
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == parameter_name)
                .map(|(_, value)| value.into_owned())
        })
}

pub fn get_authcode_listener(
    socket_address: SocketAddr,
    timeout: Duration,
) -> Result<AuthorizationCode, Error> {
    listen_for_callback_parameter(socket_address, timeout, "code").map(AuthorizationCode::new)
}

pub fn send_success_response(stream: &mut TcpStream) {
    let response = "HTTP/1.1 200 OK\r\n\r\n\
        <html>\
        <head>\
            <style>\
                body {\
                    background-color: #121212;\
                    color: #ffffff;\
                    font-family: sans-serif;\
                    display: flex;\
                    justify-content: center;\
                    align-items: center;\
                    height: 100vh;\
                    margin: 0;\
                }\
                a {\
                    color: #aaaaaa;\
                    text-decoration: underline;\
                    cursor: pointer;\
                }\
            </style>\
        </head>\
        <body>\
            <div>Successfully authenticated! You can close this window now.</div>\
        </body>\
        </html>";
    let _ = stream.write_all(response.as_bytes());
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_unix: Option<u64>,
}

impl OAuthToken {
    pub fn is_expired(&self, buffer: Duration) -> bool {
        let Some(expires_at_unix) = self.expires_at_unix else {
            return false;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let buffer_secs = buffer.as_secs();
        expires_at_unix <= now.saturating_add(buffer_secs)
    }
}

fn create_spotify_oauth_client(
    redirect_port: u16,
    client_id: &str,
) -> BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet> {
    let redirect_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), redirect_port);
    let redirect_uri = format!("http://{redirect_address}/login");

    BasicClient::new(ClientId::new(client_id.to_string()))
        .set_auth_uri(
            AuthUrl::new("https://accounts.spotify.com/authorize".to_string())
                .expect("Invalid auth URL"),
        )
        .set_token_uri(
            TokenUrl::new("https://accounts.spotify.com/api/token".to_string())
                .expect("Invalid token URL"),
        )
        .set_redirect_uri(RedirectUrl::new(redirect_uri).expect("Invalid redirect URL"))
}

fn build_oauth_token(
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<Duration>,
) -> OAuthToken {
    let expires_at_unix = expires_in
        .and_then(|expires_in| SystemTime::now().checked_add(expires_in))
        .and_then(|expires_at| expires_at.duration_since(UNIX_EPOCH).ok())
        .map(|expires_at| expires_at.as_secs());

    OAuthToken {
        access_token,
        refresh_token,
        expires_at_unix,
    }
}

pub fn generate_auth_url(redirect_port: u16, client_id: &str) -> (String, PkceCodeVerifier) {
    let client = create_spotify_oauth_client(redirect_port, client_id);
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (auth_url, _) = client
        .authorize_url(CsrfToken::new_random)
        .add_scopes(get_scopes())
        .set_pkce_challenge(pkce_challenge)
        .url();

    (auth_url.to_string(), pkce_verifier)
}

pub fn exchange_code_for_token(
    redirect_port: u16,
    code: AuthorizationCode,
    pkce_verifier: PkceCodeVerifier,
    client_id: &str,
) -> Result<OAuthToken, Error> {
    let client = create_spotify_oauth_client(redirect_port, client_id);

    let http_client = reqwest::blocking::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| Error::OAuthError(format!("Failed to build HTTP client: {err}")))?;

    let token_response = client
        .exchange_code(code)
        .set_pkce_verifier(pkce_verifier)
        .request(&http_client)
        .map_err(|err| Error::OAuthError(format!("Failed to exchange code: {err}")))?;

    let access_token = token_response.access_token().secret().to_string();
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_string());

    Ok(build_oauth_token(
        access_token,
        refresh_token,
        token_response.expires_in(),
    ))
}

pub fn refresh_access_token(refresh_token: &str, client_id: &str) -> Result<OAuthToken, Error> {
    let client = create_spotify_oauth_client(8888, client_id);
    let http_client = reqwest::blocking::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| Error::OAuthError(format!("Failed to build HTTP client: {err}")))?;

    let token_response = client
        .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
        .request(&http_client)
        .map_err(|err| Error::OAuthError(format!("Failed to refresh token: {err}")))?;

    let access_token = token_response.access_token().secret().to_string();
    let refresh = token_response
        .refresh_token()
        .map(|token| token.secret().to_string())
        .or_else(|| Some(refresh_token.to_string()));

    Ok(build_oauth_token(
        access_token,
        refresh,
        token_response.expires_in(),
    ))
}

fn get_scopes() -> Vec<Scope> {
    crate::session::access_token::ACCESS_SCOPES
        .split(',')
        .map(|s| Scope::new(s.trim().to_string()))
        .collect()
}
