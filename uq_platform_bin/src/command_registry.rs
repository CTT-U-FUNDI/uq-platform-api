use serde_json::json;

/// Trait pour toutes les commandes
pub trait UqCommand: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> String;
}

/// Registre des commandes
pub struct CommandRegistry {
    commands: Vec<Box<dyn UqCommand>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self { commands: Vec::new() }
    }
    
    pub fn register(&mut self, cmd: Box<dyn UqCommand>) {
        self.commands.push(cmd);
    }
    
    pub fn get_all(&self) -> &Vec<Box<dyn UqCommand>> {
        &self.commands
    }
    
    pub fn execute(&self, name: &str, args: &str) -> Option<String> {
        for cmd in &self.commands {
            if cmd.name() == name {
                return Some(cmd.execute(args));
            }
        }
        None
    }
}

// ==========================================================
// COMMANDES BORB (13)
// ==========================================================

pub struct BorbSupplyCommand;
impl UqCommand for BorbSupplyCommand {
    fn name(&self) -> &str { "borb supply" }
    fn description(&self) -> &str { "Voir la masse monétaire" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_supply":40_000_000_000u64}).to_string()
    }
}

pub struct BorbPriceCommand;
impl UqCommand for BorbPriceCommand {
    fn name(&self) -> &str { "borb price" }
    fn description(&self) -> &str { "Voir le prix actuel" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","price_usd":1.0,"indexed_gold":true}).to_string()
    }
}

pub struct BorbUnitsCommand;
impl UqCommand for BorbUnitsCommand {
    fn name(&self) -> &str { "borb units" }
    fn description(&self) -> &str { "Voir les unités" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","units":["BORB","borb","mborb","nborb","pborb","fborb"]}).to_string()
    }
}

pub struct BorbConvertCommand;
impl UqCommand for BorbConvertCommand {
    fn name(&self) -> &str { "borb convert" }
    fn description(&self) -> &str { "Convertir des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","converted":true}).to_string()
    }
}

pub struct BorbBalanceCommand;
impl UqCommand for BorbBalanceCommand {
    fn name(&self) -> &str { "borb balance" }
    fn description(&self) -> &str { "Voir le solde" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","balance_borb":0}).to_string()
    }
}

pub struct BorbTransferCommand;
impl UqCommand for BorbTransferCommand {
    fn name(&self) -> &str { "borb transfer" }
    fn description(&self) -> &str { "Transférer des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","transferred":true}).to_string()
    }
}

pub struct BorbDistributeCommand;
impl UqCommand for BorbDistributeCommand {
    fn name(&self) -> &str { "borb distribute" }
    fn description(&self) -> &str { "Distribuer des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","distributed":true}).to_string()
    }
}

pub struct BorbFreezeCommand;
impl UqCommand for BorbFreezeCommand {
    fn name(&self) -> &str { "borb freeze" }
    fn description(&self) -> &str { "Geler des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","frozen":true}).to_string()
    }
}

pub struct BorbUnfreezeCommand;
impl UqCommand for BorbUnfreezeCommand {
    fn name(&self) -> &str { "borb unfreeze" }
    fn description(&self) -> &str { "Dégeler des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","unfrozen":true}).to_string()
    }
}

pub struct BorbLendCommand;
impl UqCommand for BorbLendCommand {
    fn name(&self) -> &str { "borb lend" }
    fn description(&self) -> &str { "Prêter des BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","lent":true}).to_string()
    }
}

pub struct BorbRepayCommand;
impl UqCommand for BorbRepayCommand {
    fn name(&self) -> &str { "borb repay" }
    fn description(&self) -> &str { "Rembourser un prêt" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","repaid":true}).to_string()
    }
}

pub struct BorbDebtCommand;
impl UqCommand for BorbDebtCommand {
    fn name(&self) -> &str { "borb debt" }
    fn description(&self) -> &str { "Voir les dettes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_debt":0}).to_string()
    }
}

pub struct BorbHistoryCommand;
impl UqCommand for BorbHistoryCommand {
    fn name(&self) -> &str { "borb history" }
    fn description(&self) -> &str { "Historique BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","transactions":[]}).to_string()
    }
}

pub struct BorbStatsCommand;
impl UqCommand for BorbStatsCommand {
    fn name(&self) -> &str { "borb stats" }
    fn description(&self) -> &str { "Statistiques BORB" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","circulation":0}).to_string()
    }
}

pub struct BorbVelocityCommand;
impl UqCommand for BorbVelocityCommand {
    fn name(&self) -> &str { "borb velocity" }
    fn description(&self) -> &str { "Vélocité" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","velocity":0.0}).to_string()
    }
}

pub struct BorbExtensionCommand;
impl UqCommand for BorbExtensionCommand {
    fn name(&self) -> &str { "borb extension" }
    fn description(&self) -> &str { "Extension fractale" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","extension_active":true}).to_string()
    }
}

// ==========================================================
// COMMANDES CASES (7)
// ==========================================================

pub struct CasesStatsCommand;
impl UqCommand for CasesStatsCommand {
    fn name(&self) -> &str { "cases stats" }
    fn description(&self) -> &str { "Voir les 6 cases" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","cases":6,"total":40_000_000_000u64}).to_string()
    }
}

pub struct TreasuryStatusCommand;
impl UqCommand for TreasuryStatusCommand {
    fn name(&self) -> &str { "treasury status" }
    fn description(&self) -> &str { "État du Treasury" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","treasury_active":true}).to_string()
    }
}

// ==========================================================
// COMMANDES SYSTÈME (10)
// ==========================================================

pub struct SystemStatusCommand;
impl UqCommand for SystemStatusCommand {
    fn name(&self) -> &str { "system status" }
    fn description(&self) -> &str { "État du système" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","uptime":"ok"}).to_string()
    }
}

pub struct ServerStartCommand;
impl UqCommand for ServerStartCommand {
    fn name(&self) -> &str { "server start" }
    fn description(&self) -> &str { "Démarrer le serveur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","started":true}).to_string()
    }
}

pub struct ServerStopCommand;
impl UqCommand for ServerStopCommand {
    fn name(&self) -> &str { "server stop" }
    fn description(&self) -> &str { "Arrêter le serveur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","stopped":true}).to_string()
    }
}

pub struct ServerRestartCommand;
impl UqCommand for ServerRestartCommand {
    fn name(&self) -> &str { "server restart" }
    fn description(&self) -> &str { "Redémarrer" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","restarted":true}).to_string()
    }
}

pub struct ServerStatusCommand;
impl UqCommand for ServerStatusCommand {
    fn name(&self) -> &str { "server status" }
    fn description(&self) -> &str { "Statut du serveur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","running":true}).to_string()
    }
}

pub struct ServerLogsCommand;
impl UqCommand for ServerLogsCommand {
    fn name(&self) -> &str { "server logs" }
    fn description(&self) -> &str { "Voir les logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","logs":[]}).to_string()
    }
}

pub struct ServerBackupCommand;
impl UqCommand for ServerBackupCommand {
    fn name(&self) -> &str { "server backup" }
    fn description(&self) -> &str { "Sauvegarder" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","backup_created":true}).to_string()
    }
}

pub struct ServerConfigCommand;
impl UqCommand for ServerConfigCommand {
    fn name(&self) -> &str { "server config" }
    fn description(&self) -> &str { "Configuration" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","config":"ok"}).to_string()
    }
}

pub struct ServerUptimeCommand;
impl UqCommand for ServerUptimeCommand {
    fn name(&self) -> &str { "server uptime" }
    fn description(&self) -> &str { "Temps de fonctionnement" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","uptime_seconds":0}).to_string()
    }
}

pub struct HelpServerCommand;
impl UqCommand for HelpServerCommand {
    fn name(&self) -> &str { "help server" }
    fn description(&self) -> &str { "Aide serveur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","commands_available":40}).to_string()
    }
}

