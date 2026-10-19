use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

static WAKE_LOCK_HELD: AtomicBool = AtomicBool::new(false);

pub fn is_termux() -> bool {
    std::env::var("PREFIX")
        .map(|p| p.contains("com.termux"))
        .unwrap_or(false)
        || std::path::Path::new("/data/data/com.termux/files/usr").exists()
}

pub fn has_termux_api() -> bool {
    is_termux()
        && Command::new("termux-wake-lock")
            .arg("--help")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
}

pub fn acquire_wake_lock() {
    if !is_termux() {
        return;
    }
    match Command::new("termux-wake-lock").status() {
        Ok(s) if s.success() => {
            WAKE_LOCK_HELD.store(true, Ordering::Relaxed);
            eprintln!("[*] Termux wake lock acquired");
        }
        _ => eprintln!("[!] termux-wake-lock failed (pkg install termux-api)"),
    }
}

pub fn release_wake_lock() {
    if !WAKE_LOCK_HELD.load(Ordering::Relaxed) {
        return;
    }
    let _ = Command::new("termux-wake-unlock").status();
    WAKE_LOCK_HELD.store(false, Ordering::Relaxed);
}

pub fn notify(title: &str, content: &str) {
    if !is_termux() {
        return;
    }
    let _ = Command::new("termux-notification")
        .args([
            "--id",
            "stalinium-server",
            "--title",
            title,
            "--content",
            content,
            "--ongoing",
            "--priority",
            "low",
            "--icon",
            "wifi_tethering",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn clear_notification() {
    if !is_termux() {
        return;
    }
    let _ = Command::new("termux-notification-remove")
        .arg("stalinium-server")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct BatteryInfo {
    pub percentage: u32,
    pub plugged: bool,
    pub temperature: f64,
}

pub fn battery_status() -> Option<BatteryInfo> {
    if !is_termux() {
        return None;
    }
    let output = Command::new("termux-battery-status")
        .output()
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    Some(BatteryInfo {
        percentage: json["percentage"].as_u64().unwrap_or(0) as u32,
        plugged: json["plugged"].as_str().unwrap_or("UNPLUGGED") != "UNPLUGGED",
        temperature: json["temperature"].as_f64().unwrap_or(0.0),
    })
}

pub fn local_ip() -> Option<String> {
    if !is_termux() {
        return None;
    }
    let output = Command::new("termux-wifi-connectioninfo")
        .output()
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json["ip"].as_str().map(|s| s.to_string())
}

pub fn data_dir() -> String {
    if is_termux() {
        let home = std::env::var("HOME")
            .unwrap_or_else(|_| "/data/data/com.termux/files/home".to_string());
        let dir = format!("{}/.stalinium", home);
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        ".".to_string()
    }
}

pub fn warn_low_ports(addr: &str, name: &str) {
    if !is_termux() {
        return;
    }
    if let Some(port_str) = addr.rsplit(':').next() {
        if let Ok(port) = port_str.parse::<u16>() {
            if port < 1024 {
                eprintln!(
                    "[!] {} port {} requires root -- Termux has no root. Use port >= 1024.",
                    name, port
                );
            }
        }
    }
}
