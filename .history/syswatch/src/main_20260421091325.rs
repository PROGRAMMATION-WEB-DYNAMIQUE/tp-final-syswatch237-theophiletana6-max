// =============================================================================
// SysWatch — Moniteur système en réseau
// TP Intégral Rust | Génie Logiciel L4 | ENSPD 2025-2026
//
// Étapes couvertes :
//   1. Modélisation des données (structs + trait Display)
//   2. Collecte réelle et gestion d'erreurs (Result, enum d'erreur)
//   3. Formatage des réponses réseau (pattern matching, barres ASCII)
//   4. Serveur TCP multi-threadé (TcpListener, Arc<Mutex<T>>)
//   5. Journalisation fichier — BONUS (OpenOptions, append)
// =============================================================================

use chrono::Local;
use std::fmt;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use sysinfo::System;

// =============================================================================
// ÉTAPE 1 — Modélisation des données
// Concepts : struct, impl, trait Display, Vec<T>, derive(Debug, Clone)
// =============================================================================

/// Informations sur le processeur.
#[derive(Debug, Clone)]
struct CpuInfo {
    usage_percent: f32,
    core_count: usize,
}

/// Informations sur la mémoire RAM.
#[derive(Debug, Clone)]
struct MemInfo {
    total_mb: u64,
    used_mb: u64,
    free_mb: u64,
}

/// Informations sur un processus.
#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
}

/// Snapshot complet du système à un instant donné.
#[derive(Debug, Clone)]
struct SystemSnapshot {
    timestamp: String,
    cpu: CpuInfo,
    memory: MemInfo,
    top_processes: Vec<ProcessInfo>,
}

// --- Implémentation du trait Display ---

impl fmt::Display for CpuInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CPU: {:.1}% ({} cœurs)",
            self.usage_percent, self.core_count
        )
    }
}

impl fmt::Display for MemInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MEM: {} Mo utilisés / {} Mo total ({} Mo libres)",
            self.used_mb, self.total_mb, self.free_mb
        )
    }
}

impl fmt::Display for ProcessInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "  [{:>6}] {:<25} CPU:{:>5.1}%  MEM:{:>5} Mo",
            self.pid, self.name, self.cpu_usage, self.memory_mb
        )
    }
}

impl fmt::Display for SystemSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== SysWatch — {} ===", self.timestamp)?;
        writeln!(f, "{}", self.cpu)?;
        writeln!(f, "{}", self.memory)?;
        writeln!(f, "--- Top Processus ---")?;
        for p in &self.top_processes {
            writeln!(f, "{}", p)?;
        }
        write!(f, "=====================")
    }
}

// =============================================================================
// ÉTAPE 2 — Collecte réelle et gestion d'erreurs
// Concepts : Result<T, E>, enum d'erreur personnalisée, closures, .map(), .sort_by()
// =============================================================================

/// Erreurs possibles lors de la collecte des métriques.
#[derive(Debug)]
enum SysWatchError {
    CollectionFailed(String),
}

impl fmt::Display for SysWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SysWatchError::CollectionFailed(msg) => write!(f, "Erreur collecte: {}", msg),
        }
    }
}

impl std::error::Error for SysWatchError {}

/// Collecte les métriques système réelles et retourne un snapshot.
/// Retourne une erreur si aucun CPU n'est détecté.
fn collect_snapshot() -> Result<SystemSnapshot, SysWatchError> {
    let mut sys = System::new_all();
    sys.refresh_all();

    // Petite pause nécessaire pour que sysinfo calcule les deltas CPU
    thread::sleep(Duration::from_millis(500));
    sys.refresh_all();

    let cpu_usage = sys.global_cpu_info().cpu_usage(); 
    let core_count = sys.cpus().len();

    if core_count == 0 {
        return Err(SysWatchError::CollectionFailed(
            "Aucun CPU détecté".to_string(),
        ));
    }

    let total_mb = sys.total_memory() / 1024 / 1024;
    let used_mb = sys.used_memory() / 1024 / 1024;
    let free_mb = sys.free_memory() / 1024 / 1024;

    // Collecte et tri des processus : top 5 par consommation CPU
    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string(), 
            cpu_usage: p.cpu_usage(),
            memory_mb: p.memory() / 1024 / 1024,
        })
        .collect();

    // Tri décroissant par CPU%
    processes.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    processes.truncate(5);

    Ok(SystemSnapshot {
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        cpu: CpuInfo {
            usage_percent: cpu_usage,
            core_count,
        },
        memory: MemInfo {
            total_mb,
            used_mb,
            free_mb,
        },
        top_processes: processes,
    })
}

