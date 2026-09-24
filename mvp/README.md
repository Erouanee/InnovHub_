# Dictée locale (MVP)

Outil de dictée vocale « zéro friction » : un raccourci global, on parle, le texte
apparaît. **Tout le traitement (audio et texte) s'exécute sur la machine.**

> État : **phase 1** (squelette + barre des menus + raccourci + capture + transcription
> brute affichée). Voir [Feuille de route](#feuille-de-route).

## Prérequis

| | |
|---|---|
| Machine | Mac Apple Silicon (référence : M5, 16 Go). macOS 13+ |
| Outils | Rust ≥ 1.80, Node ≥ 20, CMake, Xcode Command Line Tools |
| Disque | ~600 Mo pour le modèle Whisper |

```bash
xcode-select --install          # si besoin
brew install cmake node rust    # ou rustup
```

## Installation et lancement

```bash
npm install
./scripts/download-model.sh           # une seule fois : ~550 Mo, vérifié par SHA-256
npm run tauri dev                     # développement (rechargement à chaud du front)
npm run tauri build -- --bundles app  # .app dans src-tauri/target/release/bundle/macos/
npm test                              # tests unitaires Rust
```

`./scripts/download-model.sh small` installe un modèle plus léger (~180 Mo, qualité
moindre en français), à déclarer ensuite dans `settings.json` (`whisper_model`).

Mesurer la transcription sans micro ni interface :

```bash
say -v Thomas -o /tmp/t.aiff "Bonjour, ceci est un test de dictée."
afconvert -f WAVE -d LEI16@48000 /tmp/t.aiff /tmp/t.wav
cd src-tauri && cargo run --release --example transcribe_wav -- \
  "$HOME/Library/Application Support/com.erouanee.dictee/models/ggml-large-v3-turbo-q5_0.bin" /tmp/t.wav
```

## Utilisation

- **Alt+Espace** (par défaut) : maintenir pour dicter, relâcher pour transcrire.
- Un appui de moins de 300 ms est ignoré (appui accidentel).
- Icône micro dans la barre des menus : afficher la dernière transcription, quitter.
- Pastille flottante en bas de l'écran : « Écoute… » puis « Transcription… ».

### Réglages

Fichier `~/Library/Application Support/com.erouanee.dictee/settings.json`, créé au
premier lancement. Redémarrer l'app après modification (l'interface de réglages
arrive en phase 4).

| Clé | Défaut | Rôle |
|---|---|---|
| `shortcut` | `"Alt+Space"` | Ex. `"CmdOrCtrl+Shift+D"`, `"F13"` |
| `mode` | `"push_to_talk"` | ou `"toggle"` (appuyer pour démarrer / arrêter) |
| `whisper_model` | `"ggml-large-v3-turbo-q5_0.bin"` | Fichier dans `models/` |
| `language` | `"fr"` | Langue imposée à Whisper |
| `input_device` | `null` | Nom du micro ; `null` = micro par défaut du système |
| `max_recording_secs` | `120` | Arrêt automatique au-delà |
| `show_result_window` | `true` | Phase 1 : afficher la fenêtre après chaque dictée |

**Périphérique matériel « push-to-talk »** : une pédale ou un bouton USB programmable
qui émet une touche F13–F20 fonctionne sans code supplémentaire (`"shortcut": "F13"`).

## Permissions macOS

- **Micro** : demandé au premier enregistrement. En `npm run tauri dev`, macOS
  l'attribue à l'application qui a lancé la commande (Terminal, VS Code…), pas à
  Dictee. Si l'accès est refusé, macOS fournit un signal nul : l'app le détecte et
  affiche « Aucun signal micro ». Correction : Réglages Système › Confidentialité et
  sécurité › Micro.
- **Accessibilité** : pas nécessaire en phase 1, requise en phase 2 pour simuler Cmd+V.
- Le `.app` local est signé « ad hoc » : macOS peut redemander les autorisations
  après chaque recompilation. La signature Developer ID et la notarisation sont
  prévues en phase 5.

## Architecture

```
 raccourci global ──► Controller (machine à états pure : dictation.rs)
 (global-shortcut)        │  Pressed / Released
                          ▼
                    audio::recorder ── thread dédié cpal, mono, en RAM uniquement
                          │ CapturedAudio (Zeroizing<Vec<f32>>)
                          ▼  canal mpsc
                    thread « stt-worker » (possède le modèle, chargé 1 fois)
                      1. dsp::resample      48 kHz → 16 kHz (sinc polyphase)
                      2. dsp::detect_speech VAD énergie : coupe les silences,
                                            rejette les prises vides
                      3. stt::Transcriber   whisper.cpp + Metal
                      4. stt::clean_transcript  filtre les hallucinations connues
                          │ DictationResult { texte, latences }
                          ▼  évènements Tauri
             fenêtre principale (résultat)   indicateur flottant (état)

 Phases suivantes, entre 4 et l'UI :
   [llm] reformulation (profil) ─► [fidelity] vérification ─► [inject] presse-papiers + Cmd+V
```

| Module | Responsabilité | Testé unitairement |
|---|---|---|
| `settings.rs` | Chargement/validation/écriture atomique de `settings.json` | oui |
| `dictation.rs` | Machine à états push-to-talk / toggle | oui |
| `audio/dsp.rs` | Mixage mono, rééchantillonnage, VAD, gain | oui |
| `audio/recorder.rs` | Capture cpal dans un thread dédié | manuel |
| `stt.rs` | whisper.cpp, nettoyage du texte | partiel (logique pure) |
| `pipeline.rs` | Orchestration, mesures de latence, évènements UI | manuel |
| `tray.rs`, `ui.rs` | Barre des menus, fenêtres | manuel |