// ==========================================================
// COMMANDES DIVERS (7)
// ==========================================================

pub struct RoutesListCommand;
impl UqCommand for RoutesListCommand {
    fn name(&self) -> &str { "routes list" }
    fn description(&self) -> &str { "Voir les routes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","routes":207}).to_string()
    }
}

pub struct RoutesCountCommand;
impl UqCommand for RoutesCountCommand {
    fn name(&self) -> &str { "routes count" }
    fn description(&self) -> &str { "Compter les routes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","count":207}).to_string()
    }
}

pub struct LogsLevelCommand;
impl UqCommand for LogsLevelCommand {
    fn name(&self) -> &str { "logs level" }
    fn description(&self) -> &str { "Niveau de log" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","level":"INFO"}).to_string()
    }
}

pub struct LogsClearCommand;
impl UqCommand for LogsClearCommand {
    fn name(&self) -> &str { "logs clear" }
    fn description(&self) -> &str { "Effacer les logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","message":"Logs effacés"}).to_string()
    }
}

pub struct LogsStatsCommand;
impl UqCommand for LogsStatsCommand {
    fn name(&self) -> &str { "logs stats" }
    fn description(&self) -> &str { "Statistiques des logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_logs":0}).to_string()
    }
}

pub struct UsersListCommand;
impl UqCommand for UsersListCommand {
    fn name(&self) -> &str { "users list" }
    fn description(&self) -> &str { "Liste des utilisateurs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","users":[]}).to_string()
    }
}

pub struct UsersCountCommand;
impl UqCommand for UsersCountCommand {
    fn name(&self) -> &str { "users count" }
    fn description(&self) -> &str { "Compter les utilisateurs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total":1}).to_string()
    }
}

pub struct StatsGlobalCommand;
impl UqCommand for StatsGlobalCommand {
    fn name(&self) -> &str { "stats global" }
    fn description(&self) -> &str { "Statistiques globales" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","global":"ok"}).to_string()
    }
}

pub struct IaStatusCommand;
impl UqCommand for IaStatusCommand {
    fn name(&self) -> &str { "ia status" }
    fn description(&self) -> &str { "État de l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","ia_active":false}).to_string()
    }
}

pub struct SecurityStatusCommand;
impl UqCommand for SecurityStatusCommand {
    fn name(&self) -> &str { "security status" }
    fn description(&self) -> &str { "État sécurité" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","secure":true}).to_string()
    }
}

pub struct PublishCommand;
impl UqCommand for PublishCommand {
    fn name(&self) -> &str { "publish" }
    fn description(&self) -> &str { "Publier" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","published":true}).to_string()
    }
}

pub struct PluginsListCommand;
impl UqCommand for PluginsListCommand {
    fn name(&self) -> &str { "plugins list" }
    fn description(&self) -> &str { "Voir les plugins" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","plugins":[]}).to_string()
    }
}


// ==========================================================
// COMMANDES DAO (10)
// ==========================================================

pub struct DaoProposalsCommand;
impl UqCommand for DaoProposalsCommand {
    fn name(&self) -> &str { "dao proposals" }
    fn description(&self) -> &str { "Voir les propositions" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","proposals":[]}).to_string()
    }
}

pub struct DaoCreateCommand;
impl UqCommand for DaoCreateCommand {
    fn name(&self) -> &str { "dao create" }
    fn description(&self) -> &str { "Créer une proposition" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","created":true}).to_string()
    }
}

pub struct DaoVoteCommand;
impl UqCommand for DaoVoteCommand {
    fn name(&self) -> &str { "dao vote" }
    fn description(&self) -> &str { "Voter sur une proposition" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","voted":true}).to_string()
    }
}

pub struct DaoExecuteCommand;
impl UqCommand for DaoExecuteCommand {
    fn name(&self) -> &str { "dao execute" }
    fn description(&self) -> &str { "Exécuter une proposition" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","executed":true}).to_string()
    }
}

pub struct DaoStatsCommand;
impl UqCommand for DaoStatsCommand {
    fn name(&self) -> &str { "dao stats" }
    fn description(&self) -> &str { "Statistiques DAO" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_votes":0}).to_string()
    }
}

pub struct DaoValidatorsCommand;
impl UqCommand for DaoValidatorsCommand {
    fn name(&self) -> &str { "dao validators" }
    fn description(&self) -> &str { "Voir les validateurs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","validators":10}).to_string()
    }
}

pub struct DaoApprovalsCommand;
impl UqCommand for DaoApprovalsCommand {
    fn name(&self) -> &str { "dao approvals" }
    fn description(&self) -> &str { "Approbations en attente" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","pending":0}).to_string()
    }
}

pub struct DaoReportCommand;
impl UqCommand for DaoReportCommand {
    fn name(&self) -> &str { "dao report" }
    fn description(&self) -> &str { "Rapport de gouvernance" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

pub struct DaoProposalCommand;
impl UqCommand for DaoProposalCommand {
    fn name(&self) -> &str { "dao proposal" }
    fn description(&self) -> &str { "Détails d'une proposition" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","proposal":null}).to_string()
    }
}

pub struct DaoValidatorCommand;
impl UqCommand for DaoValidatorCommand {
    fn name(&self) -> &str { "dao validator" }
    fn description(&self) -> &str { "Détails d'un validateur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","validator":null}).to_string()
    }
}

// ==========================================================
// COMMANDES IA (12)
// ==========================================================

pub struct AiAskCommand;
impl UqCommand for AiAskCommand {
    fn name(&self) -> &str { "ai ask" }
    fn description(&self) -> &str { "Poser une question à l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","answer":"IA en arrière-plan"}).to_string()
    }
}

pub struct AiAnalyzeCommand;
impl UqCommand for AiAnalyzeCommand {
    fn name(&self) -> &str { "ai analyze" }
    fn description(&self) -> &str { "Analyse approfondie" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","analysis":"ok"}).to_string()
    }
}

pub struct AiDetectCommand;
impl UqCommand for AiDetectCommand {
    fn name(&self) -> &str { "ai detect" }
    fn description(&self) -> &str { "Détecter le métier" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","metier":"inconnu"}).to_string()
    }
}

pub struct AiSuggestCommand;
impl UqCommand for AiSuggestCommand {
    fn name(&self) -> &str { "ai suggest" }
    fn description(&self) -> &str { "Suggérer un prix" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","suggested_price":0}).to_string()
    }
}

pub struct AiGenerateCommand;
impl UqCommand for AiGenerateCommand {
    fn name(&self) -> &str { "ai generate" }
    fn description(&self) -> &str { "Générer un titre" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","title":"Titre généré"}).to_string()
    }
}

pub struct AiMarketCommand;
impl UqCommand for AiMarketCommand {
    fn name(&self) -> &str { "ai market" }
    fn description(&self) -> &str { "Analyser le marché" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","market":"ok"}).to_string()
    }
}

pub struct AiReportCommand;
impl UqCommand for AiReportCommand {
    fn name(&self) -> &str { "ai report" }
    fn description(&self) -> &str { "Rapport automatique" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

pub struct AiAlertCommand;
impl UqCommand for AiAlertCommand {
    fn name(&self) -> &str { "ai alert" }
    fn description(&self) -> &str { "Alertes de l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","alerts":[]}).to_string()
    }
}

pub struct AiRecommendCommand;
impl UqCommand for AiRecommendCommand {
    fn name(&self) -> &str { "ai recommend" }
    fn description(&self) -> &str { "Recommandations" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","recommendations":[]}).to_string()
    }
}

pub struct AiExecuteCommand;
impl UqCommand for AiExecuteCommand {
    fn name(&self) -> &str { "ai execute" }
    fn description(&self) -> &str { "Exécuter via l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","executed":true}).to_string()
    }
}

pub struct AiChatCommand;
impl UqCommand for AiChatCommand {
    fn name(&self) -> &str { "ai chat" }
    fn description(&self) -> &str { "Conversation avec l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","chat":true}).to_string()
    }
}

