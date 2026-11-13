import re

path = r"C:\Users\destin\Desktop\UQ_Platform\uq_platform_bin\src\database.rs"

with open(path, 'r', encoding='utf-8') as f:
    content = f.read()

# Vérifier si journals existe
if 'CREATE TABLE IF NOT EXISTS journals' in content:
    print('  Table journals deja presente')
    exit(0)

# Trouver la table sessions et ajouter journals apres
sessions_pattern = r'(CREATE TABLE IF NOT EXISTS sessions \(.*?\);)\s*'

def add_journals(match):
    sessions_table = match.group(1)
    journals_table = '''

            -- ═══════════════════════════════════════════════════════════
            -- TABLE journals — Journal individuel par compte
            -- ═══════════════════════════════════════════════════════════
            CREATE TABLE IF NOT EXISTS journals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                tx_type TEXT NOT NULL,
                amount_points INTEGER NOT NULL,
                fee_points INTEGER DEFAULT 0,
                counterparty TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_journals_account ON journals (account_id);
'''
    return sessions_table + journals_table

# Appliquer le remplacement (DOTALL pour matcher plusieurs lignes)
new_content = re.sub(sessions_pattern, add_journals, content, count=1, flags=re.DOTALL)

# Vérifier que l'ajout a eu lieu
if 'CREATE TABLE IF NOT EXISTS journals' in new_content:
    with open(path, 'w', encoding='utf-8') as f:
        f.write(new_content)
    print('  Table journals AJOUTEE')
else:
    print('  ERREUR : sessions non trouvee')
