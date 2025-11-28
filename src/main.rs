mod database;

use axum::{
    Json, Router,
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use craftping::tokio::ping;
use database::{AdminUser, Database, PingResult};
use serde::{Deserialize, Serialize};
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{Duration, sleep};
use tower_http::services::ServeDir; // Ensure tower-http is in Cargo.toml with features=["fs"]

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::{RngCore, rngs::OsRng};

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

    let db_for_shutdown = db.clone();
    let state = AppState { db };

    // 3. Background Task
    let bg_state = state.clone();
    tokio::spawn(async move {
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let interval = 600; // Ten minutes 600 seconds I should probably change this to be an env variable
        let seconds_past = now % interval;
        let wait = interval - seconds_past;
        sleep(Duration::from_secs(wait)).await; //  Round to the nearest interval before pinging next

        // Ping each server every ten minutes
        loop {
            if let Err(e) = ping_all_servers_concurrently(&bg_state).await {
                eprintln!("Background ping error: {:?}", e);
            }
            sleep(Duration::from_secs(interval)).await;
        }
    });

    // 4. Router
    // We put API routes under /api so they don't clash with file names
    let api_routes = Router::new()
        .route("/auth/me", get(auth_me))
        .route("/servers", get(list_servers).post(create_server_json))
        .route("/servers/{id}", delete(delete_server))
        .route(
            "/servers/{id}/ping",
            get(ping_and_store).post(ping_and_store),
        )
        .route("/servers/{id}/pings", get(list_server_ping_history))
        .with_state(state.clone());

    // Auth routes need state too
    let auth_routes = Router::new()
        .route("/login", post(handle_login))
        .route("/logout", get(handle_logout))
        .with_state(state);

    let app = Router::new()
        .nest("/api", api_routes)
        .nest("/auth", auth_routes) // Note: Login form POSTs to /auth/login now
        // This serves index.html, style.css, script.js, images/, etc automatically
        .fallback_service(ServeDir::new("static"));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("Server running on http://0.0.0.0:3000");
    let is_prod = env::var("APP_ENV").unwrap_or_default() == "production";
    if !is_prod {
        println!("Press Ctrl+C to stop.");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    db_for_shutdown.close().await;
    println!("Database closed.");
}

// --- HANDLERS ---

async fn init_default_admin(db: &Database) {
    let default_user = "admin";
    let default_pass = env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "change_me".to_string());

    if db
        .get_admin_by_username(default_user)
        .await
        .unwrap_or(None)
        .is_some()
    {
        return;
    }

    let hash = hash_password(&default_pass);
    if let Err(e) = db.ensure_admin_user(default_user, &hash).await {
        eprintln!("Failed to create default admin: {:?}", e);
    } else {
        println!("Admin created: {}", default_user);
    }
}

// POST /auth/login
async fn handle_login(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    let maybe_admin = state
        .db
        .get_admin_by_username(&form.username)
        .await
        .ok()
        .flatten();
    if let Some(admin) = maybe_admin {
        if verify_password(&admin.password_hash, &form.password) {
            let token = generate_session_token();
            if state
                .db
                .create_admin_session(admin.id, &token)
                .await
                .is_ok()
            {
                let mut headers = HeaderMap::new();
                let is_prod = env::var("APP_ENV").unwrap_or_default() == "production";
                let secure = if is_prod { "; Secure" } else { "" };
                let cookie = format!(
                    "admin_session={}; HttpOnly; SameSite=Strict; Path=/{}{}",
                    token, secure, ""
                );
                headers.insert(
                    header::SET_COOKIE,
                    header::HeaderValue::from_str(&cookie).unwrap(),
                );

                // Redirect back to home on success
                return (headers, Redirect::to("/")).into_response();
            }
        }
    }
    sleep(Duration::from_secs(2)).await;
    // Redirect to the static login page with error param
    Redirect::to("/login.html?error=1").into_response()
}

// GET /auth/logout
async fn handle_logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(token) = get_session_token_from_headers(&headers) {
        let _ = state.db.delete_session(&token).await;
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        header::HeaderValue::from_static(
            "admin_session=deleted; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
        ),
    );
    (headers, Redirect::to("/"))
}

// API Handlers (JSON)