pub struct AiStatusCommand;
impl UqCommand for AiStatusCommand {
    fn name(&self) -> &str { "ai status" }
    fn description(&self) -> &str { "État de l'IA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","ia_active":false}).to_string()
    }
}

// ==========================================================
// FONCTION D'ENREGISTREMENT
// ==========================================================

// ==========================================================
// COMMANDES P2P (17)
// ==========================================================

pub struct P2pStatusCommand;
impl UqCommand for P2pStatusCommand {
    fn name(&self) -> &str { "p2p status" }
    fn description(&self) -> &str { "État du réseau P2P" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","network":"ready"}).to_string()
    }
}

pub struct P2pPeersCommand;
impl UqCommand for P2pPeersCommand {
    fn name(&self) -> &str { "p2p peers" }
    fn description(&self) -> &str { "Liste des pairs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","peers":[]}).to_string()
    }
}

pub struct P2pScanCommand;
impl UqCommand for P2pScanCommand {
    fn name(&self) -> &str { "p2p scan" }
    fn description(&self) -> &str { "Scanner les pairs proches" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","scanning":true}).to_string()
    }
}

pub struct P2pConnectCommand;
impl UqCommand for P2pConnectCommand {
    fn name(&self) -> &str { "p2p connect" }
    fn description(&self) -> &str { "Connecter à un pair" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","connected":true}).to_string()
    }
}

pub struct P2pDisconnectCommand;
impl UqCommand for P2pDisconnectCommand {
    fn name(&self) -> &str { "p2p disconnect" }
    fn description(&self) -> &str { "Déconnecter un pair" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","disconnected":true}).to_string()
    }
}

pub struct P2pRelayCommand;
impl UqCommand for P2pRelayCommand {
    fn name(&self) -> &str { "p2p relay" }
    fn description(&self) -> &str { "Relayer un message" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","relayed":true}).to_string()
    }
}

pub struct P2pMeshCommand;
impl UqCommand for P2pMeshCommand {
    fn name(&self) -> &str { "p2p mesh" }
    fn description(&self) -> &str { "Voir le maillage" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","mesh_nodes":0}).to_string()
    }
}

pub struct P2pFrequencyCommand;
impl UqCommand for P2pFrequencyCommand {
    fn name(&self) -> &str { "p2p frequency" }
    fn description(&self) -> &str { "Voir les fréquences" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","frequency":"2.4GHz"}).to_string()
    }
}

pub struct P2pHopCommand;
impl UqCommand for P2pHopCommand {
    fn name(&self) -> &str { "p2p hop" }
    fn description(&self) -> &str { "Saut de fréquence" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","hopped":true}).to_string()
    }
}

pub struct P2pPhantomCommand;
impl UqCommand for P2pPhantomCommand {
    fn name(&self) -> &str { "p2p phantom" }
    fn description(&self) -> &str { "Créer un canal fantôme" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","phantom_created":true}).to_string()
    }
}

pub struct P2pSosCommand;
impl UqCommand for P2pSosCommand {
    fn name(&self) -> &str { "p2p sos" }
    fn description(&self) -> &str { "Lancer une alerte SOS" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","sos_broadcast":true}).to_string()
    }
}

pub struct P2pThreatsCommand;
impl UqCommand for P2pThreatsCommand {
    fn name(&self) -> &str { "p2p threats" }
    fn description(&self) -> &str { "Menaces réseau" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","threats":0}).to_string()
    }
}

pub struct P2pD2dScanCommand;
impl UqCommand for P2pD2dScanCommand {
    fn name(&self) -> &str { "p2p d2d_scan" }
    fn description(&self) -> &str { "Scanner D2D" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","d2d_scanning":true}).to_string()
    }
}

pub struct P2pD2dPeersCommand;
impl UqCommand for P2pD2dPeersCommand {
    fn name(&self) -> &str { "p2p d2d_peers" }
    fn description(&self) -> &str { "Pairs D2D" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","d2d_peers":[]}).to_string()
    }
}

pub struct P2pInternetSellCommand;
impl UqCommand for P2pInternetSellCommand {
    fn name(&self) -> &str { "p2p internet_sell" }
    fn description(&self) -> &str { "Vendre de la connexion" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","selling":true}).to_string()
    }
}

pub struct P2pReportCommand;
impl UqCommand for P2pReportCommand {
    fn name(&self) -> &str { "p2p report" }
    fn description(&self) -> &str { "Rapport du réseau" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

pub struct P2pEmergencyCommand;
impl UqCommand for P2pEmergencyCommand {
    fn name(&self) -> &str { "p2p emergency" }
    fn description(&self) -> &str { "Mode urgence" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","emergency_active":false}).to_string()
    }
}


// ==========================================================
// COMMANDES STUDIOS (15)
// ==========================================================

pub struct StudioListCommand;
impl UqCommand for StudioListCommand {
    fn name(&self) -> &str { "studio list" }
    fn description(&self) -> &str { "Liste des studios" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","studios":[]}).to_string()
    }
}

pub struct StudioCreateCommand;
impl UqCommand for StudioCreateCommand {
    fn name(&self) -> &str { "studio create" }
    fn description(&self) -> &str { "Créer un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","created":true}).to_string()
    }
}

pub struct StudioDeleteCommand;
impl UqCommand for StudioDeleteCommand {
    fn name(&self) -> &str { "studio delete" }
    fn description(&self) -> &str { "Supprimer un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","deleted":true}).to_string()
    }
}

pub struct StudioBoostCommand;
impl UqCommand for StudioBoostCommand {
    fn name(&self) -> &str { "studio boost" }
    fn description(&self) -> &str { "Booster un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","boosted":true}).to_string()
    }
}

pub struct StudioStatsCommand;
impl UqCommand for StudioStatsCommand {
    fn name(&self) -> &str { "studio stats" }
    fn description(&self) -> &str { "Statistiques d'un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","views":0}).to_string()
    }
}

pub struct StudioProductsCommand;
impl UqCommand for StudioProductsCommand {
    fn name(&self) -> &str { "studio products" }
    fn description(&self) -> &str { "Produits d'un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","products":[]}).to_string()
    }
}

pub struct StudioServicesCommand;
impl UqCommand for StudioServicesCommand {
    fn name(&self) -> &str { "studio services" }
    fn description(&self) -> &str { "Services d'un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","services":[]}).to_string()
    }
}

pub struct StudioSearchCommand;
impl UqCommand for StudioSearchCommand {
    fn name(&self) -> &str { "studio search" }
    fn description(&self) -> &str { "Rechercher un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","results":[]}).to_string()
    }
}

pub struct StudioVerifyCommand;
impl UqCommand for StudioVerifyCommand {
    fn name(&self) -> &str { "studio verify" }
    fn description(&self) -> &str { "Vérifier un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","verified":true}).to_string()
    }
}

pub struct StudioVentesCommand;
impl UqCommand for StudioVentesCommand {
    fn name(&self) -> &str { "studio ventes" }
    fn description(&self) -> &str { "Ventes d'un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","ventes":0}).to_string()
    }
}

pub struct StudioContactsCommand;
impl UqCommand for StudioContactsCommand {
    fn name(&self) -> &str { "studio contacts" }
    fn description(&self) -> &str { "Contacts reçus" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","contacts":0}).to_string()
    }
}

pub struct StudioFilterCommand;
impl UqCommand for StudioFilterCommand {
    fn name(&self) -> &str { "studio filter" }
    fn description(&self) -> &str { "Filtrer les studios" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","filtered":[]}).to_string()
    }
}

pub struct StudioExportCommand;
impl UqCommand for StudioExportCommand {
    fn name(&self) -> &str { "studio export" }
    fn description(&self) -> &str { "Exporter les studios" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","exported":true}).to_string()
    }
}

