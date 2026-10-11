use super::*;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::{
    Json, Router,
    extract::Form,
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use serde_json::{Value, json};
use sha2::Digest;
use tokio::sync::oneshot;

struct TrackedListener {
    inner: TcpListener,
    dropped: Arc<AtomicBool>,
}

impl axum::serve::Listener for TrackedListener {
    type Io = tokio::net::TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        axum::serve::Listener::accept(&mut self.inner).await
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

impl Drop for TrackedListener {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn auth(directory: &tempfile::TempDir) -> ChatGptAuth {
    ChatGptAuth {
        storage: Storage::new(directory.path().join("rig")),
        client: Client::new(),
    }
}

fn state() -> State {
    State {
        host_id: "urn:uuid:test-host".into(),
        registration: Some(Registration {
            client_id: "oaiapp_rig_test".into(),
            subject: Some("test-account".into()),
            email: Some("test@example.com".into()),
            credentials: Some(Credentials {
                access_token: "old-access".into(),
                refresh_token: "old-refresh".into(),
                id_token: "old-id".into(),
                expires_at: 0,
                scopes: vec![oauth::DIRECT_SCOPE.into()],
            }),
        }),
    }
}

fn token_response(scope: &str) -> Value {
    json!({"access_token":"new-access", "refresh_token":"new-refresh", "token_type":"Bearer", "expires_in":3600, "scope":scope})
}

#[test]
fn token_expiry_uses_receipt_time_before_identity_validation() {
    let tokens = serde_json::from_value(token_response(oauth::DIRECT_SCOPE)).unwrap();
    let saved = credentials(tokens, Some("saved-id"), 1234).unwrap();
    assert_eq!(saved.expires_at, 4834);
}

async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}"), task)
}

fn fixture() -> (EncodingKey, JwkSet) {
    let fixture: Value = serde_json::from_str(include_str!("test-rsa-key.json")).unwrap();
    let der = URL_SAFE_NO_PAD
        .decode(fixture["private_key_der"].as_str().unwrap())
        .unwrap();
    (
        EncodingKey::from_rsa_der(&der),
        serde_json::from_value(fixture["jwks"].clone()).unwrap(),
    )
}

fn claims() -> Value {
    json!({"iss":oauth::ISSUER,"aud":"oaiapp_rig_test","sub":"test-account", "nonce":"test-nonce","email":"test@example.com","exp":unix_time().unwrap()+3600})
}

fn signed(claims: &Value, key: &EncodingKey) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("rig-test-key".into());
    encode(&header, claims, key).unwrap()
}

#[test]
fn validates_identity_signature_issuer_audience_expiry_nonce_and_account() {
    let (key, keys) = fixture();
    let claims = claims();
    let token = signed(&claims, &key);
    let check = |token: &str, nonce, subject| {
        oauth::verify_identity(&keys, token, "oaiapp_rig_test", nonce, subject)
    };
    let identity = check(&token, Some("test-nonce"), Some("test-account")).unwrap();
    assert_eq!(identity.email.as_deref(), Some("test@example.com"));
    assert!(check(&token, Some("wrong"), None).is_err());
    assert!(check(&token, None, Some("wrong-account")).is_err());
    for (field, invalid) in [
        ("iss", json!("wrong-issuer")),
        ("aud", json!("other-client")),
        ("exp", json!(1)),
        ("sub", json!("")),
    ] {
        let mut changed = claims.clone();
        changed[field] = invalid;
        assert!(
            check(&signed(&changed, &key), Some("test-nonce"), None).is_err(),
            "accepted invalid {field}"
        );
    }
    let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
    let mut forged = claims;
    forged["sub"] = json!("forged-account");
    parts[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
    assert!(check(&parts.join("."), Some("test-nonce"), None).is_err());
}

#[test]
fn authorization_uses_pkce_and_distinguishes_registration_from_reauthorization() {
    let redirect = "http://127.0.0.1:1234/auth/callback";
    let initial = PendingLogin::new(redirect.into(), None).unwrap();
    let url = initial.authorize_url("host-identity", None);
    let query: HashMap<_, _> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    assert_eq!(query["client_id"], "dynamic_agent_client");
    assert_eq!(query["agent_name_hint"], "Rig");
    assert_eq!(query["redirect_uri"], redirect);
    assert_eq!(query["resource"], RESOURCE);
    assert_eq!(query["ext_agent_host_id"], "host-identity");
    assert_eq!(
        query["code_challenge"],
        URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(&initial.verifier))
    );
    let returning = PendingLogin::new(redirect.into(), Some("oaiapp_saved".into())).unwrap();
    let url = returning.authorize_url("host-identity", Some("test@example.com"));
    assert!(!url.query_pairs().any(|(key, _)| key == "agent_name_hint"));
    assert!(
        url.query_pairs()
            .any(|(key, value)| key == "client_id" && value == "oaiapp_saved")
    );
    assert_ne!(initial.state, returning.state);
    assert_ne!(initial.nonce, returning.nonce);
    assert_ne!(initial.verifier, returning.verifier);
}

