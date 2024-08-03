use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Command {
    pub id: u32,
    pub command: String,
    pub args: Vec<String>,
}