// =============================================================================
// ÉTAPE 3 — Formatage des réponses réseau
// Concepts : pattern matching exhaustif sur &str, itérateurs, barres ASCII
// =============================================================================

/// Génère une barre ASCII de `width` caractères pour un pourcentage donné.
/// Utilise █ (plein) et ░ (vide).
fn ascii_bar(percent: f32, width: usize) -> String {
    let filled = ((percent / 100.0) * width as f32).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    "█".repeat(filled) + &"░".repeat(empty)
}

/// Construit la réponse textuelle à envoyer au client selon la commande reçue.
fn format_response(snapshot: &SystemSnapshot, command: &str) -> String {
    let cmd = command.trim().to_lowercase();

    match cmd.as_str() {
        // --- cpu : utilisation CPU avec barre ASCII ---
        "cpu" => {
            let bar = ascii_bar(snapshot.cpu.usage_percent, 20);
            format!(
                "[CPU]\n{}\n[{}] {:.1}%\n",
                snapshot.cpu, bar, snapshot.cpu.usage_percent
            )
        }

        // --- mem : utilisation RAM avec barre ASCII ---
        "mem" => {
            let percent = if snapshot.memory.total_mb > 0 {
                (snapshot.memory.used_mb as f64 / snapshot.memory.total_mb as f64) * 100.0
            } else {
                0.0
            };
            let bar = ascii_bar(percent as f32, 20);
            format!(
                "[MÉMOIRE]\n{}\n[{}] {:.1}%\n",
                snapshot.memory, bar, percent
            )
        }

        // --- ps : top 5 processus ---
        "ps" => {
            let lines: String = snapshot
                .top_processes
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {}", i + 1, p))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "[PROCESSUS — Top {}]\n{}\n",
                snapshot.top_processes.len(),
                lines
            )
        }

        // --- all : vue complète ---
        "all" => format!("{}\n", snapshot),

        // --- help : liste des commandes ---
        "help" => concat!(
            "Commandes disponibles:\n",
            "  cpu   — Usage CPU + barre ASCII\n",
            "  mem   — Mémoire RAM + barre ASCII\n",
            "  ps    — Top 5 processus (par CPU)\n",
            "  all   — Vue complète du système\n",
            "  help  — Afficher cette aide\n",
            "  quit  — Fermer la connexion\n",
        )
        .to_string(),

        // --- quit : géré dans handle_client, mais on prépare la réponse ---
        "quit" | "exit" => "Au revoir !\n".to_string(),

        // --- commande inconnue ---
        _ => format!(
            "Commande inconnue: '{}'. Tapez 'help' pour la liste des commandes.\n",
            command.trim()
        ),
    }
}

// =============================================================================
// ÉTAPE 4 — Serveur TCP multi-threadé
// Concepts : TcpListener, TcpStream, thread::spawn, Arc<Mutex<T>>
// =============================================================================

/// Thread de rafraîchissement : met à jour le snapshot partagé toutes les 5 s.
fn snapshot_refresher(snapshot: Arc<Mutex<SystemSnapshot>>) {
    loop {
        thread::sleep(Duration::from_secs(5));
        match collect_snapshot() {
            Ok(new_snap) => {
                if let Ok(mut snap) = snapshot.lock() {
                    *snap = new_snap;
                }
                log_event("[refresh] Métriques mises à jour");
            }
            Err(e) => eprintln!("[refresh] Erreur: {}", e),
        }
    }
}