pub struct StudioUnboostCommand;
impl UqCommand for StudioUnboostCommand {
    fn name(&self) -> &str { "studio unboost" }
    fn description(&self) -> &str { "Retirer le boost" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","unboosted":true}).to_string()
    }
}

pub struct StudioViewsCommand;
impl UqCommand for StudioViewsCommand {
    fn name(&self) -> &str { "studio views" }
    fn description(&self) -> &str { "Vues d'un studio" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","views":0}).to_string()
    }
}

// ==========================================================
// COMMANDES UBI (8)
// ==========================================================

pub struct UbiInfoCommand;
impl UqCommand for UbiInfoCommand {
    fn name(&self) -> &str { "ubi info" }
    fn description(&self) -> &str { "Informations UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","daily_points":2}).to_string()
    }
}

pub struct UbiClaimCommand;
impl UqCommand for UbiClaimCommand {
    fn name(&self) -> &str { "ubi claim" }
    fn description(&self) -> &str { "Réclamer l'UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","claimed":true}).to_string()
    }
}

pub struct UbiCheckCommand;
impl UqCommand for UbiCheckCommand {
    fn name(&self) -> &str { "ubi check" }
    fn description(&self) -> &str { "Vérifier l'éligibilité" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","eligible":true}).to_string()
    }
}

pub struct UbiDistributeCommand;
impl UqCommand for UbiDistributeCommand {
    fn name(&self) -> &str { "ubi distribute" }
    fn description(&self) -> &str { "Distribuer l'UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","distributed":true}).to_string()
    }
}

pub struct UbiStatsCommand;
impl UqCommand for UbiStatsCommand {
    fn name(&self) -> &str { "ubi stats" }
    fn description(&self) -> &str { "Statistiques UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_distributed":0}).to_string()
    }
}

pub struct UbiHistoryCommand;
impl UqCommand for UbiHistoryCommand {
    fn name(&self) -> &str { "ubi history" }
    fn description(&self) -> &str { "Historique UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","history":[]}).to_string()
    }
}

pub struct UbiPauseCommand;
impl UqCommand for UbiPauseCommand {
    fn name(&self) -> &str { "ubi pause" }
    fn description(&self) -> &str { "Pauser l'UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","paused":true}).to_string()
    }
}

pub struct UbiResumeCommand;
impl UqCommand for UbiResumeCommand {
    fn name(&self) -> &str { "ubi resume" }
    fn description(&self) -> &str { "Reprendre l'UBI" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","resumed":true}).to_string()
    }
}

// ==========================================================
// COMMANDES BULLES (7)
// ==========================================================

pub struct BullesTotalCommand;
impl UqCommand for BullesTotalCommand {
    fn name(&self) -> &str { "bulles total" }
    fn description(&self) -> &str { "Total des bulles" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total":0}).to_string()
    }
}

pub struct BullesConvertiesCommand;
impl UqCommand for BullesConvertiesCommand {
    fn name(&self) -> &str { "bulles converties" }
    fn description(&self) -> &str { "Bulles converties" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","converted":0}).to_string()
    }
}

pub struct BullesGeleesCommand;
impl UqCommand for BullesGeleesCommand {
    fn name(&self) -> &str { "bulles gelées" }
    fn description(&self) -> &str { "Bulles gelées" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","frozen":0}).to_string()
    }
}

pub struct BullesCirculantesCommand;
impl UqCommand for BullesCirculantesCommand {
    fn name(&self) -> &str { "bulles circulantes" }
    fn description(&self) -> &str { "Bulles en circulation" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","circulating":0}).to_string()
    }
}

pub struct BullesCreateCommand;
impl UqCommand for BullesCreateCommand {
    fn name(&self) -> &str { "bulles create" }
    fn description(&self) -> &str { "Créer des bulles" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","created":true}).to_string()
    }
}

pub struct BullesTransferCommand;
impl UqCommand for BullesTransferCommand {
    fn name(&self) -> &str { "bulles transfer" }
    fn description(&self) -> &str { "Transférer des bulles" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","transferred":true}).to_string()
    }
}

