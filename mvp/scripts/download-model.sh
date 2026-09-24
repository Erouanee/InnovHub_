#!/usr/bin/env bash
# Télécharge un modèle whisper.cpp et vérifie son empreinte SHA-256.
# C'est la SEULE opération réseau du projet ; elle n'est jamais lancée par l'app.
#
# Usage : ./scripts/download-model.sh [turbo|small]
#   turbo (défaut) : large-v3-turbo quantifié q5_0, ~550 Mo, meilleure qualité en français
#   small          : small quantifié q5_1, ~180 Mo, pour les machines modestes
set -euo pipefail

VARIANT="${1:-turbo}"
case "$VARIANT" in
  turbo)
    FILE="ggml-large-v3-turbo-q5_0.bin"
    SHA256="394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
    ;;
  small)
    FILE="ggml-small-q5_1.bin"
    SHA256="ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb"
    ;;
  *)
    echo "Variante inconnue : $VARIANT (attendu : turbo | small)" >&2
    exit 2
    ;;
esac

URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/${FILE}"
# Doit correspondre à `identifier` dans src-tauri/tauri.conf.json.
DEST_DIR="${DICTEE_MODELS_DIR:-$HOME/Library/Application Support/com.erouanee.dictee/models}"
DEST="$DEST_DIR/$FILE"

mkdir -p "$DEST_DIR"

if [[ -f "$DEST" ]] && echo "$SHA256  $DEST" | shasum -a 256 -c --status; then
  echo "Déjà présent et intègre : $DEST"
  exit 0
fi

TMP="$DEST.part"
trap 'rm -f "$TMP"' EXIT

echo "Téléchargement de $FILE depuis $URL"
curl --fail --location --proto '=https' --tlsv1.2 --progress-bar -o "$TMP" "$URL"

echo "Vérification SHA-256…"
if ! echo "$SHA256  $TMP" | shasum -a 256 -c --status; then
  echo "ÉCHEC : l'empreinte ne correspond pas. Fichier supprimé." >&2
  exit 1
fi

mv "$TMP" "$DEST"
trap - EXIT
echo "OK : $DEST"
