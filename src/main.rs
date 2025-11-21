mod database;
mod frontend;

use axum::{
    extract::{Form, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{delete, get},
    Json, Router,
};
use craftping::tokio::ping;
use database::{AdminUser, Database, PingResult};
use frontend::INDEX_HTML;
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};
use tokio::signal;
use std::env;

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::{rngs::OsRng, RngCore};

#[derive(Clone)]
struct AppState {
    db: Database,
}

#[derive(Debug, Deserialize)]
struct CreateServerJson {
    name: String,
    address: String,
    port: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AuthMeResponse {
    #[serde(rename = "isAdmin")]
    is_admin: bool,
}

#[derive(Debug, Serialize)]
struct SimpleResponse {
    success: bool,
}

#[derive(Debug, Serialize)]
struct ServerApi {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub port: i64,
    pub created_at: String,
    pub last_online: bool,
}

#[tokio::main]
async fn main() {
    // 1. Initialize Database
    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://sqlite.db".to_string());
    let db = Database::init(&db_url)
        .await
        .expect("failed to initialize database");

    // 2. Create default admin
    init_default_admin(&db).await;

    // Keep a clone of DB handle to close it after server stops
    let db_for_shutdown = db.clone();

    let state = AppState { db };

    // 3. Background Task
    let bg_state = state.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = ping_all_servers_concurrently(&bg_state).await {
                eprintln!("background ping dispatcher error: {:?}", e);
            }
            sleep(Duration::from_secs(600)).await;
        }
    });

    // 4. Router
    let app = Router::new()
        .route("/", get(index))
        .route("/db", get(temp_db_check))
        .route("/api/auth/me", get(auth_me))
        .route("/admin/login", get(show_login_form).post(handle_login))
        .route("/logout", get(handle_logout))
        .route("/servers", get(list_servers).post(create_server_json))
        .route("/servers/{id}", delete(delete_server))
        .route("/servers/{id}/ping", get(ping_and_store).post(ping_and_store))
        .route("/servers/{id}/pings", get(list_server_ping_history))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("failed to bind address");

    println!("Server running on http://0.0.0.0:3000");
    println!("Press Ctrl+C to stop.");

    // 5. Start Server with Graceful Shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // 6. Clean up
    println!("Server stopped. Closing database connection...");
    db_for_shutdown.close().await;
    println!("Database closed. Goodbye!");
}





/// Listens for Ctrl+C or SIGTERM
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn index() -> Html<String> {
    Html(INDEX_HTML.to_owned())
}

async fn temp_db_check(State(state): State<AppState>) -> String {
    match state.db.test_query().await {
        Ok(msg) => msg,
        Err(e) => format!("db error: {:?}", e),
    }
}

async fn init_default_admin(db: &Database) {
    let default_username = "admin";
    let default_password = env::var("ADMIN_PASSWORD")
        .unwrap_or_else(|_| "change_me".to_string());

    if let Ok(Some(_)) = db.get_admin_by_username(default_username).await {
        return;
    }

    let hash = hash_password(&default_password);
    if let Err(e) = db.ensure_admin_user(default_username, &hash).await {
        eprintln!("failed to create default admin: {:?}", e);
    } else {
        println!("--------------------------------------------------");
        println!("Admin initialized.");
        println!("Username: {}", default_username);
        if default_password == "change_me" {
            println!("WARNING: Using unsafe default password 'change_me'.");
            println!("Set ADMIN_PASSWORD environment variable in production.");
        } else {
            println!("Password set from environment variable.");
        }
        println!("--------------------------------------------------");
    }
}

fn hash_password(password: &str) -> String {
    let mut salt_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut salt_bytes);
    let salt = SaltString::encode_b64(&salt_bytes).expect("salt b64 error");

    let argon2 = Argon2::default();
    argon2.hash_password(password.as_bytes(), &salt).expect("hash error").to_string()
}