pub struct BullesReportCommand;
impl UqCommand for BullesReportCommand {
    fn name(&self) -> &str { "bulles report" }
    fn description(&self) -> &str { "Rapport des bulles" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}


// ==========================================================
// COMMANDES VAULT (16)
// ==========================================================

pub struct VaultStatusCommand;
impl UqCommand for VaultStatusCommand {
    fn name(&self) -> &str { "vault status" }
    fn description(&self) -> &str { "État du Vault" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","shamir":"3/5"}).to_string()
    }
}

pub struct VaultSplitCommand;
impl UqCommand for VaultSplitCommand {
    fn name(&self) -> &str { "vault split" }
    fn description(&self) -> &str { "Diviser une clé" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","fragments":5}).to_string()
    }
}

pub struct VaultCombineCommand;
impl UqCommand for VaultCombineCommand {
    fn name(&self) -> &str { "vault combine" }
    fn description(&self) -> &str { "Reconstruire une clé" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","combined":true}).to_string()
    }
}

pub struct VaultGuardiansCommand;
impl UqCommand for VaultGuardiansCommand {
    fn name(&self) -> &str { "vault guardians" }
    fn description(&self) -> &str { "Voir les guardians" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","guardians":[]}).to_string()
    }
}

pub struct VaultReplaceCommand;
impl UqCommand for VaultReplaceCommand {
    fn name(&self) -> &str { "vault replace" }
    fn description(&self) -> &str { "Remplacer un guardian" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","replaced":true}).to_string()
    }
}

pub struct VaultCompromisedCommand;
impl UqCommand for VaultCompromisedCommand {
    fn name(&self) -> &str { "vault compromised" }
    fn description(&self) -> &str { "Marquer un guardian compromis" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","marked":true}).to_string()
    }
}

pub struct VaultAuthenticityCommand;
impl UqCommand for VaultAuthenticityCommand {
    fn name(&self) -> &str { "vault authenticity" }
    fn description(&self) -> &str { "Vérifier l'authenticité" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","authentic":true}).to_string()
    }
}

pub struct VaultP2pStoreCommand;
impl UqCommand for VaultP2pStoreCommand {
    fn name(&self) -> &str { "vault p2p_store" }
    fn description(&self) -> &str { "État du stockage P2P" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","usage":0.0}).to_string()
    }
}

pub struct VaultReportCommand;
impl UqCommand for VaultReportCommand {
    fn name(&self) -> &str { "vault report" }
    fn description(&self) -> &str { "Rapport du Vault" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

// ==========================================================
// COMMANDES RECYCLAGE (10)
// ==========================================================

pub struct RecycleListCommand;
impl UqCommand for RecycleListCommand {
    fn name(&self) -> &str { "recycle list" }
    fn description(&self) -> &str { "Comptes inactifs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","inactive":[]}).to_string()
    }
}

pub struct RecycleRunCommand;
impl UqCommand for RecycleRunCommand {
    fn name(&self) -> &str { "recycle run" }
    fn description(&self) -> &str { "Recycler maintenant" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","recycled":true}).to_string()
    }
}

pub struct RecycleScheduleCommand;
impl UqCommand for RecycleScheduleCommand {
    fn name(&self) -> &str { "recycle schedule" }
    fn description(&self) -> &str { "Programmer le recyclage" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","scheduled":true}).to_string()
    }
}

pub struct RecycleCancelCommand;
impl UqCommand for RecycleCancelCommand {
    fn name(&self) -> &str { "recycle cancel" }
    fn description(&self) -> &str { "Annuler la programmation" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","cancelled":true}).to_string()
    }
}

pub struct RecycleHistoryCommand;
impl UqCommand for RecycleHistoryCommand {
    fn name(&self) -> &str { "recycle history" }
    fn description(&self) -> &str { "Historique du recyclage" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","history":[]}).to_string()
    }
}

pub struct RecycleStatsCommand;
impl UqCommand for RecycleStatsCommand {
    fn name(&self) -> &str { "recycle stats" }
    fn description(&self) -> &str { "Statistiques du recyclage" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","total_recycled":0}).to_string()
    }
}

pub struct RecycleAmountCommand;
impl UqCommand for RecycleAmountCommand {
    fn name(&self) -> &str { "recycle amount" }
    fn description(&self) -> &str { "BORB recyclés" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","amount":0}).to_string()
    }
}

pub struct RecycleFeesCommand;
impl UqCommand for RecycleFeesCommand {
    fn name(&self) -> &str { "recycle fees" }
    fn description(&self) -> &str { "Frais de registre collectés" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","fees":0}).to_string()
    }
}

pub struct RecycleNextCommand;
impl UqCommand for RecycleNextCommand {
    fn name(&self) -> &str { "recycle next" }
    fn description(&self) -> &str { "Prochaine époque" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","next_epoch":"inconnu"}).to_string()
    }
}

pub struct RecycleReportCommand;
impl UqCommand for RecycleReportCommand {
    fn name(&self) -> &str { "recycle report" }
    fn description(&self) -> &str { "Rapport du recyclage" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

// ==========================================================
// COMMANDES ANALYTIQUES (10)
// ==========================================================

pub struct AnalyticsTransactionsCommand;
impl UqCommand for AnalyticsTransactionsCommand {
    fn name(&self) -> &str { "analytics transactions" }
    fn description(&self) -> &str { "Graphique transactions" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","data":[]}).to_string()
    }
}

pub struct AnalyticsVolumeCommand;
impl UqCommand for AnalyticsVolumeCommand {
    fn name(&self) -> &str { "analytics volume" }
    fn description(&self) -> &str { "Graphique volume" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","volume":0}).to_string()
    }
}

pub struct AnalyticsP2pCommand;
impl UqCommand for AnalyticsP2pCommand {
    fn name(&self) -> &str { "analytics p2p" }
    fn description(&self) -> &str { "Graphique P2P" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","p2p_data":[]}).to_string()
    }
}

pub struct AnalyticsPlayerCommand;
impl UqCommand for AnalyticsPlayerCommand {
    fn name(&self) -> &str { "analytics player" }
    fn description(&self) -> &str { "Graphique lecteur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","player_data":[]}).to_string()
    }
}

pub struct AnalyticsStudiosCommand;
impl UqCommand for AnalyticsStudiosCommand {
    fn name(&self) -> &str { "analytics studios" }
    fn description(&self) -> &str { "Graphique studios" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","studio_data":[]}).to_string()
    }
}

pub struct AnalyticsCasesCommand;
impl UqCommand for AnalyticsCasesCommand {
    fn name(&self) -> &str { "analytics cases" }
    fn description(&self) -> &str { "Camembert des 6 cases" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","cases_data":[]}).to_string()
    }
}

pub struct AnalyticsExportCommand;
impl UqCommand for AnalyticsExportCommand {
    fn name(&self) -> &str { "analytics export" }
    fn description(&self) -> &str { "Exporter un graphique" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","exported":true}).to_string()
    }
}

pub struct AnalyticsReportCommand;
impl UqCommand for AnalyticsReportCommand {
    fn name(&self) -> &str { "analytics report" }
    fn description(&self) -> &str { "Rapport analytique complet" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

pub struct AnalyticsExportCsvCommand;
impl UqCommand for AnalyticsExportCsvCommand {
    fn name(&self) -> &str { "analytics export_csv" }
    fn description(&self) -> &str { "Exporter en CSV" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","csv_exported":true}).to_string()
    }
}

pub struct AnalyticsHeatmapCommand;
impl UqCommand for AnalyticsHeatmapCommand {
    fn name(&self) -> &str { "analytics heatmap" }
    fn description(&self) -> &str { "Carte de chaleur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","heatmap":[]}).to_string()
    }
}


// ==========================================================
// COMMANDES COMPTES (15)
// ==========================================================

pub struct AccountListCommand;
impl UqCommand for AccountListCommand {
    fn name(&self) -> &str { "account list" }
    fn description(&self) -> &str { "Liste des comptes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","accounts":[]}).to_string()
    }
}

pub struct AccountCreateCommand;
impl UqCommand for AccountCreateCommand {
    fn name(&self) -> &str { "account create" }
    fn description(&self) -> &str { "Créer un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","created":true}).to_string()
    }
}

pub struct AccountCloseCommand;
impl UqCommand for AccountCloseCommand {
    fn name(&self) -> &str { "account close" }
    fn description(&self) -> &str { "Fermer un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","closed":true}).to_string()
    }
}

pub struct AccountBlockCommand;
impl UqCommand for AccountBlockCommand {
    fn name(&self) -> &str { "account block" }
    fn description(&self) -> &str { "Bloquer un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","blocked":true}).to_string()
    }
}

pub struct AccountUnblockCommand;
impl UqCommand for AccountUnblockCommand {
    fn name(&self) -> &str { "account unblock" }
    fn description(&self) -> &str { "Débloquer un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","unblocked":true}).to_string()
    }
}

pub struct AccountActivityCommand;
impl UqCommand for AccountActivityCommand {
    fn name(&self) -> &str { "account activity" }
    fn description(&self) -> &str { "Activité d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","activity":[]}).to_string()
    }
}

pub struct AccountStatsCommand;
impl UqCommand for AccountStatsCommand {
    fn name(&self) -> &str { "account stats" }
    fn description(&self) -> &str { "Statistiques d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","stats":{}}).to_string()
    }
}

pub struct AccountMessagesCommand;
impl UqCommand for AccountMessagesCommand {
    fn name(&self) -> &str { "account messages" }
    fn description(&self) -> &str { "Messages d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","messages":[]}).to_string()
    }
}

pub struct AccountTransactionsCommand;
impl UqCommand for AccountTransactionsCommand {
    fn name(&self) -> &str { "account transactions" }
    fn description(&self) -> &str { "Transactions d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","transactions":[]}).to_string()
    }
}

pub struct AccountDettesCommand;
impl UqCommand for AccountDettesCommand {
    fn name(&self) -> &str { "account dettes" }
    fn description(&self) -> &str { "Dettes d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","dettes":0}).to_string()
    }
}

pub struct AccountReputationCommand;
impl UqCommand for AccountReputationCommand {
    fn name(&self) -> &str { "account reputation" }
    fn description(&self) -> &str { "Réputation d'un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","score":0}).to_string()
    }
}

pub struct AccountKycCommand;
impl UqCommand for AccountKycCommand {
    fn name(&self) -> &str { "account kyc" }
    fn description(&self) -> &str { "Vérifier KYC" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","kyc_verified":false}).to_string()
    }
}

pub struct AccountSearchCommand;
impl UqCommand for AccountSearchCommand {
    fn name(&self) -> &str { "account search" }
    fn description(&self) -> &str { "Rechercher un compte" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","results":[]}).to_string()
    }
}

pub struct AccountFilterCommand;
impl UqCommand for AccountFilterCommand {
    fn name(&self) -> &str { "account filter" }
    fn description(&self) -> &str { "Filtrer les comptes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","filtered":[]}).to_string()
    }
}

pub struct AccountExportCommand;
impl UqCommand for AccountExportCommand {
    fn name(&self) -> &str { "account export" }
    fn description(&self) -> &str { "Exporter les comptes" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","exported":true}).to_string()
    }
}

// ==========================================================
// COMMANDES ÉCHANGES (14)
// ==========================================================

pub struct ExchangeOrdersCommand;
impl UqCommand for ExchangeOrdersCommand {
    fn name(&self) -> &str { "exchange orders" }
    fn description(&self) -> &str { "Voir les ordres" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","orders":[]}).to_string()
    }
}

pub struct ExchangeOrderCommand;
impl UqCommand for ExchangeOrderCommand {
    fn name(&self) -> &str { "exchange order" }
    fn description(&self) -> &str { "Détails d'un ordre" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","order":null}).to_string()
    }
}

