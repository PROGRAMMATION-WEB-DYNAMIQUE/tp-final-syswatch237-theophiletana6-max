// (gardez toutes les structures, collect_snapshot, format_response, log_event inchangés)

// ============================================================
// Serveur TCP multi-threadé (conforme au TP)
// ============================================================

fn handle_client(mut stream: TcpStream, shared_snapshot: Arc<Mutex<SystemSnapshot>>) {
    let peer = stream.peer_addr().unwrap().to_string();
    println!("[+] Client connecté : {}", peer);
    log_event(&format!("Connexion de {}", peer));

    let mut writer = stream.try_clone().expect("clone stream");
    let reader = BufReader::new(stream);

    // Message de bienvenue (conforme TP)
    let _ = writer.write_all(b"=== SysWatch Server ===\r\nTapez 'help' pour la liste des commandes.\r\n");

    for line in reader.lines() {
        let cmd = match line {
            Ok(l) => l.trim().to_lowercase(),
            Err(_) => break,
        };
        if cmd.is_empty() { continue; }

        println!("[{}] commande : {}", peer, cmd);
        log_event(&format!("{} >> {}", peer, cmd));

        if cmd == "quit" {
            let _ = writer.write_all(b"Au revoir !\r\n");
            break;
        }

        let snapshot = shared_snapshot.lock().unwrap().clone();
        let response = format_response(&snapshot, &cmd);
        let _ = writer.write_all(response.as_bytes());
    }

    println!("[-] Client déconnecté : {}", peer);
    log_event(&format!("Déconnexion de {}", peer));
}

fn main() {
    println!("Démarrage de SysWatch sur le port 7878...");

    let initial = collect_snapshot().unwrap_or_else(|e| {
        eprintln!("Erreur initiale : {}", e);
        std::process::exit(1);
    });
    let shared = Arc::new(Mutex::new(initial));

    // Thread de rafraîchissement toutes les 5 secondes
    let shared_refresh = shared.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(5));
        match collect_snapshot() {
            Ok(snap) => {
                *shared_refresh.lock().unwrap() = snap;
                println!("[refresh] Snapshot mis à jour.");
            }
            Err(e) => eprintln!("[refresh] Erreur : {}", e),
        }
    });

    let listener = TcpListener::bind("0.0.0.0:7878").expect("Impossible de lier le port 7878");
    println!("Serveur prêt. En attente de connexions...");
    log_event("Serveur SysWatch démarré sur le port 7878");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let shared_clone = shared.clone();
                thread::spawn(move || handle_client(s, shared_clone));
            }
            Err(e) => eprintln!("Erreur connexion : {}", e),
        }
    }
}