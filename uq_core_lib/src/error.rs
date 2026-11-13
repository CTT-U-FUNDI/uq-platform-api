use thiserror::Error;

/// Type d'erreur unifié pour tout UQ Core.
/// Chaque fonction publique retourne Result<T, UqError>.
#[derive(Error, Debug)]
pub enum UqError {
    // ─── Cryptographie ───────────────────────
    #[error("Erreur cryptographique: {0}")]
    CryptoError(String),

    #[error("Clé invalide: {0}")]
    InvalidKey(String),

    #[error("Signature invalide: {0}")]
    InvalidSignature(String),

    #[error("Déchiffrement échoué: {0}")]
    DecryptionFailed(String),

    // ─── Adresse & Identité ──────────────────
    #[error("Adresse UQ invalide: {0}")]
    InvalidAddress(String),

    #[error("Format d'adresse non reconnu: {0}")]
    AddressFormatError(String),

    #[error("Checksum d'adresse invalide: attendu {0}, reçu {1}")]
    AddressChecksumError(String, String),

    // ─── Réseau ──────────────────────────────
    #[error("Erreur réseau: {0}")]
    NetworkError(String),

    #[error("Connexion refusée: {0}")]
    ConnectionRefused(String),

    #[error("Hôte introuvable: {0}")]
    HostNotFound(String),

    #[error("Timeout après {0}ms")]
    Timeout(u64),

    // ─── DHT ─────────────────────────────────
    #[error("Erreur DHT: {0}")]
    DhtError(String),

    #[error("Valeur DHT expirée (TTL dépassé)")]
    DhtValueExpired,

    #[error("Aucun nœud trouvé pour la clé {0}")]
    DhtNoPeers(String),

    // ─── Confiance ────────────────────────────
    #[error("Niveau de confiance insuffisant: {0} (minimum: {1})")]
    TrustTooLow(f64, f64),

    #[error("Gène non initialisé")]
    GeneNotInitialized,

    // ─── Permissions ─────────────────────────
    #[error("Permission refusée: {0}")]
    PermissionDenied(String),

    #[error("Module non enregistré: {0}")]
    ModuleNotRegistered(String),

    // ─── Stockage ────────────────────────────
    #[error("Erreur stockage: {0}")]
    StorageError(String),

    #[error("Données corrompues: {0}")]
    CorruptedData(String),

    // ─── Sérialisation ───────────────────────
    #[error("Erreur sérialisation: {0}")]
    SerializationError(String),

    #[error("Erreur désérialisation: {0}")]
    DeserializationError(String),

    // ─── Acoustique ──────────────────────────
    #[error("Microphone non disponible: {0}")]
    MicrophoneUnavailable(String),

    #[error("Haut-parleur non disponible: {0}")]
    SpeakerUnavailable(String),

    // ─── Interne ─────────────────────────────
    #[error("Erreur interne: {0}")]
    InternalError(String),

    #[error("Non initialisé: {0}")]
    NotInitialized(String),

    #[error("Déjà initialisé")]
    AlreadyInitialized,
}