async fn auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthMeResponse>, StatusCode> {
    let token = get_session_token_from_headers(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let admin = state
        .db
        .get_admin_by_session_token(&token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if admin.is_some() {
        Ok(Json(AuthMeResponse { is_admin: true }))
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn list_servers(State(state): State<AppState>) -> Result<Json<Vec<ServerApi>>, StatusCode> {
    let servers = state
        .db
        .list_servers()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut res = Vec::new();
    for s in servers {
        let last = state
            .db
            .get_last_ping_for_server(s.id)
            .await
            .unwrap_or(None);
        res.push(ServerApi {
            id: s.id,
            name: s.name,
            address: s.address,
            port: s.port,
            created_at: s.created_at,
            last_online: last.map(|p| p.online).unwrap_or(false),
        });
    }
    Ok(Json(res))
}

async fn create_server_json(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateServerJson>,
) -> Result<Json<ServerApi>, StatusCode> {
    let _ = get_admin_from_headers(&state, &headers).await?;
    if body.port.unwrap_or(25565) < 1 || body.name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let id = state
        .db
        .insert_server(&body.name, &body.address, body.port.unwrap_or(25565))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let s = state.db.get_server_by_id(id).await.unwrap().unwrap();

    Ok(Json(ServerApi {
        id: s.id,
        name: s.name,
        address: s.address,
        port: s.port,
        created_at: s.created_at,
        last_online: false,
    }))
}

async fn delete_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<SimpleResponse>, StatusCode> {
    let _ = get_admin_from_headers(&state, &headers).await?;
    state
        .db
        .delete_server(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SimpleResponse { success: true }))
}

async fn ping_and_store(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<SimpleResponse>, StatusCode> {
    ping_one_server(&state, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(SimpleResponse { success: true }))
}

async fn list_server_ping_history(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<PingResult>>, StatusCode> {
    let mut res = state
        .db
        .list_ping_results_for_server(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    res.reverse();
    Ok(Json(res))
}

// Utilities

async fn ping_all_servers_concurrently(state: &AppState) -> Result<(), ()> {
    let servers = state
        .db
        .list_servers()
        .await
        .map_err(|e| eprintln!("Ping list error: {:?}", e))?;
    for s in servers {
        let st = state.clone();
        tokio::spawn(async move {
            let _ = ping_one_server(&st, s.id).await;
        });
    }
    Ok(())
}

async fn ping_one_server(state: &AppState, id: i64) -> Result<(), ()> {
    let s = match state.db.get_server_by_id(id).await {
        Ok(Some(v)) => v,
        _ => return Ok(()),
    };

    let mut stream = match TcpStream::connect((s.address.as_str(), s.port as u16)).await {
        Ok(s) => s,
        Err(_) => {
            let _ = state
                .db
                .insert_ping_result(s.id, false, None, None, None, None, None)
                .await;
            return Ok(());
        }
    };

    match ping(&mut stream, s.address.as_str(), s.port as u16).await {
        Ok(r) => {
            let desc = r
                .description
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default();
            let _ = state
                .db
                .insert_ping_result(
                    s.id,
                    true,
                    None,
                    Some(r.online_players as i64),
                    Some(r.max_players as i64),
                    Some(r.version.as_str()),
                    Some(desc.as_str()),
                )
                .await;
        }
        Err(_) => {
            let _ = state
                .db
                .insert_ping_result(s.id, false, None, None, None, None, None)
                .await;
        }
    }
    Ok(())
}

// Auth Utilities
fn hash_password(p: &str) -> String {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    Argon2::default()
        .hash_password(p.as_bytes(), &SaltString::encode_b64(&salt).unwrap())
        .unwrap()
        .to_string()
}
fn verify_password(h: &str, p: &str) -> bool {
    PasswordHash::new(h)
        .map(|ph| Argon2::default().verify_password(p.as_bytes(), &ph).is_ok())
        .unwrap_or(false)
}
fn generate_session_token() -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
fn get_session_token_from_headers(h: &HeaderMap) -> Option<String> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| s.trim().strip_prefix("admin_session=").map(String::from))
}
async fn get_admin_from_headers(state: &AppState, h: &HeaderMap) -> Result<AdminUser, StatusCode> {
    let t = get_session_token_from_headers(h).ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .db
        .get_admin_by_session_token(&t)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.unwrap();
    };
    #[cfg(unix)]
    let term = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .unwrap()
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
