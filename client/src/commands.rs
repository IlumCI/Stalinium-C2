use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Command {
    pub id: usize,
    pub command: String,
    pub args: Vec<String>,
}