#[test]
fn callback_requires_matching_state_and_issued_client() {
    let pending = PendingLogin::new("http://127.0.0.1:1234/auth/callback".into(), None).unwrap();
    let callback = |state: &str, client_id: Option<&str>, error: Option<&str>| oauth::Callback {
        state: Some(state.into()),
        client_id: client_id.map(str::to_owned),
        error: error.map(str::to_owned),
        code: Some("one-use-code".into()),
    };
    assert!(
        pending
            .accept(callback("wrong-state", Some("oaiapp_test"), None))
            .is_err()
    );
    assert!(
        pending
            .accept(callback(&pending.state, None, None))
            .is_err()
    );
    assert!(
        pending
            .accept(callback(&pending.state, Some("dynamic_agent_client"), None))
            .is_err()
    );
    assert!(
        pending
            .accept(callback(
                &pending.state,
                Some("oaiapp_test"),
                Some("access_denied")
            ))
            .is_err()
    );
    assert_eq!(
        pending
            .accept(callback(&pending.state, Some("oaiapp_test"), None))
            .unwrap(),
        ("oaiapp_test".into(), "one-use-code".into())
    );
    let pending = PendingLogin::new(pending.redirect_uri, Some("oaiapp_saved".into())).unwrap();
    assert!(
        pending
            .accept(callback(&pending.state, Some("oaiapp_other"), None))
            .is_err()
    );
    assert_eq!(
        pending
            .accept(callback(&pending.state, None, None))
            .unwrap()
            .0,
        "oaiapp_saved"
    );
}

#[tokio::test]
async fn callback_completes_browser_response_and_releases_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let callback = oauth::receive_callback(TrackedListener {
        inner: listener,
        dropped: dropped.clone(),
    });
    let browser = async {
        let response = Client::new()
            .get(format!(
                "http://{address}/auth/callback?state=test&code=code&client_id=oaiapp_test"
            ))
            .send()
            .await
            .unwrap();
        assert!(response.text().await.unwrap().contains("return to Rig"));
    };
    let (callback, ()) = tokio::join!(callback, browser);
    assert_eq!(callback.unwrap().state.as_deref(), Some("test"));
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancelling_callback_releases_listener_without_a_background_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let callback = tokio::spawn(oauth::receive_callback(TrackedListener {
        inner: listener,
        dropped: dropped.clone(),
    }));
    tokio::task::yield_now().await;
    callback.abort();
    assert!(callback.await.err().unwrap().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn token_exchange_checks_plan_scope_and_never_displays_response_tokens() {
    let router = Router::new()
        .route(
            "/no-plan",
            post(|| async { Json(token_response("openid email")) }),
        )
        .route(
            "/failure",
            post(|| async {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"access_token":"secret-must-not-leak"})),
                )
            }),
        );
    let (url, task) = server(router).await;
    let client = Client::new();
    assert!(
        request_token(&client, &format!("{url}/no-plan"), &[])
            .await
            .err()
            .unwrap()
            .contains("not authorized")
    );
    let error = request_token(&client, &format!("{url}/failure"), &[])
        .await
        .err()
        .unwrap();
    assert!(error.contains("400"));
    assert!(!error.contains("secret-must-not-leak"));
    task.abort();
}

