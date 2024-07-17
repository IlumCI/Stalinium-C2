use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Command {
    pub id: usize,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CommandResult {
    pub id: usize,
    pub result: String,
}
