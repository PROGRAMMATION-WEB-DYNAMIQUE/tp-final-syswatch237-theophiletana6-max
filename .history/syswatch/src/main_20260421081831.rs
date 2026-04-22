// =============================================================================
// SysWatch - Serveur de surveillance système via TCP
// Commandes : cpu, mem, ps, all, help, quit
// Port : 7878 | Rafraîchissement : 5 secondes
// =============================================================================

use chrono::Local;
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};

// ─────────────────────────────────────────────────────────────────────────────
// Structures de données partagées
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot des métriques système, mis à jour toutes les 5 secondes.
#[derive(Clone, Default)]
struct SystemMetrics {
    /// Utilisation CPU globale (0.0 – 100.0)
    cpu_usage: f32,
    /// Nombre de cœurs logiques
    cpu_cores: usize,
    /// RAM utilisée (octets)
    mem_used: u64,
    /// RAM totale (octets)
    mem_total: u64,
    /// Top-5 processus (pid, nom, cpu%, mémoire en octets)
    top_processes: Vec<(u32, String, f32, u64)>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Journalisation dans syswatch.log
// ─────────────────────────────────────────────────────────────────────────────

/// Écrit une ligne horodatée dans `syswatch.log`.
/// Le format est : [YYYY-MM-DD HH:MM:SS] <message>
fn log(message: &str) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format!("[{}] {}\n", timestamp, message);

    // Affichage console (non bloquant en cas d'erreur d'écriture fichier)
    print!("{}", line);

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("syswatch.log")
    {
        let _ = file.write_all(line.as_bytes());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Barres ASCII
// ─────────────────────────────────────────────────────────────────────────────

/// Génère une barre ASCII de `total` caractères représentant `percent` %.
/// Caractères : █ (plein) et ░ (vide).
fn ascii_bar(percent: f32, total: usize) -> String {
    let filled = ((percent / 100.0) * total as f32).round() as usize;
    let filled = filled.min(total);
    let empty = total - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

// ─────────────────────────────────────────────────────────────────────────────
// Formatage des réponses aux commandes
// ─────────────────────────────────────────────────────────────────────────────

/// Retourne la réponse à la commande `cpu`.
fn fmt_cpu(m: &SystemMetrics) -> String {
    let bar = ascii_bar(m.cpu_usage, 20);
    format!(
        "CPU  : [{}] {:.1}%  ({} cœurs)\r\n",
        bar, m.cpu_usage, m.cpu_cores
    )
}

/// Retourne la réponse à la commande `mem`.
fn fmt_mem(m: &SystemMetrics) -> String {
    let used_mo = m.mem_used / (1024 * 1024);
    let total_mo = m.mem_total / (1024 * 1024);
    let percent = if m.mem_total > 0 {
        (m.mem_used as f32 / m.mem_total as f32) * 100.0
    } else {
        0.0
    };
    let bar = ascii_bar(percent, 20);
    format!(
        "RAM  : [{}] {:.1}%  {} Mo / {} Mo\r\n",
        bar, percent, used_mo, total_mo
    )
}

/// Retourne la réponse à la commande `ps` (top 5 processus).
fn fmt_ps(m: &SystemMetrics) -> String {
    let mut out = String::new();
    out.push_str("PID      CPU%    MEM(Mo)  NOM\r\n");
    out.push_str("-------- ------- -------- --------------------------------\r\n");
    for (pid, name, cpu, mem) in &m.top_processes {
        let mem_mo = mem / (1024 * 1024);
        out.push_str(&format!(
            "{:<8} {:>6.1}% {:>7} Mo  {}\r\n",
            pid, cpu, mem_mo, name
        ));
    }
    out
}

/// Retourne la réponse complète à la commande `all`.
fn fmt_all(m: &SystemMetrics) -> String {
    format!(
        "=== Rapport système ===\r\n{}{}{}\r\n",
        fmt_cpu(m),
        fmt_mem(m),
        fmt_ps(m)
    )
}

/// Retourne le texte d'aide.
fn fmt_help() -> String {
    concat!(
        "Commandes disponibles :\r\n",
        "  cpu   - Utilisation CPU\r\n",
        "  mem   - Utilisation RAM\r\n",
        "  ps    - Top 5 processus (CPU)\r\n",
        "  all   - CPU + RAM + processus\r\n",
        "  help  - Afficher cette aide\r\n",
        "  quit  - Fermer la connexion\r\n",
    )
    .to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Thread de collecte des métriques (arrière-plan)
// ─────────────────────────────────────────────────────────────────────────────

/// Lance une boucle infinie qui rafraîchit les métriques toutes les 5 secondes
/// et met à jour le `Arc<Mutex<SystemMetrics>>` partagé.
fn metrics_collector(shared: Arc<Mutex<SystemMetrics>>) {
    // Initialisation de sysinfo avec uniquement les sous-systèmes nécessaires
    let mut sys = System::new_with_specifics(
        RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything())
            .with_processes(ProcessRefreshKind::everything()),
    );

    log("Collecteur de métriques démarré (intervalle : 5 s)");

    loop {
        // Premier rafraîchissement : nécessaire pour initialiser les compteurs CPU
        sys.refresh_all();
        // Petite pause pour que sysinfo puisse calculer les deltas CPU
        thread::sleep(Duration::from_millis(200));
        sys.refresh_all();

        // ── CPU ──────────────────────────────────────────────────────────────
        let cpu_usage = sys.global_cpu_usage();
        let cpu_cores = sys.cpus().len();

        // ── RAM ──────────────────────────────────────────────────────────────
        let mem_used = sys.used_memory();
        let mem_total = sys.total_memory();

        // ── Processus : top 5 par CPU ─────────────────────────────────────
        let mut procs: Vec<(u32, String, f32, u64)> = sys
            .processes()
            .iter()
            .map(|(pid, p)| {
                (
                    pid.as_u32(),
                    p.name().to_string_lossy().to_string(),
                    p.cpu_usage(),
                    p.memory(),
                )
            })
            .collect();

        // Tri décroissant par CPU%
        procs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        procs.truncate(5);

        // ── Mise à jour partagée ──────────────────────────────────────────
        if let Ok(mut m) = shared.lock() {
            m.cpu_usage = cpu_usage;
            m.cpu_cores = cpu_cores;
            m.mem_used = mem_used;
            m.mem_total = mem_total;
            m.top_processes = procs;
        }

        // Attente avant la prochaine collecte
        thread::sleep(Duration::from_secs(5));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Gestion d'un client connecté
// ─────────────────────────────────────────────────────────────────────────────

/// Traite la session d'un client TCP jusqu'à déconnexion ou commande `quit`.
fn handle_client(stream: TcpStream, shared: Arc<Mutex<SystemMetrics>>) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "inconnu".to_string());

    log(&format!("Connexion : {}", peer));

    // Clonage pour lecture / écriture séparée
    let stream_write = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            log(&format!("Erreur clone stream {} : {}", peer, e));
            return;
        }
    };

    let mut writer = std::io::BufWriter::new(stream_write);
    let reader = BufReader::new(&stream);

    // Message de bienvenue
    let _ = writer
        .write_all(b"=== SysWatch Server ===\r\nTapez 'help' pour la liste des commandes.\r\n");
    let _ = writer.flush();

    // Lecture des commandes ligne par ligne
    for line in reader.lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break, // Déconnexion inattendue
        };