#[tokio::test]
async fn code_exchange_validates_identity_before_committing_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let auth = auth(&directory);
    let _lock = auth.storage.lock().unwrap();
    let mut state = state();
    let registration = state.registration.as_mut().unwrap();
    registration.subject = None;
    registration.email = None;
    registration.credentials = None;
    auth.storage.save(&state).unwrap();
    let pending = PendingLogin::new("http://127.0.0.1:1234/auth/callback".into(), None).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let discovery = json!({"issuer":oauth::ISSUER,"jwks_uri":format!("{url}/keys"),"revocation_endpoint":format!("{url}/revoke")});
    let (key, keys) = fixture();
    let mut claims = claims();
    claims["nonce"] = json!(pending.nonce);
    let mut tokens = token_response(oauth::DIRECT_SCOPE);
    tokens["id_token"] = json!(signed(&claims, &key));
    let (sent, received) = oneshot::channel();
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Some(sent)));
    let router = Router::new()
        .route(
            "/token",
            post(move |Form(form): Form<HashMap<String, String>>| {
                let sent = sent.clone();
                let tokens = tokens.clone();
                async move {
                    sent.lock().unwrap().take().unwrap().send(form).unwrap();
                    Json(tokens)
                }
            }),
        )
        .route(
            "/discovery",
            get(move || {
                let discovery = discovery.clone();
                async move { Json(discovery) }
            }),
        )
        .route(
            "/keys",
            get(move || {
                let keys = keys.clone();
                async move { Json(keys) }
            }),
        );
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    assert_eq!(
        auth.exchange(
            &mut state,
            &pending,
            "one-use-code",
            &format!("{url}/token"),
            &format!("{url}/discovery")
        )
        .await
        .unwrap(),
        "test@example.com"
    );
    let form = received.await.unwrap();
    assert_eq!(form["client_id"], "oaiapp_rig_test");
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "one-use-code");
    assert_eq!(form["code_verifier"], pending.verifier);
    assert_eq!(form["redirect_uri"], pending.redirect_uri);
    assert_eq!(form["resource"], RESOURCE);
    assert!(!form.contains_key("client_secret"));
    let saved = auth.storage.load().unwrap().unwrap().registration.unwrap();
    assert_eq!(saved.subject.as_deref(), Some("test-account"));
    assert_eq!(saved.credentials.unwrap().refresh_token, "new-refresh");
    task.abort();
}

#[tokio::test]
async fn authorization_completes_while_browser_launcher_is_still_running() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let browser = Command::new("sleep")
        .arg("60")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let login = receive_authorization(listener, browser);
    let callback = async {
        Client::new()
            .get(format!(
                "http://{address}/auth/callback?state=test&code=test"
            ))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(login, callback)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().code.as_deref(), Some("test"));
}

