use std::process::Command;
use std::io::Write;

pub fn add_persistence() {
    let exe_path = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "stalinium_server".to_string());

    if crate::termux::is_termux() {
        add_termux_persistence(&exe_path);
        return;
    }

    if cfg!(target_os = "windows") {
        let status = Command::new("schtasks")
            .args(&["/create", "/sc", "onlogon", "/tn", "StaliniumServer", "/tr", &exe_path, "/f"])
            .status();
        match status {
            Ok(s) if s.success() => eprintln!("[*] Persistence added (schtasks)."),
            _ => eprintln!("[!] Failed to add persistence."),
        }
    } else if cfg!(target_os = "linux") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let svc_dir = format!("{}/.config/systemd/user", home);
        let svc_path = format!("{}/stalinium-server.service", svc_dir);
        let unit = format!(
            "[Unit]\nDescription=Stalinium Server\nAfter=network.target\n\n\
             [Service]\nExecStart={}\nRestart=always\nRestartSec=10\n\n\
             [Install]\nWantedBy=default.target\n",
            exe_path
        );

        let systemd_ok = std::fs::create_dir_all(&svc_dir).is_ok()
            && std::fs::write(&svc_path, &unit).is_ok()
            && Command::new("systemctl")
                .args(&["--user", "enable", "--now", "stalinium-server.service"])
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
        let plist_path = format!("{}/Library/LaunchAgents/com.stalinium.server.plist", home);
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>com.stalinium.server</string>\n\
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

fn add_termux_persistence(exe_path: &str) {
    let home = std::env::var("HOME")
        .unwrap_or_else(|_| "/data/data/com.termux/files/home".to_string());
    let boot_dir = format!("{}/.termux/boot", home);
    let boot_script = format!("{}/stalinium-server", boot_dir);

    let work_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| home.clone());

    let script_content = format!(
        "#!/data/data/com.termux/files/usr/bin/bash\n\
         termux-wake-lock\n\
         cd \"{work_dir}\"\n\
         nohup \"{exe_path}\" > \"$HOME/.stalinium/server.log\" 2>&1 &\n\
         termux-notification \\\n  \
           --id stalinium-boot \\\n  \
           --title \"Stalinium Server\" \\\n  \
           --content \"Server started on boot\" \\\n  \
           --priority low\n",
    );

    let dir_ok = std::fs::create_dir_all(&boot_dir).is_ok();
    let write_ok = dir_ok && std::fs::write(&boot_script, &script_content).is_ok();
    let chmod_ok = write_ok
        && Command::new("chmod")
            .args(["+x", &boot_script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if chmod_ok {
        eprintln!("[*] Persistence added (Termux:Boot).");
        eprintln!("    Script: {}", boot_script);
    } else {
        eprintln!("[!] Termux:Boot persistence failed.");
        eprintln!("    Install Termux:Boot from F-Droid for auto-start.");
    }

    let _ = std::fs::create_dir_all(format!("{}/.stalinium", home));
}