        // Nettoyage : suppression des espaces et du \r résiduel
        let cmd = raw.trim().to_lowercase();

        if cmd.is_empty() {
            continue;
        }

        log(&format!("Commande de {} : {}", peer, cmd));

        // Construction de la réponse
        let response: String = match cmd.as_str() {
            "cpu" => {
                let m = shared.lock().map(|m| m.clone()).unwrap_or_default();
                fmt_cpu(&m)
            }
            "mem" => {
                let m = shared.lock().map(|m| m.clone()).unwrap_or_default();
                fmt_mem(&m)
            }
            "ps" => {
                let m = shared.lock().map(|m| m.clone()).unwrap_or_default();
                fmt_ps(&m)
            }
            "all" => {
                let m = shared.lock().map(|m| m.clone()).unwrap_or_default();
                fmt_all(&m)
            }
            "help" => fmt_help(),
            "quit" => {
                let bye = "Au revoir !\r\n".to_string();
                let _ = writer.write_all(bye.as_bytes());
                let _ = writer.flush();
                log(&format!("Déconnexion (quit) : {}", peer));
                return;
            }
            other => format!(
                "Commande inconnue : '{}'. Tapez 'help' pour la liste des commandes.\r\n",
                other
            ),
        };

        if writer.write_all(response.as_bytes()).is_err() {
            break;
        }
        if writer.flush().is_err() {
            break;
        }
    }

    log(&format!("Déconnexion : {}", peer));
}

// ─────────────────────────────────────────────────────────────────────────────
// Point d'entrée principal
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    let addr = "0.0.0.0:7878";

    // Données partagées entre tous les threads
    let shared: Arc<Mutex<SystemMetrics>> = Arc::new(Mutex::new(SystemMetrics::default()));

    // ── Thread de collecte en arrière-plan ───────────────────────────────────
    {
        let shared_clone = Arc::clone(&shared);
        thread::spawn(move || {
            metrics_collector(shared_clone);
        });
    }

    // ── Liaison du listener TCP ──────────────────────────────────────────────
    let listener = TcpListener::bind(addr).expect("Impossible de lier le port 7878");
    log(&format!("SysWatch écoute sur {}", addr));

    // ── Boucle d'acceptation des connexions ───────────────────────────────────
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let shared_clone = Arc::clone(&shared);
                // Un thread par client
                thread::spawn(move || {
                    handle_client(stream, shared_clone);
                });
            }
            Err(e) => {
                log(&format!("Erreur d'acceptation de connexion : {}", e));
            }
        }
    }
}