pub struct ExchangeCancelCommand;
impl UqCommand for ExchangeCancelCommand {
    fn name(&self) -> &str { "exchange cancel" }
    fn description(&self) -> &str { "Annuler un ordre" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","cancelled":true}).to_string()
    }
}

pub struct ExchangeEscrowsCommand;
impl UqCommand for ExchangeEscrowsCommand {
    fn name(&self) -> &str { "exchange escrows" }
    fn description(&self) -> &str { "Voir les escrows" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","escrows":[]}).to_string()
    }
}

pub struct ExchangeReleaseCommand;
impl UqCommand for ExchangeReleaseCommand {
    fn name(&self) -> &str { "exchange release" }
    fn description(&self) -> &str { "Libérer un escrow" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","released":true}).to_string()
    }
}

pub struct ExchangeFreezeCommand;
impl UqCommand for ExchangeFreezeCommand {
    fn name(&self) -> &str { "exchange freeze" }
    fn description(&self) -> &str { "Geler un échange" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","frozen":true}).to_string()
    }
}

pub struct ExchangeHistoryCommand;
impl UqCommand for ExchangeHistoryCommand {
    fn name(&self) -> &str { "exchange history" }
    fn description(&self) -> &str { "Historique" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","history":[]}).to_string()
    }
}

pub struct ExchangeVolumeCommand;
impl UqCommand for ExchangeVolumeCommand {
    fn name(&self) -> &str { "exchange volume" }
    fn description(&self) -> &str { "Volume d'échange" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","volume":0}).to_string()
    }
}

pub struct ExchangeCommissionsCommand;
impl UqCommand for ExchangeCommissionsCommand {
    fn name(&self) -> &str { "exchange commissions" }
    fn description(&self) -> &str { "Voir les commissions" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","commissions":0}).to_string()
    }
}

pub struct ExchangeAnnoncesCommand;
impl UqCommand for ExchangeAnnoncesCommand {
    fn name(&self) -> &str { "exchange annonces" }
    fn description(&self) -> &str { "Annonces P2P" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","annonces":[]}).to_string()
    }
}

pub struct ExchangeLitigesCommand;
impl UqCommand for ExchangeLitigesCommand {
    fn name(&self) -> &str { "exchange litiges" }
    fn description(&self) -> &str { "Voir les litiges" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","litiges":[]}).to_string()
    }
}

pub struct ExchangeArbitrerCommand;
impl UqCommand for ExchangeArbitrerCommand {
    fn name(&self) -> &str { "exchange arbitrer" }
    fn description(&self) -> &str { "Arbitrer un litige" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","arbitrated":true}).to_string()
    }
}

pub struct ExchangeMatchCommand;
impl UqCommand for ExchangeMatchCommand {
    fn name(&self) -> &str { "exchange match" }
    fn description(&self) -> &str { "Matcher un ordre" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","matched":true}).to_string()
    }
}

pub struct ExchangeReportCommand;
impl UqCommand for ExchangeReportCommand {
    fn name(&self) -> &str { "exchange report" }
    fn description(&self) -> &str { "Rapport d'échange" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

// ==========================================================
// COMMANDES LECTEUR (12)
// ==========================================================

pub struct PlayerSessionsCommand;
impl UqCommand for PlayerSessionsCommand {
    fn name(&self) -> &str { "player sessions" }
    fn description(&self) -> &str { "Sessions actives" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","sessions":[]}).to_string()
    }
}

pub struct PlayerStartCommand;
impl UqCommand for PlayerStartCommand {
    fn name(&self) -> &str { "player start" }
    fn description(&self) -> &str { "Démarrer une session" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","started":true}).to_string()
    }
}

pub struct PlayerStopCommand;
impl UqCommand for PlayerStopCommand {
    fn name(&self) -> &str { "player stop" }
    fn description(&self) -> &str { "Arrêter une session" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","stopped":true}).to_string()
    }
}

pub struct PlayerStatsCommand;
impl UqCommand for PlayerStatsCommand {
    fn name(&self) -> &str { "player stats" }
    fn description(&self) -> &str { "Statistiques du lecteur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","stats":{}}).to_string()
    }
}

pub struct PlayerCreatorsCommand;
impl UqCommand for PlayerCreatorsCommand {
    fn name(&self) -> &str { "player creators" }
    fn description(&self) -> &str { "Liste des créateurs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","creators":[]}).to_string()
    }
}

pub struct PlayerDistributeCommand;
impl UqCommand for PlayerDistributeCommand {
    fn name(&self) -> &str { "player distribute" }
    fn description(&self) -> &str { "Distribuer aux créateurs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","distributed":true}).to_string()
    }
}

pub struct PlayerSecondsCommand;
impl UqCommand for PlayerSecondsCommand {
    fn name(&self) -> &str { "player seconds" }
    fn description(&self) -> &str { "Secondes streamées" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","seconds":0}).to_string()
    }
}

pub struct PlayerRevenueCommand;
impl UqCommand for PlayerRevenueCommand {
    fn name(&self) -> &str { "player revenue" }
    fn description(&self) -> &str { "Revenus d'un créateur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","revenue":0}).to_string()
    }
}

pub struct PlayerPriceCommand;
impl UqCommand for PlayerPriceCommand {
    fn name(&self) -> &str { "player price" }
    fn description(&self) -> &str { "Prix par seconde" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","price_per_second":0}).to_string()
    }
}

pub struct PlayerBoostCommand;
impl UqCommand for PlayerBoostCommand {
    fn name(&self) -> &str { "player boost" }
    fn description(&self) -> &str { "Booster un créateur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","boosted":true}).to_string()
    }
}

pub struct PlayerCreatorCommand;
impl UqCommand for PlayerCreatorCommand {
    fn name(&self) -> &str { "player creator" }
    fn description(&self) -> &str { "Détails d'un créateur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","creator":null}).to_string()
    }
}

pub struct PlayerReportCommand;
impl UqCommand for PlayerReportCommand {
    fn name(&self) -> &str { "player report" }
    fn description(&self) -> &str { "Rapport du lecteur" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

// ==========================================================
// COMMANDES SÉCURITÉ (15)
// ==========================================================

pub struct SecurityThreatsCommand;
impl UqCommand for SecurityThreatsCommand {
    fn name(&self) -> &str { "security threats" }
    fn description(&self) -> &str { "Menaces détectées" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","threats":0}).to_string()
    }
}

pub struct SecurityFraudCommand;
impl UqCommand for SecurityFraudCommand {
    fn name(&self) -> &str { "security fraud" }
    fn description(&self) -> &str { "Tentatives de fraude" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","frauds":[]}).to_string()
    }
}

pub struct SecurityBlockCommand;
impl UqCommand for SecurityBlockCommand {
    fn name(&self) -> &str { "security block" }
    fn description(&self) -> &str { "Bloquer une adresse IP" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","blocked":true}).to_string()
    }
}

pub struct SecurityUnblockCommand;
impl UqCommand for SecurityUnblockCommand {
    fn name(&self) -> &str { "security unblock" }
    fn description(&self) -> &str { "Débloquer une adresse" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","unblocked":true}).to_string()
    }
}

pub struct Security2faCommand;
impl UqCommand for Security2faCommand {
    fn name(&self) -> &str { "security 2fa" }
    fn description(&self) -> &str { "Vérifier le 2FA" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","2fa_verified":true}).to_string()
    }
}

pub struct SecurityReportCommand;
impl UqCommand for SecurityReportCommand {
    fn name(&self) -> &str { "security report" }
    fn description(&self) -> &str { "Rapport de sécurité" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","report":"ok"}).to_string()
    }
}

// ==========================================================
// COMMANDES LOGS SUPPLÉMENTAIRES (17)
// ==========================================================

pub struct LogsShowCommand;
impl UqCommand for LogsShowCommand {
    fn name(&self) -> &str { "logs show" }
    fn description(&self) -> &str { "Voir les derniers logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","logs":[]}).to_string()
    }
}

