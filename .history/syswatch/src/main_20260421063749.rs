use std::fmt;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::Local;
use sysinfo::System;

// ============================================================
// ÉTAPE 1 — Modélisation des données
// ============================================================

#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub usage: f32,
    pub core_count: usize,
}

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bar = make_bar(self.usage, 20);
        write!(
            f,
            "CPU  [{bar}] {:.1}%  ({} cœurs)",
            self.usage, self.core_count
        )
    }
}

#[derive(Debug, Clone)]
pub struct MemInfo {
    pub total_mb: u64,
    pub used_mb: u64,
}

impl fmt::Display for MemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pct = self.used_mb as f32 / self.total_mb as f32 * 100.0;
        let bar = make_bar(pct, 20);
        write!(
            f,
            "RAM  [{bar}] {:.1}%  ({} Mo / {} Mo)",
            pct, self.used_mb, self.total_mb
        )
    }
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_mb: u64,
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<8} {:<25} CPU: {:>5.1}%  MEM: {:>6} Mo",
            self.pid, self.name, self.cpu_usage, self.mem_mb
        )
    }
}

#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub cpu: CpuInfo,
    pub mem: MemInfo,
    pub processes: Vec<ProcessInfo>,
    pub timestamp: String,
}

impl fmt::Display for SystemSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== SysWatch — {} ===", self.timestamp)?;
        writeln!(f, "{}", self.cpu)?;
        writeln!(f, "{}", self.mem)?;
        writeln!(f, "--- Top processus ---")?;
        for p in &self.processes {
            writeln!(f, "  {}", p)?;
        }
        Ok(())
    }
}

// Barre ASCII helper
fn make_bar(pct: f32, width: usize) -> String {
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

// ============================================================
// ÉTAPE 2 — Collecte réelle et gestion d'erreurs
// ============================================================

#[derive(Debug)]
pub enum SysWatchError {
    CollectError(String),
}

impl fmt::Display for SysWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysWatchError::CollectError(msg) => write!(f, "Erreur de collecte : {}", msg),
        }
    }
}

pub fn collect_snapshot() -> Result<SystemSnapshot, SysWatchError> {
    let mut sys = System::new_all();
    sys.refresh_all();

    // Petit délai pour que sysinfo ait le temps de mesurer le CPU
    thread::sleep(Duration::from_millis(500));
    sys.refresh_all();

    // CPU
    let usage = sys.global_cpu_usage();
    let core_count = sys.cpus().len();
    let cpu = CpuInfo { usage, core_count };

    // RAM (sysinfo retourne des octets)
    let total_mb = sys.total_memory() / 1024 / 1024;
    let used_mb = sys.used_memory() / 1024 / 1024;
    let mem = MemInfo { total_mb, used_mb };

    // Processus — top 5 par CPU
    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p: &sysinfo::Process| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_usage: p.cpu_usage(),
            mem_mb: p.memory() / 1024 / 1024,
        })
        .collect();

    processes.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap());
    processes.truncate(5);

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    Ok(SystemSnapshot {
        cpu,
        mem,
        processes,
        timestamp,
    })
}

// ============================================================
// ÉTAPE 3 — Formatage des réponses réseau
// ============================================================

pub fn format_response(snapshot: &SystemSnapshot, command: &str) -> String {
    match command.trim() {
        "cpu" => format!("[CPU] {}\r\n", snapshot.cpu),
        "mem" => format!("[MEM] {}\r\n", snapshot.mem),
        "ps" => {
            let mut resp = format!("[PS] Top 5 processus — {}\r\n", snapshot.timestamp);
            resp.push_str(&format!(
                "{:<8} {:<25} {:>8}  {:>10}\r\n",
                "PID", "NOM", "CPU%", "MEM(Mo)"
            ));
            resp.push_str(&"-".repeat(58));
            resp.push_str("\r\n");
            for p in &snapshot.processes {
                resp.push_str(&format!(
                    "{:<8} {:<25} {:>7.1}%  {:>8} Mo\r\n",
                    p.pid, p.name, p.cpu_usage, p.mem_mb
                ));
            }
            resp
        }
        "all" => format!("{}\r\n", snapshot),
        "help" => concat!(
            "=== Commandes disponibles ===\r\n",
            "  cpu   — utilisation du processeur\r\n",
            "  mem   — utilisation de la mémoire RAM\r\n",
            "  ps    — top 5 processus par CPU\r\n",
            "  all   — rapport complet\r\n",
            "  help  — cette aide\r\n",
            "  quit  — fermer la connexion\r\n"
        )
        .to_string(),
        "quit" => "Au revoir !\r\n".to_string(),
        other => format!("Commande inconnue : '{}'. Tapez 'help'.\r\n", other),
    }
}

// ============================================================
// ÉTAPE 5 — Journalisation fichier (Bonus)
// ============================================================

fn log_event(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("syswatch.log")
    {
        let _ = writeln!(file, "[{}] {}", timestamp, message);
    }
}

// ============================================================
// ÉTAPE 4 — Serveur TCP multi-threadé
// ============================================================

fn handle_client(stream: TcpStream, shared_snapshot: Arc<Mutex<SystemSnapshot>>) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "inconnu".to_string());

    println!("[+] Nouveau client connecté : {}", peer);
    log_event(&format!("Connexion de {}", peer));

    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Erreur clone stream : {}", e);
            return;
        }
    };
    let reader = BufReader::new(stream);

    // Message de bienvenue
    let _ = writer
        .write_all(b"=== SysWatch Server ===\r\nTapez 'help' pour la liste des commandes.\r\n");

    for line in reader.lines() {
        let cmd = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let cmd = cmd.trim().to_lowercase();

        println!("[{}] commande : {}", peer, cmd);
        log_event(&format!("{} >> {}", peer, cmd));

        if cmd == "quit" {
            let _ = writer.write_all(b"Au revoir !\r\n");
            break;
        }

        // Lire le snapshot partagé
        let snapshot = {
            let lock = shared_snapshot.lock().unwrap();
            lock.clone()
        };

        let response = format_response(&snapshot, &cmd);
        if writer.write_all(response.as_bytes()).is_err() {
            break;
        }
    }

    println!("[-] Client déconnecté : {}", peer);
    log_event(&format!("Déconnexion de {}", peer));
}

fn main() {
    println!("Démarrage de SysWatch sur le port 7878...");

    // Collecte initiale
    let initial = collect_snapshot().unwrap_or_else(|e| {
        eprintln!("Erreur initiale : {}", e);
        std::process::exit(1);
    });

    let shared = Arc::new(Mutex::new(initial));

    // Thread de rafraîchissement toutes les 5 secondes
    {
        let shared_clone = Arc::clone(&shared);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            match collect_snapshot() {
                Ok(snap) => {
                    let mut lock = shared_clone.lock().unwrap();
                    *lock = snap;
                    println!("[refresh] Snapshot mis à jour.");
                }
                Err(e) => eprintln!("[refresh] Erreur : {}", e),
            }
        });
    }

    // Serveur TCP
    let listener = TcpListener::bind("0.0.0.0:7878").expect("Impossible de lier le port 7878");
    println!("Serveur prêt. En attente de connexions...");
    log_event("Serveur SysWatch démarré sur le port 7878");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let shared_clone = Arc::clone(&shared);
                thread::spawn(move || handle_client(s, shared_clone));
            }
            Err(e) => eprintln!("Erreur connexion entrante : {}", e),
        }
    }
}
