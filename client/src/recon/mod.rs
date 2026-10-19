pub mod policy;

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crusade_core::{Cluster, Host, Identifier, LinkKind, Linker, Service};
use crusade_enrich::{enrich_host, EnrichConfig, GeoAsnDb};
use crusade_probe::{probe_service, Fingerprints, ProbeConfig};
use crusade_scan::{Blocklist, RateLimiterParams, ScanConfig, TcpScanner};

use crate::error::C2Error;
use self::policy::AdaptiveScanPolicy;

pub struct ReconConfig {
    pub ports: Vec<u16>,
    pub connect_timeout_ms: u64,
    pub probe_timeout_ms: u64,
    pub rate_per_sec: f64,
    pub concurrency: usize,
    pub allow_private: bool,
    pub honeypot_threshold: f32,
    pub enable_enrichment: bool,
    pub nmap_probes_path: Option<String>,
    pub geoasn_v4_path: Option<String>,
    pub geoasn_v6_path: Option<String>,
    pub adaptive_timing: bool,
}

impl Default for ReconConfig {
    fn default() -> Self {
        ReconConfig {
            ports: vec![21, 22, 23, 25, 53, 80, 110, 143, 443, 445, 993, 995, 3306, 3389, 5432, 8080, 8443],
            connect_timeout_ms: 3000,
            probe_timeout_ms: 6000,
            rate_per_sec: 100.0,
            concurrency: 50,
            allow_private: false,
            honeypot_threshold: 0.6,
            enable_enrichment: true,
            nmap_probes_path: None,
            geoasn_v4_path: None,
            geoasn_v6_path: None,
            adaptive_timing: true,
        }
    }
}

pub struct ReconEngine {
    config: ReconConfig,
    blocklist: Blocklist,
    fingerprints: Option<Arc<Fingerprints>>,
    geo_db: GeoAsnDb,
    policy: AdaptiveScanPolicy,
}

impl ReconEngine {
    pub fn new(config: ReconConfig) -> Self {
        let blocklist = Blocklist::new(config.allow_private);

        let fingerprints = config.nmap_probes_path.as_deref()
            .and_then(|p| Fingerprints::load(Path::new(p)).ok())
            .or_else(|| Fingerprints::load_default().ok())
            .map(Arc::new);

        let geo_db = match (&config.geoasn_v4_path, &config.geoasn_v6_path) {
            (Some(v4), v6) => {
                GeoAsnDb::load(Path::new(v4), v6.as_deref().map(Path::new)).unwrap_or_default()
            }
            _ => GeoAsnDb::default(),
        };

        let policy = AdaptiveScanPolicy::new(config.adaptive_timing);

        ReconEngine {
            config,
            blocklist,
            fingerprints,
            geo_db,
            policy,
        }
    }

    pub async fn scan_targets(&self, targets: &[IpAddr]) -> Result<Vec<(IpAddr, u16)>, C2Error> {
        let scan_config = ScanConfig {
            ports: self.config.ports.clone(),
            connect_timeout: Duration::from_millis(self.config.connect_timeout_ms),
            rate: RateLimiterParams {
                conn_per_sec: self.policy.effective_rate(self.config.rate_per_sec),
                burst: self.policy.effective_rate(self.config.rate_per_sec),
                concurrency: self.config.concurrency,
            },
            allow_private: self.config.allow_private,
        };

        let scanner = TcpScanner::new(scan_config, self.blocklist.clone());
        let open = scanner.scan(targets).await;
        Ok(open)
    }

    pub async fn probe_open_ports(&self, open: &[(IpAddr, u16)]) -> Vec<Service> {
        let fp = match &self.fingerprints {
            Some(fp) => fp.clone(),
            None => return Vec::new(),
        };

        let probe_cfg = ProbeConfig {
            timeout: Duration::from_millis(self.config.probe_timeout_ms),
            tls: true,
            sni: None,
        };

        let mut services = Vec::with_capacity(open.len());
        for &(ip, port) in open {
            if self.config.adaptive_timing {
                self.policy.delay().await;
            }
            let svc = probe_service(ip, port, &fp, &probe_cfg).await;
            services.push(svc);
        }
        services
    }

    pub async fn build_hosts(&self, scan_results: &[(IpAddr, u16)], services: Vec<Service>) -> Vec<Host> {
        let mut host_map: std::collections::HashMap<IpAddr, Host> = std::collections::HashMap::new();

        let mut svc_iter = services.into_iter();
        for &(ip, _port) in scan_results {
            let host = host_map.entry(ip).or_insert_with(|| Host::new(ip));
            if let Some(svc) = svc_iter.next() {
                host.services.push(svc);
            }
        }

        let enrich_cfg = EnrichConfig {
            geo: self.config.enable_enrichment,
            reverse_dns: self.config.enable_enrichment,
        };

        let mut hosts: Vec<Host> = host_map.into_values().collect();
        if self.config.enable_enrichment {
            for host in &mut hosts {
                enrich_host(host, &self.geo_db, &enrich_cfg).await;
            }
        }

        hosts
    }