fn verify_password(hash: &str, password: &str) -> bool {
    if let Ok(parsed) = PasswordHash::new(hash) {
        Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
    } else {
        false
    }
}

fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn get_session_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;
    for part in cookie_str.split(';') {
        let trimmed = part.trim();
        if let Some(rest) = trimmed.strip_prefix("admin_session=") {
            return Some(rest.to_string());
        }
    }
    None
}

async fn get_admin_from_headers(state: &AppState, headers: &HeaderMap) -> Result<AdminUser, StatusCode> {
    let token = get_session_token_from_headers(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let admin = state.db.get_admin_by_session_token(&token).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    admin.ok_or(StatusCode::UNAUTHORIZED)
}

async fn auth_me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<AuthMeResponse>, StatusCode> {
    let token = match get_session_token_from_headers(&headers) {
        Some(t) => t,
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    let admin = state.db.get_admin_by_session_token(&token).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if admin.is_some() {
        Ok(Json(AuthMeResponse { is_admin: true }))
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn show_login_form() -> Html<String> {
    let html = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"/><title>Admin Login</title><meta name="viewport" content="width=device-width, initial-scale=1"/><style>body{margin:0;padding:0;background:#0a0a0a;color:#fafafa;font-family:system-ui,sans-serif;display:flex;align-items:center;justify-content:center;min-height:100vh}.card{background:#171717;border:1px solid #262626;padding:1.5rem;border-radius:8px;width:320px}h1{margin-top:0;margin-bottom:1rem;font-size:1.1rem;letter-spacing:0.08em;text-transform:uppercase;color:#a3a3a3}label{display:block;font-size:0.8rem;color:#a3a3a3;margin-bottom:0.3rem}input{width:100%;padding:0.5rem;border-radius:4px;border:1px solid #262626;background:#0a0a0a;color:#fafafa;margin-bottom:0.8rem}input:focus{outline:none;border-color:#84cc16}button{width:100%;padding:0.6rem;border-radius:4px;border:none;background:#84cc16;color:#000;font-weight:700;text-transform:uppercase;font-size:0.8rem;cursor:pointer}.error{color:#f97373;font-size:0.8rem;margin-bottom:0.8rem}a{display:inline-block;margin-top:0.6rem;font-size:0.8rem;color:#a3a3a3;text-decoration:none}</style></head><body><div class="card"><h1>Admin Login</h1><form method="post" action="/admin/login"><div id="error-msg" class="error" style="display:none;">Invalid credentials.</div><label for="username">Username</label><input name="username" id="username" autocomplete="username"/><label for="password">Password</label><input name="password" id="password" type="password" autocomplete="current-password"/><button type="submit">Sign in</button></form><a href="/">Back to dashboard</a></div><script>const params=new URLSearchParams(window.location.search);if(params.get("error")==="1"){document.getElementById("error-msg").style.display="block";}</script></body></html>"#;
    Html(html.to_string())
}

async fn handle_login(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let maybe_admin = state.db.get_admin_by_username(&form.username).await.ok().flatten();
    if let Some(admin) = maybe_admin {
        if verify_password(&admin.password_hash, &form.password) {
            let token = generate_session_token();
            if let Err(e) = state.db.create_admin_session(admin.id, &token).await {
                eprintln!("Failed to create session: {:?}", e);
                return Redirect::to("/admin/login?error=1").into_response();
            }
            let mut headers = HeaderMap::new();
            let is_prod = env::var("APP_ENV").unwrap_or_default() == "production";
            let secure_flag = if is_prod { "; Secure" } else { "" };
            let cookie_value = format!("admin_session={}; HttpOnly; SameSite=Strict; Path=/{}{}", token, secure_flag, "");
            headers.insert(header::SET_COOKIE, header::HeaderValue::from_str(&cookie_value).unwrap());
            return (headers, Redirect::to("/")).into_response();
        }
    }
    sleep(Duration::from_secs(2)).await;
    Redirect::to("/admin/login?error=1").into_response()
}

async fn handle_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = get_session_token_from_headers(&headers) {
        let _ = state.db.delete_session(&token).await;
    }
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::SET_COOKIE, header::HeaderValue::from_static("admin_session=deleted; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"));
    (resp_headers, Redirect::to("/"))
}

async fn list_servers(State(state): State<AppState>) -> Result<Json<Vec<ServerApi>>, StatusCode> {
    let servers = state.db.list_servers().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut result = Vec::new();
    for s in servers {
        let last_ping = state.db.get_last_ping_for_server(s.id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let last_online = last_ping.map(|p| p.online).unwrap_or(false);
        result.push(ServerApi { id: s.id, name: s.name, address: s.address, port: s.port, created_at: s.created_at, last_online });
    }
    Ok(Json(result))
}

async fn create_server_json(State(state): State<AppState>, headers: HeaderMap, Json(body): Json<CreateServerJson>) -> Result<Json<ServerApi>, StatusCode> {
    let _admin = get_admin_from_headers(&state, &headers).await?;
    let port = body.port.unwrap_or(25565);
    if port < 1 || port > 65535 { return Err(StatusCode::BAD_REQUEST); }
    if body.address.trim().is_empty() || body.name.trim().is_empty() { return Err(StatusCode::BAD_REQUEST); }
    let id = state.db.insert_server(&body.name, &body.address, port).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let server = state.db.get_server_by_id(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?.ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ServerApi { id: server.id, name: server.name, address: server.address, port: server.port, created_at: server.created_at, last_online: false }))
}

async fn delete_server(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Result<Json<SimpleResponse>, StatusCode> {
    let _admin = get_admin_from_headers(&state, &headers).await?;
    state.db.delete_server(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SimpleResponse { success: true }))
}

async fn ping_one_server(state: &AppState, server_id: i64) -> Result<(), ()> {
    let maybe_server = state.db.get_server_by_id(server_id).await.map_err(|e| eprintln!("db error in ping: {:?}", e))?;
    let Some(server) = maybe_server else { return Ok(()); };
    let host = server.address.clone();
    let port_u16 = server.port as u16;
    let mut stream = match TcpStream::connect((host.as_str(), port_u16)).await {
        Ok(s) => s,
        Err(_) => { let _ = state.db.insert_ping_result(server.id, false, None, None, None, None, None).await; return Ok(()); }
    };
    let resp = match ping(&mut stream, host.as_str(), port_u16).await {
        Ok(r) => r,
        Err(_) => { let _ = state.db.insert_ping_result(server.id, false, None, None, None, None, None).await; return Ok(()); }
    };
    let online_players = resp.online_players as i64;
    let max_players = resp.max_players as i64;
    let version = resp.version.clone();
    let desc_str = resp.description.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "No description".to_string());
    let _ = state.db.insert_ping_result(server.id, true, None, Some(online_players), Some(max_players), Some(version.as_str()), Some(desc_str.as_str())).await;
    Ok(())
}

async fn ping_all_servers_concurrently(state: &AppState) -> Result<(), ()> {
    let servers = state.db.list_servers().await.map_err(|e| eprintln!("list_servers error: {:?}", e))?;
    for s in servers {
        let state_clone = state.clone();
        tokio::spawn(async move { let _ = ping_one_server(&state_clone, s.id).await; });
    }
    Ok(())
}

async fn ping_and_store(State(state): State<AppState>, Path(server_id): Path<i64>) -> Result<Json<SimpleResponse>, StatusCode> {
    ping_one_server(&state, server_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SimpleResponse { success: true }))
}

async fn list_server_ping_history(State(state): State<AppState>, Path(id): Path<i64>) -> Result<Json<Vec<PingResult>>, StatusCode> {
    let mut results = state.db.list_ping_results_for_server(id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    results.reverse();
    Ok(Json(results))
}