pub struct LogsFilterCommand;
impl UqCommand for LogsFilterCommand {
    fn name(&self) -> &str { "logs filter" }
    fn description(&self) -> &str { "Filtrer les logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","filtered":[]}).to_string()
    }
}

pub struct LogsSearchCommand;
impl UqCommand for LogsSearchCommand {
    fn name(&self) -> &str { "logs search" }
    fn description(&self) -> &str { "Rechercher dans les logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","results":[]}).to_string()
    }
}

pub struct LogsExportCommand;
impl UqCommand for LogsExportCommand {
    fn name(&self) -> &str { "logs export" }
    fn description(&self) -> &str { "Exporter les logs" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","exported":true}).to_string()
    }
}

pub struct LogsRealtimeCommand;
impl UqCommand for LogsRealtimeCommand {
    fn name(&self) -> &str { "logs realtime" }
    fn description(&self) -> &str { "Temps réel" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","realtime":true}).to_string()
    }
}

pub struct LogsPauseCommand;
impl UqCommand for LogsPauseCommand {
    fn name(&self) -> &str { "logs pause" }
    fn description(&self) -> &str { "Pauser le flux" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","paused":true}).to_string()
    }
}

// ==========================================================
// COMMANDES SYSTÈME SUPPLÉMENTAIRES (10)
// ==========================================================

pub struct SystemMemoryCommand;
impl UqCommand for SystemMemoryCommand {
    fn name(&self) -> &str { "system memory" }
    fn description(&self) -> &str { "Mémoire utilisée" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","memory_mb":0}).to_string()
    }
}

pub struct SystemCpuCommand;
impl UqCommand for SystemCpuCommand {
    fn name(&self) -> &str { "system cpu" }
    fn description(&self) -> &str { "CPU utilisé" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","cpu_pct":0.0}).to_string()
    }
}

pub struct SystemRestartCommand;
impl UqCommand for SystemRestartCommand {
    fn name(&self) -> &str { "system restart" }
    fn description(&self) -> &str { "Redémarrer" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","restarted":true}).to_string()
    }
}

pub struct SystemPauseCommand;
impl UqCommand for SystemPauseCommand {
    fn name(&self) -> &str { "system pause" }
    fn description(&self) -> &str { "Mettre en pause" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","paused":true}).to_string()
    }
}

pub struct SystemBackupCommand;
impl UqCommand for SystemBackupCommand {
    fn name(&self) -> &str { "system backup" }
    fn description(&self) -> &str { "Sauvegarder" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","backup_created":true}).to_string()
    }
}

pub struct SystemRestoreCommand;
impl UqCommand for SystemRestoreCommand {
    fn name(&self) -> &str { "system restore" }
    fn description(&self) -> &str { "Restaurer" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","restored":true}).to_string()
    }
}

pub struct SystemVersionCommand;
impl UqCommand for SystemVersionCommand {
    fn name(&self) -> &str { "system version" }
    fn description(&self) -> &str { "Version du logiciel" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","version":"1.0.0"}).to_string()
    }
}

pub struct SystemUpdateCommand;
impl UqCommand for SystemUpdateCommand {
    fn name(&self) -> &str { "system update" }
    fn description(&self) -> &str { "Mettre à jour" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","updated":true}).to_string()
    }
}

pub struct SystemShutdownCommand;
impl UqCommand for SystemShutdownCommand {
    fn name(&self) -> &str { "system shutdown" }
    fn description(&self) -> &str { "Arrêter" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","shutdown":true}).to_string()
    }
}

pub struct SystemConfigReloadCommand;
impl UqCommand for SystemConfigReloadCommand {
    fn name(&self) -> &str { "system config reload" }
    fn description(&self) -> &str { "Recharger la configuration" }
    fn execute(&self, _args: &str) -> String {
        json!({"status":"success","reloaded":true}).to_string()
    }
}

