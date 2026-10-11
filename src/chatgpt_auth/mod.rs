mod oauth;
mod storage;
#[cfg(test)]
mod tests;

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::Client;
use tokio::{
    net::TcpListener,
    process::{Child, Command},
};

use oauth::{PendingLogin, RESOURCE, TOKEN_ENDPOINT, discover, request_token, validate_identity};
use storage::{Credentials, Registration, State, Storage};

const DISCOVERY: &str = "https://auth.openai.com/.well-known/openid-configuration";

#[derive(Clone)]
pub struct ChatGptAuth {
    storage: Storage,
    client: Client,
}

impl ChatGptAuth {
    pub fn new() -> Result<Self, String> {
        let config = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(config) => PathBuf::from(config),
            None => {
                PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".config")
            }
        };
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|error| format!("Cannot create ChatGPT login client: {error}"))?;
        Ok(Self {
            storage: Storage::new(config.join("rig")),
            client,
        })
    }

    pub async fn login(&self) -> Result<String, String> {
        let _lock = self.storage.lock()?;
        let mut state = match self.storage.load()? {
            Some(state) => state,
            None => {
                let state = State {
                    host_id: oauth::host_id()?,
                    registration: None,
                };
                self.storage.save(&state)?;
                state
            }
        };
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("Cannot start ChatGPT login callback: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("Cannot read ChatGPT callback address: {error}"))?;
        let pending = PendingLogin::new(
            format!("http://127.0.0.1:{}/auth/callback", address.port()),
            state
                .registration
                .as_ref()
                .map(|registration| registration.client_id.clone()),
        )?;
        let email = state
            .registration
            .as_ref()
            .and_then(|registration| registration.email.as_deref());
        let callback = authorize_in_browser(
            listener,
            pending.authorize_url(&state.host_id, email).as_str(),
        )
        .await?;
        let (client_id, code) = pending.accept(callback)?;
        if state.registration.is_none() {
            state.registration = Some(Registration {
                client_id,
                subject: None,
                email: None,
                credentials: None,
            });
            self.storage.save(&state)?;
        }
        self.exchange(&mut state, &pending, &code, TOKEN_ENDPOINT, DISCOVERY)
            .await
    }

    async fn exchange(
        &self,
        state: &mut State,
        pending: &PendingLogin,
        code: &str,
        token_endpoint: &str,
        discovery_endpoint: &str,
    ) -> Result<String, String> {
        let registration = state.registration.as_mut().unwrap();
        let tokens = request_token(
            &self.client,
            token_endpoint,
            &[
                ("grant_type", "authorization_code"),
                ("client_id", &registration.client_id),
                ("code", code),
                ("code_verifier", &pending.verifier),
                ("redirect_uri", &pending.redirect_uri),
                ("resource", RESOURCE),
            ],
        )
        .await?;
        let id_token = tokens
            .id_token
            .as_deref()
            .ok_or("ChatGPT login did not return an identity token")?;
        let discovery = discover(&self.client, discovery_endpoint).await?;
        let identity = validate_identity(
            &self.client,
            &discovery,
            id_token,
            &registration.client_id,
            Some(&pending.nonce),
            registration.subject.as_deref(),
        )
        .await?;
        let label = identity.email.as_ref().unwrap_or(&identity.sub).clone();
        registration.subject = Some(identity.sub);
        registration.email = identity.email;
        registration.credentials = Some(credentials(tokens, None)?);
        self.storage.save(state)?;
        Ok(label)
    }

    pub async fn access_token(&self) -> Result<String, String> {
        let _lock = self.storage.lock()?;
        let mut state = self
            .storage
            .load()?
            .ok_or("Not signed in to ChatGPT; run rig login")?;
        let registration = state
            .registration
            .as_mut()
            .ok_or("Not signed in to ChatGPT; run rig login")?;
        let credentials = registration
            .credentials
            .as_ref()
            .ok_or("Not signed in to ChatGPT; run rig login")?;
        if unix_time()? < credentials.expires_at {
            return Ok(credentials.access_token.clone());
        }
        self.refresh(&mut state, TOKEN_ENDPOINT, DISCOVERY).await
    }

    async fn refresh(
        &self,
        state: &mut State,
        token_endpoint: &str,
        discovery_endpoint: &str,
    ) -> Result<String, String> {
        let registration = state.registration.as_mut().unwrap();
        let saved = registration.credentials.as_ref().unwrap();
        let tokens = request_token(
            &self.client,
            token_endpoint,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &registration.client_id),
                ("refresh_token", &saved.refresh_token),
                ("resource", RESOURCE),
            ],
        )
        .await?;
        if let Some(id_token) = tokens.id_token.as_deref() {
            let discovery = discover(&self.client, discovery_endpoint).await?;
            validate_identity(
                &self.client,
                &discovery,
                id_token,
                &registration.client_id,
                None,
                registration.subject.as_deref(),
            )
            .await?;
        }
        let updated = credentials(tokens, Some(&saved.id_token))?;
        let token = updated.access_token.clone();
        registration.credentials = Some(updated);
        self.storage.save(state)?;
        Ok(token)
    }

    pub async fn logout(&self) -> Result<(), String> {
        let _lock = self.storage.lock()?;
        let Some(mut state) = self.storage.load()? else {
            return Ok(());
        };
        let Some(registration) = state.registration.as_ref() else {
            return Ok(());
        };
        let Some(_) = registration.credentials.as_ref() else {
            return Ok(());
        };
        let discovery = discover(&self.client, DISCOVERY).await?;
        self.revoke(&mut state, &discovery.revocation_endpoint)
            .await
    }

    async fn revoke(&self, state: &mut State, endpoint: &str) -> Result<(), String> {
        let registration = state.registration.as_mut().unwrap();
        let credentials = registration.credentials.as_ref().unwrap();
        let response = self
            .client
            .post(endpoint)
            .form(&[
                ("token", credentials.refresh_token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", registration.client_id.as_str()),
            ])
            .send()
            .await
            .map_err(|_| "ChatGPT logout request failed; login was retained")?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "ChatGPT logout failed with HTTP {}; login was retained",
                response.status()
            ));
        }
        registration.credentials = None;
        self.storage.save(state)
    }
}

fn credentials(
    tokens: oauth::TokenResponse,
    previous_id_token: Option<&str>,
) -> Result<Credentials, String> {
    Ok(Credentials {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        id_token: tokens
            .id_token
            .or_else(|| previous_id_token.map(str::to_owned))
            .ok_or("ChatGPT login did not return an identity token")?,
        expires_at: unix_time()? + tokens.expires_in,
        scopes: tokens.scope.split_whitespace().map(str::to_owned).collect(),
    })
}

fn unix_time() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("Cannot read the current time: {error}"))
}

async fn authorize_in_browser(listener: TcpListener, url: &str) -> Result<oauth::Callback, String> {
    let executable = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let browser = Command::new(executable)
        .arg(url)
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "Cannot launch the system browser for ChatGPT login")?;
    receive_authorization(listener, browser).await
}

async fn receive_authorization(
    listener: TcpListener,
    mut browser: Child,
) -> Result<oauth::Callback, String> {
    let callback = oauth::receive_callback(listener);
    tokio::pin!(callback);
    tokio::select! {
        callback = &mut callback => callback,
        status = browser.wait() => {
            let status = status.map_err(|_| "Cannot wait for the ChatGPT browser launcher")?;
            if !status.success() {
                return Err("The system browser failed to open ChatGPT login".into());
            }
            callback.await
        }
    }
}
