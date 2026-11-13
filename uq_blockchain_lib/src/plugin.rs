// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// UQ CORE - plugin.rs
// SystÃ¨me de plugins - ExtensibilitÃ© modulaire
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginRegistry {
    pub plugins: HashMap<String, Plugin>,
    pub loaded_order: Vec<String>,
    pub max_plugins: u32,
    pub sandbox_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub permissions: Vec<PluginPermission>,
    pub state: PluginState,
    pub hash: [u8; 32],
    pub loaded_at: Option<u64>,
    pub error_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginState { Registered, Loaded, Running, Paused, Failed, Unloaded }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PluginPermission {
    ReadStorage, WriteStorage, NetworkAccess, ReadIdentity,
    AccessWallet, SendTransactions, ValidateBlocks, BroadcastMessages,
    AccessCamera, AccessMicrophone, AccessLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String, pub name: String, pub version: String,
    pub author: String, pub description: String,
    pub permissions: Vec<PluginPermission>, pub min_uq_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStats {
    pub total_plugins: u32, pub loaded_plugins: u32,
    pub running_plugins: u32, pub failed_plugins: u32,
}

fn get_hardbit() -> u64 { crate::hardbit::get_current_hardbit() }
fn sha256(data: &[u8]) -> [u8; 32] { let mut h = Sha256::new(); h.update(data); let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r }

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F1. CRÃ‰ER LE REGISTRE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn create_registry(max_plugins: u32) -> PluginRegistry {
    PluginRegistry { plugins: HashMap::new(), loaded_order: Vec::new(), max_plugins, sandbox_enabled: true }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F2. ENREGISTRER UN PLUGIN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn register_plugin(registry: &mut PluginRegistry, manifest: &PluginManifest, code_hash: &[u8; 32]) -> Result<(), ChainError> {
    if registry.plugins.len() >= registry.max_plugins as usize {
        return Err(ChainError::InternalError("Registre plein".into()));
    }
    if registry.plugins.contains_key(&manifest.id) {
        return Err(ChainError::InternalError("Plugin dÃ©jÃ  enregistrÃ©".into()));
    }
    let plugin = Plugin {
        id: manifest.id.clone(), name: manifest.name.clone(),
        version: manifest.version.clone(), author: manifest.author.clone(),
        description: manifest.description.clone(), permissions: manifest.permissions.clone(),
        state: PluginState::Registered, hash: *code_hash, loaded_at: None, error_count: 0,
    };
    registry.plugins.insert(manifest.id.clone(), plugin);
    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F3. CHARGER UN PLUGIN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn load_plugin(registry: &mut PluginRegistry, plugin_id: &str) -> Result<(), ChainError> {
    if let Some(p) = registry.plugins.get_mut(plugin_id) {
        p.state = PluginState::Loaded;
        p.loaded_at = Some(get_hardbit());
        registry.loaded_order.push(plugin_id.to_string());
        Ok(())
    } else { Err(ChainError::InternalError("Plugin introuvable".into())) }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F4. DÃ‰MARRER UN PLUGIN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn start_plugin(registry: &mut PluginRegistry, plugin_id: &str) -> Result<(), ChainError> {
    if let Some(p) = registry.plugins.get_mut(plugin_id) {
        if p.state != PluginState::Loaded { return Err(ChainError::InternalError("Plugin non chargÃ©".into())); }
        p.state = PluginState::Running;
        Ok(())
    } else { Err(ChainError::InternalError("Plugin introuvable".into())) }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F5. ARRÃŠTER UN PLUGIN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn stop_plugin(registry: &mut PluginRegistry, plugin_id: &str) -> Result<(), ChainError> {
    if let Some(p) = registry.plugins.get_mut(plugin_id) {
        p.state = PluginState::Paused;
        Ok(())
    } else { Err(ChainError::InternalError("Plugin introuvable".into())) }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F6. DÃ‰CHARGER UN PLUGIN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn unload_plugin(registry: &mut PluginRegistry, plugin_id: &str) -> Result<(), ChainError> {
    registry.loaded_order.retain(|id| id != plugin_id);
    if let Some(p) = registry.plugins.get_mut(plugin_id) {
        p.state = PluginState::Unloaded; Ok(())
    } else { Err(ChainError::InternalError("Plugin introuvable".into())) }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F7. VÃ‰RIFIER LES PERMISSIONS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn check_permission(registry: &PluginRegistry, plugin_id: &str, permission: &PluginPermission) -> Result<bool, ChainError> {
    if let Some(p) = registry.plugins.get(plugin_id) {
        Ok(p.permissions.contains(permission))
    } else { Err(ChainError::InternalError("Plugin introuvable".into())) }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F8. STATISTIQUES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_plugin_stats(registry: &PluginRegistry) -> PluginStats {
    PluginStats {
        total_plugins: registry.plugins.len() as u32,
        loaded_plugins: registry.plugins.values().filter(|p| p.state == PluginState::Loaded || p.state == PluginState::Running).count() as u32,
        running_plugins: registry.plugins.values().filter(|p| p.state == PluginState::Running).count() as u32,
        failed_plugins: registry.plugins.values().filter(|p| p.state == PluginState::Failed).count() as u32,
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F9. LISTER LES PLUGINS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn list_plugins(registry: &PluginRegistry) -> Vec<&Plugin> {
    registry.plugins.values().collect()
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F10. CHARGER TOUS LES PLUGINS DANS L'ORDRE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn load_all_plugins(registry: &mut PluginRegistry) -> Result<u32, ChainError> {
    let ids: Vec<String> = registry.plugins.keys().cloned().collect();
    let mut count = 0u32;
    for id in &ids {
        if load_plugin(registry, id).is_ok() { count += 1; }
    }
    Ok(count)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// TESTS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> PluginManifest {
        PluginManifest { id:"test.plugin".into(), name:"Test".into(), version:"1.0".into(), author:"UQ".into(), description:"Test plugin".into(), permissions:vec![PluginPermission::ReadStorage, PluginPermission::NetworkAccess], min_uq_version:"1.0".into() }
    }

    fn registry() -> PluginRegistry { create_registry(10) }

    #[test] fn test_register() { let mut r = registry(); assert!(register_plugin(&mut r, &manifest(), &[1u8;32]).is_ok()); assert_eq!(r.plugins.len(), 1); }
    #[test] fn test_register_duplicate() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); assert!(register_plugin(&mut r, &manifest(), &[2u8;32]).is_err()); }
    #[test] fn test_load() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); assert!(load_plugin(&mut r, "test.plugin").is_ok()); }
    #[test] fn test_start_stop() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); load_plugin(&mut r, "test.plugin").unwrap_or_default(); assert!(start_plugin(&mut r, "test.plugin").is_ok()); assert!(stop_plugin(&mut r, "test.plugin").is_ok()); }
    #[test] fn test_unload() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); load_plugin(&mut r, "test.plugin").unwrap_or_default(); assert!(unload_plugin(&mut r, "test.plugin").is_ok()); }
    #[test] fn test_permissions() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); assert!(check_permission(&r, "test.plugin", &PluginPermission::ReadStorage).unwrap_or_default()); assert!(!check_permission(&r, "test.plugin", &PluginPermission::AccessWallet).unwrap_or_default()); }
    #[test] fn test_stats() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); load_plugin(&mut r, "test.plugin").unwrap_or_default(); start_plugin(&mut r, "test.plugin").unwrap_or_default(); let s = get_plugin_stats(&r); assert_eq!(s.total_plugins, 1); assert_eq!(s.running_plugins, 1); }
    #[test] fn test_list() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); assert_eq!(list_plugins(&r).len(), 1); }
    #[test] fn test_load_all() { let mut r = registry(); register_plugin(&mut r, &manifest(), &[1u8;32]).unwrap_or_default(); let m2 = PluginManifest { id:"p2".into(), ..manifest() }; register_plugin(&mut r, &m2, &[2u8;32]).unwrap_or_default(); assert_eq!(load_all_plugins(&mut r).unwrap_or_default(), 2); }
    #[test] fn test_not_found() { let r = registry(); assert!(load_plugin(&mut r.clone(), "nope").is_err()); }
}