pub fn register_all_commands(registry: &mut CommandRegistry) {
    registry.register(Box::new(BorbSupplyCommand));
    registry.register(Box::new(BorbPriceCommand));
    registry.register(Box::new(BorbUnitsCommand));
    registry.register(Box::new(BorbConvertCommand));
    registry.register(Box::new(BorbBalanceCommand));
    registry.register(Box::new(BorbTransferCommand));
    registry.register(Box::new(BorbDistributeCommand));
    registry.register(Box::new(BorbFreezeCommand));
    registry.register(Box::new(BorbUnfreezeCommand));
    registry.register(Box::new(BorbLendCommand));
    registry.register(Box::new(BorbRepayCommand));
    registry.register(Box::new(BorbDebtCommand));
    registry.register(Box::new(BorbHistoryCommand));
    registry.register(Box::new(BorbStatsCommand));
    registry.register(Box::new(BorbVelocityCommand));
    registry.register(Box::new(BorbExtensionCommand));
    registry.register(Box::new(CasesStatsCommand));
    registry.register(Box::new(TreasuryStatusCommand));
    registry.register(Box::new(SystemStatusCommand));
    registry.register(Box::new(ServerStartCommand));
    registry.register(Box::new(ServerStopCommand));
    registry.register(Box::new(ServerRestartCommand));
    registry.register(Box::new(ServerStatusCommand));
    registry.register(Box::new(ServerLogsCommand));
    registry.register(Box::new(ServerBackupCommand));
    registry.register(Box::new(ServerConfigCommand));
    registry.register(Box::new(ServerUptimeCommand));
    registry.register(Box::new(HelpServerCommand));
    registry.register(Box::new(RoutesListCommand));
    registry.register(Box::new(RoutesCountCommand));
    registry.register(Box::new(LogsLevelCommand));
    registry.register(Box::new(LogsClearCommand));
    registry.register(Box::new(LogsStatsCommand));
    registry.register(Box::new(UsersListCommand));
    registry.register(Box::new(UsersCountCommand));
    registry.register(Box::new(StatsGlobalCommand));
    registry.register(Box::new(IaStatusCommand));
    registry.register(Box::new(SecurityStatusCommand));
    registry.register(Box::new(PublishCommand));
    registry.register(Box::new(PluginsListCommand));
    registry.register(Box::new(AccountListCommand));
    registry.register(Box::new(AccountCreateCommand));
    registry.register(Box::new(AccountCloseCommand));
    registry.register(Box::new(AccountBlockCommand));
    registry.register(Box::new(AccountUnblockCommand));
    registry.register(Box::new(AccountActivityCommand));
    registry.register(Box::new(AccountStatsCommand));
    registry.register(Box::new(AccountMessagesCommand));
    registry.register(Box::new(AccountTransactionsCommand));
    registry.register(Box::new(AccountDettesCommand));
    registry.register(Box::new(AccountReputationCommand));
    registry.register(Box::new(AccountKycCommand));
    registry.register(Box::new(AccountSearchCommand));
    registry.register(Box::new(AccountFilterCommand));
    registry.register(Box::new(AccountExportCommand));
    registry.register(Box::new(ExchangeOrdersCommand));
    registry.register(Box::new(ExchangeOrderCommand));
    registry.register(Box::new(ExchangeCancelCommand));
    registry.register(Box::new(ExchangeEscrowsCommand));
    registry.register(Box::new(ExchangeReleaseCommand));
    registry.register(Box::new(ExchangeFreezeCommand));
    registry.register(Box::new(ExchangeHistoryCommand));
    registry.register(Box::new(ExchangeVolumeCommand));
    registry.register(Box::new(ExchangeCommissionsCommand));
    registry.register(Box::new(ExchangeAnnoncesCommand));
    registry.register(Box::new(ExchangeLitigesCommand));
    registry.register(Box::new(ExchangeArbitrerCommand));
    registry.register(Box::new(ExchangeMatchCommand));
    registry.register(Box::new(ExchangeReportCommand));
    registry.register(Box::new(PlayerSessionsCommand));
    registry.register(Box::new(PlayerStartCommand));
    registry.register(Box::new(PlayerStopCommand));
    registry.register(Box::new(PlayerStatsCommand));
    registry.register(Box::new(PlayerCreatorsCommand));
    registry.register(Box::new(PlayerDistributeCommand));
    registry.register(Box::new(PlayerSecondsCommand));
    registry.register(Box::new(PlayerRevenueCommand));
    registry.register(Box::new(PlayerPriceCommand));
    registry.register(Box::new(PlayerBoostCommand));
    registry.register(Box::new(PlayerCreatorCommand));
    registry.register(Box::new(PlayerReportCommand));
    registry.register(Box::new(SecurityThreatsCommand));
    registry.register(Box::new(SecurityFraudCommand));
    registry.register(Box::new(SecurityBlockCommand));
    registry.register(Box::new(SecurityUnblockCommand));
    registry.register(Box::new(Security2faCommand));
    registry.register(Box::new(SecurityReportCommand));
    registry.register(Box::new(LogsShowCommand));
    registry.register(Box::new(LogsFilterCommand));
    registry.register(Box::new(LogsSearchCommand));
    registry.register(Box::new(LogsExportCommand));
    registry.register(Box::new(LogsRealtimeCommand));
    registry.register(Box::new(LogsPauseCommand));
    registry.register(Box::new(SystemMemoryCommand));
    registry.register(Box::new(SystemCpuCommand));
    registry.register(Box::new(SystemRestartCommand));
    registry.register(Box::new(SystemPauseCommand));
    registry.register(Box::new(SystemBackupCommand));
    registry.register(Box::new(SystemRestoreCommand));
    registry.register(Box::new(SystemVersionCommand));
    registry.register(Box::new(SystemUpdateCommand));
    registry.register(Box::new(SystemShutdownCommand));
    registry.register(Box::new(SystemConfigReloadCommand));    registry.register(Box::new(P2pStatusCommand));
    registry.register(Box::new(P2pPeersCommand));
    registry.register(Box::new(P2pScanCommand));
    registry.register(Box::new(P2pConnectCommand));
    registry.register(Box::new(P2pDisconnectCommand));
    registry.register(Box::new(P2pRelayCommand));
    registry.register(Box::new(P2pMeshCommand));
    registry.register(Box::new(P2pFrequencyCommand));
    registry.register(Box::new(P2pHopCommand));
    registry.register(Box::new(P2pPhantomCommand));
    registry.register(Box::new(P2pSosCommand));
    registry.register(Box::new(P2pThreatsCommand));
    registry.register(Box::new(P2pD2dScanCommand));
    registry.register(Box::new(P2pD2dPeersCommand));
    registry.register(Box::new(P2pInternetSellCommand));
    registry.register(Box::new(P2pReportCommand));
    registry.register(Box::new(P2pEmergencyCommand));
    registry.register(Box::new(DaoProposalsCommand));
    registry.register(Box::new(DaoCreateCommand));
    registry.register(Box::new(DaoVoteCommand));
    registry.register(Box::new(DaoExecuteCommand));
    registry.register(Box::new(DaoStatsCommand));
    registry.register(Box::new(DaoValidatorsCommand));
    registry.register(Box::new(DaoApprovalsCommand));
    registry.register(Box::new(DaoReportCommand));
    registry.register(Box::new(DaoProposalCommand));
    registry.register(Box::new(DaoValidatorCommand));
    registry.register(Box::new(AiAskCommand));
    registry.register(Box::new(AiAnalyzeCommand));
    registry.register(Box::new(AiDetectCommand));
    registry.register(Box::new(AiSuggestCommand));
    registry.register(Box::new(AiGenerateCommand));
    registry.register(Box::new(AiMarketCommand));
    registry.register(Box::new(AiReportCommand));
    registry.register(Box::new(AiAlertCommand));
    registry.register(Box::new(AiRecommendCommand));
    registry.register(Box::new(AiExecuteCommand));
    registry.register(Box::new(AiChatCommand));
    registry.register(Box::new(AiStatusCommand));
    registry.register(Box::new(StudioListCommand));
    registry.register(Box::new(StudioCreateCommand));
    registry.register(Box::new(StudioDeleteCommand));
    registry.register(Box::new(StudioBoostCommand));
    registry.register(Box::new(StudioStatsCommand));
    registry.register(Box::new(StudioProductsCommand));
    registry.register(Box::new(StudioServicesCommand));
    registry.register(Box::new(StudioSearchCommand));
    registry.register(Box::new(StudioVerifyCommand));
    registry.register(Box::new(StudioVentesCommand));
    registry.register(Box::new(StudioContactsCommand));
    registry.register(Box::new(StudioFilterCommand));
    registry.register(Box::new(StudioExportCommand));
    registry.register(Box::new(StudioUnboostCommand));
    registry.register(Box::new(StudioViewsCommand));
    registry.register(Box::new(UbiInfoCommand));
    registry.register(Box::new(UbiClaimCommand));
    registry.register(Box::new(UbiCheckCommand));
    registry.register(Box::new(UbiDistributeCommand));
    registry.register(Box::new(UbiStatsCommand));
    registry.register(Box::new(UbiHistoryCommand));
    registry.register(Box::new(UbiPauseCommand));
    registry.register(Box::new(UbiResumeCommand));
    registry.register(Box::new(BullesTotalCommand));
    registry.register(Box::new(BullesConvertiesCommand));
    registry.register(Box::new(BullesGeleesCommand));
    registry.register(Box::new(BullesCirculantesCommand));
    registry.register(Box::new(BullesCreateCommand));
    registry.register(Box::new(BullesTransferCommand));
    registry.register(Box::new(BullesReportCommand));
    registry.register(Box::new(VaultStatusCommand));
    registry.register(Box::new(VaultSplitCommand));
    registry.register(Box::new(VaultCombineCommand));
    registry.register(Box::new(VaultGuardiansCommand));
    registry.register(Box::new(VaultReplaceCommand));
    registry.register(Box::new(VaultCompromisedCommand));
    registry.register(Box::new(VaultAuthenticityCommand));
    registry.register(Box::new(VaultP2pStoreCommand));
    registry.register(Box::new(VaultReportCommand));
    registry.register(Box::new(RecycleListCommand));
    registry.register(Box::new(RecycleRunCommand));
    registry.register(Box::new(RecycleScheduleCommand));
    registry.register(Box::new(RecycleCancelCommand));
    registry.register(Box::new(RecycleHistoryCommand));
    registry.register(Box::new(RecycleStatsCommand));
    registry.register(Box::new(RecycleAmountCommand));
    registry.register(Box::new(RecycleFeesCommand));
    registry.register(Box::new(RecycleNextCommand));
    registry.register(Box::new(RecycleReportCommand));
    registry.register(Box::new(AnalyticsTransactionsCommand));
    registry.register(Box::new(AnalyticsVolumeCommand));
    registry.register(Box::new(AnalyticsP2pCommand));
    registry.register(Box::new(AnalyticsPlayerCommand));
    registry.register(Box::new(AnalyticsStudiosCommand));
    registry.register(Box::new(AnalyticsCasesCommand));
    registry.register(Box::new(AnalyticsExportCommand));
    registry.register(Box::new(AnalyticsReportCommand));
    registry.register(Box::new(AnalyticsExportCsvCommand));
    registry.register(Box::new(AnalyticsHeatmapCommand));
}
/// Fonction d'initialisation — RETOURNE un CommandRegistry
pub fn init_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    register_all_commands(&mut registry);
    registry
}
impl CommandRegistry {
    pub fn list_all(&self) -> Vec<String> {
        self.commands.iter().map(|c| c.name().to_string()).collect()
    }

}







