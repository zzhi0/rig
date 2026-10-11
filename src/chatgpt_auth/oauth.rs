use std::{
    future::IntoFuture,
    sync::{Arc, Mutex},
};

use axum::{Router, extract::Query, routing::get};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::{Client, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, sync::oneshot};

pub(super) const ISSUER: &str = "https://auth.openai.com";
pub(super) const RESOURCE: &str = "https://api.openai.com/v1";
pub(super) const TOKEN_ENDPOINT: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub(super) const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

pub(super) struct PendingLogin {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
    pub client_id: Option<String>,
}

impl PendingLogin {
    pub fn new(redirect_uri: String, client_id: Option<String>) -> Result<Self, String> {
        Ok(Self {
            state: random_value()?,
            nonce: random_value()?,
            verifier: random_value()?,
            redirect_uri,
            client_id,
        })
    }

    pub fn authorize_url(&self, host_id: &str, email: Option<&str>) -> Url {
        let mut url = Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            (
                "client_id",
                self.client_id.as_deref().unwrap_or("dynamic_agent_client"),
            ),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", SCOPES),
            ("resource", RESOURCE),
            ("state", self.state.as_str()),
            ("nonce", self.nonce.as_str()),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                URL_SAFE_NO_PAD
                    .encode(Sha256::digest(&self.verifier))
                    .as_str(),
            ),
        ]);
        if self.client_id.is_none() {
            query.append_pair("agent_name_hint", "Rig");
        }
        if let Some(email) = email {
            query.append_pair("login_hint", email);
        }
        drop(query);
        url
    }

    pub fn accept(&self, callback: Callback) -> Result<(String, String), String> {
        if callback.state.as_deref() != Some(self.state.as_str()) {
            return Err("ChatGPT login callback state did not match".into());
        }
        if callback.error.is_some() {
            return Err("ChatGPT authorization was denied or failed".into());
        }
        let client_id = match (&self.client_id, callback.client_id) {
            (Some(expected), Some(actual)) if expected != &actual => {
                return Err("ChatGPT login callback changed the registered client ID".into());
            }
            (Some(expected), _) => expected.clone(),
            (None, Some(issued)) if !issued.is_empty() && issued != "dynamic_agent_client" => {
                issued
            }
            _ => return Err("ChatGPT registration did not return an issued client ID".into()),
        };
        let code = callback
            .code
            .filter(|code| !code.is_empty())
            .ok_or("ChatGPT login callback did not return an authorization code")?;
        Ok((client_id, code))
    }
}

#[derive(Deserialize)]
pub(super) struct Callback {
    pub state: Option<String>,
    pub code: Option<String>,
    pub client_id: Option<String>,
    pub error: Option<String>,
}

pub(super) async fn receive_callback(listener: TcpListener) -> Result<Callback, String> {
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let router = Router::new().route(
        "/auth/callback",
        get(move |Query(callback): Query<Callback>| {
            let sender = sender.clone();
            async move {
                if let Some(sender) = sender.lock().unwrap().take() {
                    let _ = sender.send(callback);
                }
                "You can return to Rig. Check the terminal for the login result."
            }
        }),
    );
    let (stop, stopped) = oneshot::channel();
    let server = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);
    tokio::select! {
        callback = receiver => {
            let _ = stop.send(());
            server.await.map_err(|error| format!("ChatGPT callback server failed: {error}"))?;
            callback.map_err(|_| "ChatGPT callback server closed before authorization".into())
        }
        result = &mut server => {
            result.map_err(|error| format!("ChatGPT callback server failed: {error}"))?;
            Err("ChatGPT callback server stopped before authorization".into())
        }
    }
}

#[derive(Deserialize)]
pub(super) struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub revocation_endpoint: String,
}

pub(super) async fn discover(client: &Client, url: &str) -> Result<Discovery, String> {
    let metadata: Discovery =
        read_json(client.get(url).send().await, "OpenAI identity discovery").await?;
    if metadata.issuer != ISSUER {
        return Err("OpenAI identity discovery returned an unexpected issuer".into());
    }
    Ok(metadata)
}

#[derive(Deserialize)]
pub(super) struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
}

pub(super) async fn request_token(
    client: &Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<TokenResponse, String> {
    let tokens: TokenResponse = read_json(
        client.post(url).form(form).send().await,
        "ChatGPT token exchange",
    )
    .await?;
    if !tokens.token_type.eq_ignore_ascii_case("Bearer") {
        return Err("ChatGPT token exchange returned an unsupported token type".into());
    }
    if !tokens
        .scope
        .split_whitespace()
        .any(|scope| scope == DIRECT_SCOPE)
    {
        return Err(
            "ChatGPT plan use was not authorized; run rig login and grant plan access".into(),
        );
    }
    Ok(tokens)
}

#[derive(Deserialize)]
pub(super) struct Identity {
    pub sub: String,
    pub email: Option<String>,
    pub nonce: Option<String>,
}

pub(super) async fn validate_identity(
    client: &Client,
    discovery: &Discovery,
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
) -> Result<Identity, String> {
    let keys = read_json(
        client.get(&discovery.jwks_uri).send().await,
        "OpenAI signing keys",
    )
    .await?;
    verify_identity(&keys, token, client_id, nonce, subject)
}

pub(super) fn verify_identity(
    keys: &JwkSet,
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
) -> Result<Identity, String> {
    let header = decode_header(token).map_err(|_| "Invalid ChatGPT identity token header")?;
    let key = header
        .kid
        .as_deref()
        .and_then(|kid| keys.find(kid))
        .ok_or("ChatGPT identity token signing key was not found")?;
    let key = DecodingKey::from_jwk(key).map_err(|_| "Invalid OpenAI identity signing key")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.leeway = 0;
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    let identity = decode::<Identity>(token, &key, &validation)
        .map_err(|_| "ChatGPT identity token signature, issuer, audience or expiry is invalid")?
        .claims;
    if nonce.is_some() && identity.nonce.as_deref() != nonce {
        return Err("ChatGPT identity token nonce did not match".into());
    }
    if identity.sub.is_empty() || subject.is_some_and(|subject| subject != identity.sub) {
        return Err("ChatGPT login identity did not match the registered account".into());
    }
    Ok(identity)
}

async fn read_json<T: serde::de::DeserializeOwned>(
    response: Result<reqwest::Response, reqwest::Error>,
    operation: &str,
) -> Result<T, String> {
    let response = response.map_err(|_| format!("{operation} request failed"))?;
    if !response.status().is_success() {
        return Err(format!(
            "{operation} failed with HTTP {}",
            response.status()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| format!("{operation} returned an invalid response"))
}

pub(super) fn host_id() -> Result<String, String> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot generate ChatGPT host identity")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn random_value() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Cannot generate ChatGPT login randomness")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