### Choix techniques

- **Tauri 2** plutôt qu'Electron : binaire d'environ 10 Mo et cœur en Rust (inférence
  native sans passer par Node), sans navigateur Chromium embarqué. Tauri 3 n'existe
  encore qu'en alpha.
- **whisper.cpp large-v3-turbo q5_0** : meilleur compromis qualité/latence en
  français sur Metal. Mesures sur M5 : **~550 ms pour 13 s de parole**, ~190 ms
  pour 3 s.
- **Fenêtre d'encodeur adaptée à la durée** (`audio_ctx`) : Whisper encode sinon
  toujours 30 s. Latence divisée par 2 sans perte visible sur nos essais. C'est une
  option expérimentale de whisper.cpp, à revalider sur le jeu d'évaluation.
- **VAD par énergie** plutôt que Silero en phase 1 : suffisant pour couper les
  silences en bord de prise, sans dépendance. whisper.cpp embarque Silero ; on
  basculera dessus si le jeu d'évaluation montre des coupures en milieu bruyant.
- **Indicateur toujours ouvert, transparent et traversé par la souris** : seul son
  contenu HTML change. Le montrer ou le cacher à chaque dictée pourrait voler le
  focus à l'application cible, ce qui casserait l'injection.
- **Rééchantillonneur maison** (~60 lignes testées) plutôt qu'une bibliothèque :
  besoin simple, 6 ms pour 13 s d'audio.

### Risques techniques et parades

| Risque | Parade |
|---|---|
| Budget de 1,5 s dépassé une fois le LLM ajouté (STT ≈ 0,55 s) | Modèle 3–4B Q4, prompt système en cache KV, sortie plafonnée, bench des candidats en phase 3 ; mode « brut » toujours disponible |
| Conflit de symboles ggml entre whisper.cpp et llama.cpp liés dans le même binaire | Test d'intégration dès le début de la phase 3 ; repli : bibliothèques dynamiques ou LLM dans un processus fils piloté par stdin/stdout (sans socket) |
| Collage bloqué (champs sécurisés, applis qui interceptent Cmd+V) | Repli par saisie clavier simulée (phase 2) |
| Gestionnaires d'historique du presse-papiers qui conservent le texte dicté | Marqueurs `org.nspasteboard.ConcealedType` / `TransientType`, restauration immédiate (phase 2) |
| Hallucinations Whisper sur silence (« Sous-titres réalisés par… ») | VAD + filtre de phrases connues (fait) |
| Ajouts ou omissions du LLM | Vérification des entités, repli sur la transcription brute (phase 3) |

## Garanties de confidentialité et leurs limites

Garanties **par conception**, vérifiables dans le code :

- **Aucun code réseau** dans l'application. Seul le script `download-model.sh`,
  lancé à la main, télécharge un modèle et vérifie son SHA-256. Au lancement, `lsof`
  ne montre aucune socket ouverte.
- **L'audio n'est jamais écrit sur disque** : buffers en mémoire, effacés
  (`zeroize`) à leur libération.
- **Aucun texte dicté dans les journaux** : les logs (stderr uniquement, aucun
  fichier) contiennent des durées et des nombres de mots, jamais de contenu. Les
  options de whisper.cpp qui impriment le texte sont désactivées.
- **Aucune télémétrie.**
- **CSP stricte** dans les fenêtres : aucune ressource externe chargeable.

Limites connues, en toute transparence :

- L'effacement mémoire est un « au mieux ». Les copies laissées par une
  réallocation de buffer et les tampons internes de whisper.cpp (spectrogramme)
  ne sont pas effacés ; ils sont écrasés à la dictée suivante ou libérés à la
  fermeture. Le système peut aussi paginer la mémoire vers le swap, qui est
  chiffré sous macOS.
- Le dernier texte transcrit reste en mémoire (fenêtre de résultat) jusqu'à la
  dictée suivante ou la fermeture.
- En mode développement (`tauri dev`), le front est servi par Vite sur
  `127.0.0.1:1420`. Ce serveur est local, mais c'est bien une socket. Le `.app` de
  production n'en ouvre aucune.
- À partir de la phase 2, le texte passe brièvement par le presse-papiers système.
- Un test automatisé prouvant l'absence de trafic, où l'app tourne dans un bac à
  sable qui interdit le réseau, est prévu en phase 5.

Ce projet ne revendique **aucune conformité réglementaire**. Points à faire valider
par un juriste avant toute diffusion : qualification des traitements au sens du
RGPD (dont l'absence de sous-traitant), statut d'« outil de saisie » par rapport
aux dispositifs médicaux en cas d'usage en santé, licences des modèles
redistribués, et contenu des mentions d'information à l'utilisateur.

## Feuille de route

1. ✅ Squelette Tauri, barre des menus, raccourci global, capture, transcription brute affichée
2. Injection au curseur dans l'application active
3. Reformulation LLM, profils, vérification de fidélité
4. Détection de l'application active, réglages, gestion des modèles
5. Métriques locales, optimisation de la latence, test « zéro réseau », installeur
