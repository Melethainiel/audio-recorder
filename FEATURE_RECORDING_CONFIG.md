# Feature: Recording Configuration

Cette branche (`feature/recording-config`) ajoute des options de configuration pour le stockage et l'upload des enregistrements.

## Nouvelles fonctionnalités

### 1. Dossier d'enregistrement personnalisé
- Interface de sélection de dossier via file picker GTK4
- Support de l'expansion du tilde (~) vers le répertoire home
- Fallback sur le dossier courant si non configuré
- Création automatique du dossier s'il n'existe pas

### 2. Upload automatique vers N8N
- Configuration d'un endpoint webhook N8N
- Upload asynchrone (non-bloquant) avec timeout de 30 secondes
- Format multipart/form-data avec :
  - `file`: fichier audio (.ogg)
  - `filename`: nom du fichier
  - `timestamp`: date/heure ISO 8601
- Notifications GTK pour le feedback (succès/échec)

### 3. Options de stockage flexibles
- **Local uniquement**: Enregistrer dans le dossier configuré
- **Upload uniquement**: Envoyer à N8N puis supprimer le fichier local
- **Hybride**: Enregistrer localement ET uploader vers N8N

## Interface utilisateur

Nouvelles sections dans le dialog Settings (⚙):

### Dossier d'enregistrement
- Champ texte affichant le chemin actuel
- Bouton "Parcourir..." ouvrant un file picker
- Placeholder: "Dossier courant"

### Upload N8N
- Checkbox "Activer l'upload vers N8N"
- Champ URL de l'endpoint (activé seulement si checkbox cochée)
- Checkbox "Conserver le fichier localement après upload"

## Configuration

Tous les paramètres sont sauvegardés dans `~/.config/audio-recorder/config.json`:

```json
{
  "selected_mic_index": 0,
  "selected_loopback_index": null,
  "mic_gain": 1.0,
  "save_directory": "/home/user/Recordings",
  "n8n_endpoint": "https://n8n.example.com/webhook/audio",
  "n8n_enabled": true,
  "save_locally": true
}
```

## Dépendances ajoutées

- `reqwest` (0.11) avec features "blocking" et "multipart"
- `tokio` (1.x) avec features "rt", "rt-multi-thread", et "fs"

## Exemples d'utilisation

### Upload vers N8N avec webhook

1. Ouvrir Settings (⚙)
2. Activer "Activer l'upload vers N8N"
3. Entrer l'URL de votre webhook N8N: `https://your-n8n.com/webhook/audio-upload`
4. Choisir si vous voulez garder le fichier localement
5. Enregistrer et arrêter un audio
6. Une notification apparaît pour confirmer l'upload

### Endpoint N8N - Configuration recommandée

Dans N8N, créez un workflow avec un trigger Webhook:
- Method: POST
- Path: `/audio-upload` (ou autre)
- Response Mode: "When Last Node Finishes"

Le webhook recevra:
- `file`: fichier audio (binary)
- `filename`: nom du fichier
- `timestamp`: timestamp ISO 8601

## Tests

Pour tester sans N8N, vous pouvez utiliser un serveur local:

```bash
# Avec Python
python3 -m http.server 8000

# Ou avec un webhook de test
https://webhook.site
```

## Notes techniques

- L'upload est asynchrone et ne bloque pas l'interface
- Timeout de 30 secondes pour éviter les blocages
- Les erreurs réseau sont capturées et affichées via notifications
- Le fichier reste toujours sauvegardé localement en cas d'échec d'upload

## TODO (futures améliorations)

- [ ] Mécanisme de retry automatique en cas d'échec
- [ ] Queue d'upload pour les fichiers non envoyés
- [ ] Support d'autres formats (MP3, WAV, FLAC)
- [ ] Metadata enrichies (durée, taille, sample rate)
- [ ] Authentification (Bearer token, API key)
