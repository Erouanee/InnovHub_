# Contexte pour les prochaines fenêtres

## État actuel : Phase 1 ✅
- ✅ Squelette Tauri 2 + barre des menus
- ✅ Raccourci global (Alt+Espace, configurable)
- ✅ Capture micro (16 kHz mono, mémoire uniquement)
- ✅ Transcription Whisper large-v3-turbo q5_0 via Metal (~550 ms pour 13 s)
- ✅ Pastille flottante + fenêtre de résultat
- ✅ 28 tests unitaires passants
- ⏳ Tests manuels : raccourci, micro, fenêtre, pastille, mode toggle

## Arborescence clé

```
mvp/
├── src-tauri/src/
│   ├── lib.rs              # point d'entrée, init app
│   ├── settings.rs         # config settings.json
│   ├── dictation.rs        # machine à états (push-to-talk / toggle)
│   ├── pipeline.rs         # orchestration, latences, évènements
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── dsp.rs          # rééchantillonnage, VAD, normalisateur
│   │   └── recorder.rs     # capture cpal
│   ├── stt.rs              # whisper.cpp, nettoyage transcription
│   ├── tray.rs             # menu barre des menus
│   └── ui.rs               # fenêtres (main + indicator)
├── src/
│   ├── types.ts            # miroir structures Rust (sérialisation)
│   ├── main.ts / .css      # fenêtre résultat
│   └── indicator.ts / .css # pastille flottante
├── scripts/
│   └── download-model.sh   # télécharge + SHA-256
├── Cargo.toml, package.json, tauri.conf.json
└── README.md               # guide complet
```

## Commandes essentielles

```bash
cd mvp

# Développement
npm run tauri dev                    # Vite hot-reload + app

# Tests
npm test                             # tests unitaires Rust (28 passing)
cargo clippy                         # linter
cargo run --release --example transcribe_wav -- <model.bin> <file.wav>

# Build
npm run build                        # frontend TypeScript → dist/
npm run tauri build -- --bundles app # .app complet dans src-tauri/target/release/bundle/macos/

# Modèle
./scripts/download-model.sh turbo   # ~550 Mo, meilleure qualité FR
./scripts/download-model.sh small   # ~180 Mo, rapide mais moins bon
```

## Latences mesurées (M5, 13 s de parole)

| Étape | Temps |
|---|---|
| Prétraitement (resample + VAD) | 6 ms |
| Transcription Whisper | ~550 ms |
| **Total** | ~560 ms |
| Budget restant (1500 ms - 560 ms) | **~940 ms** |

Optimisations déjà faites :
- Rééchantillonneur polyphase (182 ms → 6 ms)
- Fenêtre encodeur adaptée à la durée (`audio_ctx`, -50% latence)

## Phase 2 : Injection au curseur
Point d'entrée : `pipeline.rs` ligne ~300, après `set_phase(..., Idle)`.

Pipeline à ajouter :
```rust
// pipeline.rs
// Après transcription brute réussie :
fn inject_to_active_app(text: &str) -> Result<(), String> {
    // 1. Trouver app active (nom processus, titre fenêtre)
    // 2. Copier presse-papiers courant (backup)
    // 3. Écrire `text` dans presse-papiers
    // 4. Simuler Cmd+V
    // 5. Restaurer presse-papiers (ou marquer comme transitoire)
    // Repli si Cmd+V échoue : saisie clavier simulée
}
```

Dépendances probables : `objc` (Cocoa API), `enigo` (simulation clavier).

## Phase 3 : Reformulation LLM + Vérification
Deux risques majeurs :
1. **Conflit ggml** : whisper.cpp + llama.cpp embarquent tous deux ggml.
   - Test en premier avec `llama-cpp-rs` pour voir si ça compile/linke.
   - Repli : LLM dans subprocess piloté par stdin/stdout, zéro socket.

