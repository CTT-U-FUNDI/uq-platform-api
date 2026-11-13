use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// ÉTATS DU HANDSHAKE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum HandshakeState {
    Idle,
    Waiting,
    Completed,
    Failed,
    Timeout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HandshakeStep {
    Hello,
    Challenge,
    Response,
    Confirm,
    Done,
}

// ═══════════════════════════════════════════════════════════════
// MESSAGE DE HANDSHAKE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct HandshakeMessage {
    pub step: HandshakeStep,
    pub sender: [u8; 32],
    pub receiver: [u8; 32],
    pub nonce: u64,
    pub challenge: Option<[u8; 32]>,
    pub response: Option<[u8; 32]>,
    pub timestamp: u64,
}

impl HandshakeMessage {
    pub fn new(step: HandshakeStep, sender: [u8; 32], receiver: [u8; 32]) -> Self {
        HandshakeMessage {
            step,
            sender,
            receiver,
            nonce: rand::random(),
            challenge: None,
            response: None,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    pub fn with_challenge(mut self, challenge: [u8; 32]) -> Self {
        self.challenge = Some(challenge);
        self
    }

    pub fn with_response(mut self, response: [u8; 32]) -> Self {
        self.response = Some(response);
        self
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE HANDSHAKE
// ═══════════════════════════════════════════════════════════════

pub struct HandshakeManager {
    state: HandshakeState,
    current_step: HandshakeStep,
    our_address: [u8; 32],
    peer_address: Option<[u8; 32]>,
    our_challenge: Option<[u8; 32]>,     // Le challenge qu'on a envoyé
    peer_challenge: Option<[u8; 32]>,     // Le challenge reçu du pair
    attempts: u32,
    max_attempts: u32,
    timeout_secs: u64,
    start_time: Option<u64>,
    completed_handshakes: u64,
    failed_handshakes: u64,
}

impl HandshakeManager {
    pub fn new(our_address: [u8; 32], max_attempts: u32, timeout_secs: u64) -> Self {
        HandshakeManager {
            state: HandshakeState::Idle,
            current_step: HandshakeStep::Hello,
            our_address,
            peer_address: None,
            our_challenge: None,
            peer_challenge: None,
            attempts: 0,
            max_attempts,
            timeout_secs,
            start_time: None,
            completed_handshakes: 0,
            failed_handshakes: 0,
        }
    }

    /// Initie un handshake avec un pair
    pub fn initiate(&mut self, peer_address: [u8; 32]) -> Result<HandshakeMessage, ChainError> {
        if self.state != HandshakeState::Idle && self.state != HandshakeState::Failed {
            return Err(ChainError::InternalError("Handshake déjà en cours".into()));
        }

        self.peer_address = Some(peer_address);
        self.state = HandshakeState::Waiting;
        self.current_step = HandshakeStep::Hello;
        self.attempts = 1;
        self.start_time = Some(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs());

        Ok(HandshakeMessage::new(HandshakeStep::Hello, self.our_address, peer_address))
    }

    /// Traite un message de handshake reçu
    pub fn process_message(&mut self, msg: &HandshakeMessage) -> Result<Option<HandshakeMessage>, ChainError> {
        match msg.step {
            HandshakeStep::Hello => {
                // On reçoit Hello → On génère un challenge et on répond
                let challenge: [u8; 32] = rand::random();
                self.our_challenge = Some(challenge);
                self.peer_address = Some(msg.sender);
                self.state = HandshakeState::Waiting;
                self.current_step = HandshakeStep::Challenge;

                let reply = HandshakeMessage::new(HandshakeStep::Challenge, self.our_address, msg.sender)
                    .with_challenge(challenge);
                Ok(Some(reply))
            }

            HandshakeStep::Challenge => {
                // On reçoit un challenge → On calcule la réponse
                if let Some(challenge) = msg.challenge {
                    self.peer_challenge = Some(challenge);
                    let response = Self::compute_response_static(&challenge, &self.our_address);
                    self.current_step = HandshakeStep::Response;

                    let reply = HandshakeMessage::new(HandshakeStep::Response, self.our_address, msg.sender)
                        .with_response(response);
                    Ok(Some(reply))
                } else {
                    self.fail();
                    Err(ChainError::InternalError("Challenge manquant".into()))
                }
            }

            HandshakeStep::Response => {
                // On reçoit une réponse → On vérifie
                if let (Some(challenge), Some(response)) = (self.our_challenge, msg.response) {
                    let expected = Self::compute_response_static(&challenge, &msg.sender);
                    if response == expected {
                        self.state = HandshakeState::Completed;
                        self.current_step = HandshakeStep::Done;
                        self.completed_handshakes += 1;

                        let confirm = HandshakeMessage::new(HandshakeStep::Confirm, self.our_address, msg.sender);
                        Ok(Some(confirm))
                    } else {
                        self.fail();
                        Err(ChainError::InternalError("Réponse invalide".into()))
                    }
                } else {
                    self.fail();
                    Err(ChainError::InternalError("Données manquantes pour vérification".into()))
                }
            }

            HandshakeStep::Confirm => {
                // On reçoit Confirm → Handshake terminé
                self.state = HandshakeState::Completed;
                self.current_step = HandshakeStep::Done;
                self.completed_handshakes += 1;
                Ok(None)
            }

            HandshakeStep::Done => Ok(None),
        }
    }

    /// Calcule la réponse au challenge (fonction statique déterministe)
    fn compute_response_static(challenge: &[u8; 32], address: &[u8; 32]) -> [u8; 32] {
        let mut response = [0u8; 32];
        for i in 0..32 {
            response[i] = challenge[i].wrapping_add(address[i]).wrapping_add(i as u8);
        }
        response
    }

    /// Marque le handshake comme échoué
    fn fail(&mut self) {
        self.state = HandshakeState::Failed;
        self.failed_handshakes += 1;
    }

    /// Vérifie si le handshake a expiré
    pub fn check_timeout(&mut self) -> bool {
        if self.state != HandshakeState::Waiting {
            return false;
        }

        if let Some(start) = self.start_time {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if now - start > self.timeout_secs {
                if self.attempts < self.max_attempts {
                    self.attempts += 1;
                    self.start_time = Some(now);
                    false
                } else {
                    self.state = HandshakeState::Timeout;
                    self.failed_handshakes += 1;
                    true
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    /// Réinitialise le handshake
    pub fn reset(&mut self) {
        self.state = HandshakeState::Idle;
        self.current_step = HandshakeStep::Hello;
        self.peer_address = None;
        self.our_challenge = None;
        self.peer_challenge = None;
        self.attempts = 0;
        self.start_time = None;
    }

    /// Retourne l'état actuel
    pub fn get_state(&self) -> &HandshakeState { &self.state }

    /// Vérifie si le handshake est complété
    pub fn is_completed(&self) -> bool { self.state == HandshakeState::Completed }

    /// Vérifie si un handshake est en cours
    pub fn is_active(&self) -> bool { self.state == HandshakeState::Waiting }

    /// Retourne l'adresse du pair
    pub fn get_peer(&self) -> Option<&[u8; 32]> { self.peer_address.as_ref() }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> HandshakeStats {
        HandshakeStats {
            state: format!("{:?}", self.state),
            step: format!("{:?}", self.current_step),
            attempts: self.attempts,
            completed: self.completed_handshakes,
            failed: self.failed_handshakes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandshakeStats {
    pub state: String,
    pub step: String,
    pub attempts: u32,
    pub completed: u64,
    pub failed: u64,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_addr(n: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = n; a }

    #[test] fn test_new() {
        let hm = HandshakeManager::new(make_addr(1), 3, 30);
        assert_eq!(hm.get_state(), &HandshakeState::Idle);
        assert!(!hm.is_completed());
    }

    #[test] fn test_initiate() {
        let mut hm = HandshakeManager::new(make_addr(1), 3, 30);
        let msg = hm.initiate(make_addr(2)).unwrap();
        assert_eq!(msg.step, HandshakeStep::Hello);
        assert!(hm.is_active());
    }

    #[test] fn test_double_initiate_fails() {
        let mut hm = HandshakeManager::new(make_addr(1), 3, 30);
        hm.initiate(make_addr(2)).unwrap();
        assert!(hm.initiate(make_addr(3)).is_err());
    }

    #[test] fn test_full_handshake() {
        let mut alice = HandshakeManager::new(make_addr(1), 3, 30);
        let mut bob = HandshakeManager::new(make_addr(2), 3, 30);

        // Alice → Bob : Hello
        let hello = alice.initiate(make_addr(2)).unwrap();
        assert_eq!(hello.step, HandshakeStep::Hello);

        // Bob reçoit Hello → envoie Challenge
        let challenge = bob.process_message(&hello).unwrap().unwrap();
        assert_eq!(challenge.step, HandshakeStep::Challenge);

        // Alice reçoit Challenge → envoie Response
        let response = alice.process_message(&challenge).unwrap().unwrap();
        assert_eq!(response.step, HandshakeStep::Response);

        // Bob reçoit Response → vérifie et envoie Confirm
        let confirm = bob.process_message(&response).unwrap().unwrap();
        assert_eq!(confirm.step, HandshakeStep::Confirm);
        assert!(bob.is_completed());

        // Alice reçoit Confirm → terminé
        let done = alice.process_message(&confirm).unwrap();
        assert!(done.is_none());
        assert!(alice.is_completed());
    }

    #[test] fn test_bad_response() {
        let mut bob = HandshakeManager::new(make_addr(2), 3, 30);

        // Bob reçoit Hello
        let hello = HandshakeMessage::new(HandshakeStep::Hello, make_addr(1), make_addr(2));
        let _challenge = bob.process_message(&hello).unwrap().unwrap();

        // Envoyer une mauvaise réponse à Bob
        let mut fake_response = HandshakeMessage::new(HandshakeStep::Response, make_addr(1), make_addr(2));
        fake_response.response = Some([0u8; 32]);

        assert!(bob.process_message(&fake_response).is_err());
        assert_eq!(bob.get_state(), &HandshakeState::Failed);
    }

    #[test] fn test_reset() {
        let mut hm = HandshakeManager::new(make_addr(1), 3, 30);
        hm.initiate(make_addr(2)).unwrap();
        assert!(hm.is_active());
        hm.reset();
        assert_eq!(hm.get_state(), &HandshakeState::Idle);
        assert!(!hm.is_active());
    }

    #[test] fn test_stats() {
        let hm = HandshakeManager::new(make_addr(1), 3, 30);
        let stats = hm.get_stats();
        assert_eq!(stats.completed, 0);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.attempts, 0);
    }

    #[test] fn test_get_peer() {
        let mut hm = HandshakeManager::new(make_addr(1), 3, 30);
        hm.initiate(make_addr(2)).unwrap();
        assert_eq!(hm.get_peer().unwrap(), &make_addr(2));
    }

    #[test] fn test_initiate_after_reset() {
        let mut hm = HandshakeManager::new(make_addr(1), 3, 30);
        hm.initiate(make_addr(2)).unwrap();
        hm.reset();
        // Doit pouvoir réinitier après reset
        assert!(hm.initiate(make_addr(3)).is_ok());
    }
}