    pub fn cluster_hosts(&self, hosts: &[Host]) -> Vec<Cluster> {
        let mut linker = Linker::new();

        for host in hosts {
            let ip_str = host.ip.to_string();
            let mut identifiers = Vec::new();

            for svc in &host.services {
                if let Some(tls) = &svc.tls {
                    if let Some(fp) = &tls.fingerprint_sha256 {
                        identifiers.push(Identifier::new(LinkKind::Certificate, fp.as_str()));
                    }
                }
                if let Some(ssh) = &svc.ssh {
                    if let Some(key) = &ssh.host_key_sha256 {
                        identifiers.push(Identifier::new(LinkKind::SshHostKey, key.as_str()));
                    }
                    if let Some(hassh) = &ssh.hassh {
                        identifiers.push(Identifier::new(LinkKind::Hassh, hassh.as_str()));
                    }
                }
                if let Some(http) = &svc.http {
                    if let Some(fav) = http.favicon_hash {
                        identifiers.push(Identifier::new(LinkKind::Favicon, fav.to_string()));
                    }
                }
                if let Some(jarm) = &svc.jarm {
                    identifiers.push(Identifier::new(LinkKind::Jarm, jarm.as_str()));
                }
            }

            if !identifiers.is_empty() {
                linker.add(ip_str, identifiers);
            }
        }

        linker.clusters()
    }

    pub fn filter_honeypots(&self, hosts: Vec<Host>) -> (Vec<Host>, Vec<Host>) {
        let mut clean = Vec::new();
        let mut flagged = Vec::new();

        for host in hosts {
            let is_honeypot = host.honeypot.as_ref()
                .map(|h| h.score >= self.config.honeypot_threshold)
                .unwrap_or(false);

            let has_latency_anomaly = host.services.iter().any(|svc| {
                svc.latency.as_ref().and_then(|l| l.jitter()).map(|j| j > 0.8).unwrap_or(false)
                    || svc.latency.as_ref().map(|l| l.mean_ratio > 50.0).unwrap_or(false)
            });

            if is_honeypot || has_latency_anomaly {
                flagged.push(host);
            } else {
                clean.push(host);
            }
        }

        (clean, flagged)
    }

    pub async fn full_recon(&self, target_specs: &[String]) -> Result<ReconResult, C2Error> {
        let mut all_targets: Vec<IpAddr> = Vec::new();
        for spec in target_specs {
            match crusade_scan::expand_targets(spec, &self.blocklist) {
                Ok(addrs) => all_targets.extend(addrs),
                Err(e) => {
                    eprintln!("[!] target expansion failed for {}: {}", spec, e);
                }
            }
        }

        if all_targets.is_empty() {
            return Err(C2Error::CommandError("no valid targets after expansion".to_string()));
        }

        let open = self.scan_targets(&all_targets).await?;
        let services = self.probe_open_ports(&open).await;
        let hosts = self.build_hosts(&open, services).await;
        let clusters = self.cluster_hosts(&hosts);
        let (clean_hosts, honeypot_hosts) = self.filter_honeypots(hosts);

        Ok(ReconResult {
            total_targets: all_targets.len(),
            open_ports: open.len(),
            hosts: clean_hosts,
            honeypots: honeypot_hosts,
            clusters: clusters_to_summaries(clusters),
        })
    }

    pub fn expand_targets(&self, specs: &[String]) -> Result<Vec<IpAddr>, C2Error> {
        let mut out = Vec::new();
        for spec in specs {
            match crusade_scan::expand_targets(spec, &self.blocklist) {
                Ok(addrs) => out.extend(addrs),
                Err(e) => return Err(C2Error::CommandError(format!("target expansion: {}", e))),
            }
        }
        Ok(out)
    }
}

#[derive(serde::Serialize)]
pub struct ReconResult {
    pub total_targets: usize,
    pub open_ports: usize,
    pub hosts: Vec<Host>,
    pub honeypots: Vec<Host>,
    pub clusters: Vec<ClusterSummary>,
}

#[derive(serde::Serialize)]
pub struct ClusterSummary {
    pub id: String,
    pub members: Vec<String>,
    pub size: usize,
    pub evidence: Vec<String>,
}

impl From<Cluster> for ClusterSummary {
    fn from(c: Cluster) -> Self {
        let evidence = c.evidence.iter()
            .map(|(id, count)| format!("{}:{} ({}x)", id.kind.as_str(), &id.value[..id.value.len().min(16)], count))
            .collect();
        ClusterSummary {
            id: c.id.clone(),
            size: c.len(),
            members: c.members,
            evidence,
        }
    }
}

fn clusters_to_summaries(clusters: Vec<Cluster>) -> Vec<ClusterSummary> {
    clusters.into_iter().map(ClusterSummary::from).collect()
}