2. **Hallucination / Fidélité** : Le LLM ne doit pas ajouter d'infos.
   - Vérifier les entités (nombres, durées, noms de médicaments) entre brute et reformulée.
   - Repli : afficher la version brute et signaler l'écart.

Pipeline à ajouter :
```rust
// stt.rs
fn reformat_text(text: &str, profile: &Profile) -> Result<String, String> {
    // 1. Charger llama.cpp + prompt du profil
    // 2. Générer reformulation
}

// pipeline.rs (fidelity checker)
fn check_fidelity(raw: &str, reformatted: &str) -> Result<(), String> {
    // Extraire entités critiques (nombres, dates, négatifs)
    // Comparer et rejeter si écart > seuil
}
```

Fichiers à créer :
- `src-tauri/src/llm.rs` (chargement + inférence llama.cpp)
- `src-tauri/src/profiles.rs` (YAML/JSON des profils : prompt, gabarit, lexique)
- `src-tauri/src/fidelity.rs` (vérification entités)

## Phase 4 : Détection app + Réglages
- `src-tauri/src/app_detect.rs` : `getActiveApp()` → nom processus (CoreFoundation)
- Interface de réglages dans le front : raccourci, profil par défaut, micro, modèles
- Commandes Tauri : `set_settings`, `list_devices`, `list_profiles`

## Phase 5 : Métriques + Packaging + Tests
- Métriques locales : `sqlite` ou fichier JSON dans `~/Library/Application Support/`
  - latence par dictée, mots utiles par jour (heuristique : avant/après vérification fidelity)
  - rétention à 7 jours (nombre de sessions par jour)
- Test « zéro réseau » : lancer l'app dans une VM avec réseau bloqué, mesurer `lsof -nP`
- Installeur `.dmg`, signature Developer ID + notarisation Apple
- Jeu d'évaluation : 20+ dictées médicales fictives, mesure fidélité + latence

## Points à surveiller en Phase 2+

| Point | Action |
|---|---|
| Conflit ggml | Compiler `transcriber.rs` avec llama-cpp-rs dès le démarrage de phase 3 |
| Focus/pastille | Vérifier en tauri dev que la pastille ne vole pas le focus à TextEdit |
| Presse-papiers | Restaurer immédiatement après Cmd+V, utiliser markers transitoires si possible |
| Budget latence | Benchmark chaque nouvelle version : `cargo run --release --example transcribe_wav` |
| Hallucinations | Tester sur le jeu d'évaluation, ajouter à `HALLUCINATIONS` en stt.rs si besoin |
| Permissions macOS | Vérifier après chaque recompilation que macOS ne redemande pas le micro |

## Configuration locale

```bash
# Modèle (Phase 1 complétée)
$HOME/Library/Application\ Support/com.erouanee.dictee/models/ggml-large-v3-turbo-q5_0.bin

# Réglages (créés au démarrage s'ils manquent)
$HOME/Library/Application\ Support/com.erouanee.dictee/settings.json

# Logs de développement (stderr uniquement, jamais de contenu dicté)
# Voir tauri dev console ou RUST_LOG=info npm run tauri dev
```

## Raccourcis VSCode utiles

- `Cmd+K Cmd+0` : replier tous les blocs
- `Cmd+Shift+O` : chercher symbole dans le fichier (Rust : fn, struct)
- `Cmd+Shift+F` : chercher dans tout le projet
- Cmd+Clic sur un type Rust → définition

## Si crash ou build error

1. `cargo clean` (long mais radicial)
2. `npm run tauri dev` relance WebView après un edit TS : pas besoin de redémarrer cargo
3. Logs Rust : `RUST_LOG=debug npm run tauri dev` (beaucoup de bruit from ggml)
4. Logs d'app : les erreurs vont toujours sur stderr, même en build release
5. Vérifier qu'aucun processus `dictee` ou `cargo` ne tourne en arrière-plan

---

**Prochaine étape :** Validation manuelle phase 1 (raccourci, micro, fenêtres), puis phase 2 (injection presse-papiers).