/// Gère la session complète d'un client connecté.
/// Lit les commandes ligne par ligne et envoie les réponses formatées.
fn handle_client(mut stream: TcpStream, snapshot: Arc<Mutex<SystemSnapshot>>) {
    // Adresse du client pour les logs
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "inconnu".to_string());

    log_event(&format!("[+] Connexion de {}", peer));

    // Message de bienvenue
    let welcome = concat!(
        "=== SysWatch Server ===\r\n",
        "Tapez 'help' pour la liste des commandes.\r\n",
    );
    if stream.write_all(welcome.as_bytes()).is_err() {
        return;
    }

    // BufReader pour lire ligne par ligne
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(e) => {
            log_event(&format!(
                "[!] Impossible de cloner le stream {}: {}",
                peer, e
            ));
            return;
        }
    };

    for line in reader.lines() {
        let cmd = match line {
            Ok(l) => l,
            Err(_) => break, // Déconnexion inattendue
        };

        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            continue;
        }

        log_event(&format!("[{}] commande: '{}'", peer, cmd));

        // Commande quit : fermer proprement
        if cmd.eq_ignore_ascii_case("quit") || cmd.eq_ignore_ascii_case("exit") {
            let _ = stream.write_all(b"Au revoir !\r\n");
            break;
        }

        // Construire la réponse depuis le snapshot partagé
        // Le verrou est relâché immédiatement après le clone
        let response = {
            let snap = snapshot.lock().unwrap_or_else(|e| e.into_inner());
            format_response(&snap, &cmd)
        };

        // Envoyer la réponse + marqueur de fin
        if stream.write_all(response.as_bytes()).is_err() {
            break;
        }
        if stream.write_all(b"\r\n").is_err() {
            break;
        }
    }

    log_event(&format!("[-] Déconnexion de {}", peer));
}

// =============================================================================
// ÉTAPE 5 (BONUS) — Journalisation fichier
// Concepts : OpenOptions, mode append, std::io::Write
// =============================================================================

/// Écrit un événement horodaté dans la console ET dans `syswatch.log`.
/// Format : [YYYY-MM-DD HH:MM:SS] message
fn log_event(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format!("[{}] {}\n", timestamp, message);

    // Affichage console
    print!("{}", line);

    // Écriture dans le fichier (mode append, création automatique)
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("syswatch.log")
    {
        let _ = file.write_all(line.as_bytes());
    }
}

// =============================================================================
// POINT D'ENTRÉE
// =============================================================================

fn main() {
    println!("SysWatch démarrage...");

    // Collecte initiale — obligatoire avant d'accepter des connexions
    let initial = collect_snapshot().expect("Impossible de collecter les métriques initiales");
    log_event("Métriques initiales collectées avec succès");
    println!("{}", initial);

    // Snapshot partagé entre tous les threads via Arc<Mutex<T>>
    let shared_snapshot = Arc::new(Mutex::new(initial));

    // Thread de rafraîchissement automatique toutes les 5 secondes
    {
        let snap_clone = Arc::clone(&shared_snapshot);
        thread::spawn(move || snapshot_refresher(snap_clone));
    }

    // Démarrage du serveur TCP sur le port 7878
    let listener = TcpListener::bind("0.0.0.0:7878").expect("Impossible de bind le port 7878");
    log_event("Serveur en écoute sur 0.0.0.0:7878");
    println!("Connecte-toi avec : nc localhost 7878");

    // Boucle d'acceptation des connexions entrantes
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let snap_clone = Arc::clone(&shared_snapshot);
                // Un thread par client
                thread::spawn(move || handle_client(stream, snap_clone));
            }
            Err(e) => eprintln!("Erreur connexion entrante: {}", e),
        }
    }
}