#[tokio::test]
async fn refresh_posts_registered_client_and_rotates_credentials_together() {
    let (sent, received) = oneshot::channel();
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Some(sent)));
    let router = Router::new().route(
        "/token",
        post(move |Form(form): Form<HashMap<String, String>>| {
            let sent = sent.clone();
            async move {
                sent.lock().unwrap().take().unwrap().send(form).unwrap();
                Json(token_response(oauth::DIRECT_SCOPE))
            }
        }),
    );
    let (url, task) = server(router).await;
    let directory = tempfile::tempdir().unwrap();
    let auth = auth(&directory);
    let _lock = auth.storage.lock().unwrap();
    let mut state = state();
    auth.storage.save(&state).unwrap();
    assert_eq!(
        auth.refresh(&mut state, &format!("{url}/token"), "unused")
            .await
            .unwrap(),
        "new-access"
    );
    let form = received.await.unwrap();
    assert_eq!(form["client_id"], "oaiapp_rig_test");
    assert_eq!(form["refresh_token"], "old-refresh");
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["resource"], RESOURCE);
    assert!(!form.contains_key("scope"));
    let saved = auth
        .storage
        .load()
        .unwrap()
        .unwrap()
        .registration
        .unwrap()
        .credentials
        .unwrap();
    assert_eq!(saved.access_token, "new-access");
    assert_eq!(saved.refresh_token, "new-refresh");
    assert_eq!(saved.id_token, "old-id");
    assert!(saved.expires_at > unix_time().unwrap());
    task.abort();
}

#[tokio::test]
async fn logout_clears_tokens_after_revocation_and_preserves_registration() {
    let router = Router::new().route(
        "/revoke",
        post(|Form(form): Form<HashMap<String, String>>| async move {
            assert_eq!(form["token"], "old-refresh");
            assert_eq!(form["token_type_hint"], "refresh_token");
            assert_eq!(form["client_id"], "oaiapp_rig_test");
            StatusCode::OK
        }),
    );
    let (url, task) = server(router).await;
    let directory = tempfile::tempdir().unwrap();
    let auth = auth(&directory);
    let _lock = auth.storage.lock().unwrap();
    let mut state = state();
    auth.storage.save(&state).unwrap();
    auth.revoke(&mut state, &format!("{url}/revoke"))
        .await
        .unwrap();
    let saved = auth.storage.load().unwrap().unwrap();
    assert_eq!(saved.host_id, "urn:uuid:test-host");
    let registration = saved.registration.unwrap();
    assert_eq!(registration.client_id, "oaiapp_rig_test");
    assert_eq!(registration.subject.as_deref(), Some("test-account"));
    assert!(registration.credentials.is_none());
    task.abort();
}

#[tokio::test]
async fn refresh_and_logout_failures_retain_saved_credentials_without_retry() {
    let router = Router::new().route("/failure", post(|| async { StatusCode::BAD_REQUEST }));
    let (url, task) = server(router).await;
    let directory = tempfile::tempdir().unwrap();
    let auth = auth(&directory);
    let _lock = auth.storage.lock().unwrap();
    let mut state = state();
    auth.storage.save(&state).unwrap();
    assert!(
        auth.refresh(&mut state, &format!("{url}/failure"), "unused")
            .await
            .is_err()
    );
    assert!(
        auth.revoke(&mut state, &format!("{url}/failure"))
            .await
            .is_err()
    );
    let saved = auth
        .storage
        .load()
        .unwrap()
        .unwrap()
        .registration
        .unwrap()
        .credentials
        .unwrap();
    assert_eq!(saved.access_token, "old-access");
    assert_eq!(saved.refresh_token, "old-refresh");
    task.abort();
}

#[tokio::test]
async fn unexpired_access_token_needs_no_network_and_signed_out_state_fails() {
    let directory = tempfile::tempdir().unwrap();
    let auth = auth(&directory);
    {
        let _lock = auth.storage.lock().unwrap();
        let mut state = state();
        state
            .registration
            .as_mut()
            .unwrap()
            .credentials
            .as_mut()
            .unwrap()
            .expires_at = unix_time().unwrap() + 3600;
        auth.storage.save(&state).unwrap();
    }
    assert_eq!(auth.access_token().await.unwrap(), "old-access");
    {
        let _lock = auth.storage.lock().unwrap();
        let mut state = auth.storage.load().unwrap().unwrap();
        state.registration.as_mut().unwrap().credentials = None;
        auth.storage.save(&state).unwrap();
    }
    assert!(auth.access_token().await.unwrap_err().contains("rig login"));
}
