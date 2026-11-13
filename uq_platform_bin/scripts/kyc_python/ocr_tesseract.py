#!/usr/bin/env python3
"""
OCR Tesseract — Extraction automatique des champs CIN/Passeport
Usage: python ocr_tesseract.py <image_path> [--lang=fra] [--country=RDC]
Output: JSON avec champs extraits
"""
import sys
import json
import re
from pathlib import Path

try:
    import pytesseract
    from PIL import Image
    import cv2
    import numpy as np
except ImportError as e:
    print(json.dumps({"status": "error", "message": f"Module manquant: {e}"}))
    sys.exit(1)

# Tesseract
pytesseract.pytesseract.tesseract_cmd = r'C:\Program Files\Tesseract-OCR\tesseract.exe'

# Mapping pays → langues
LANG_BY_COUNTRY = {
    "RDC": "fra+swa+lin",
    "Congo": "fra+lin",
    "Angola": "por",
    "Kenya": "eng+swa",
    "Rwanda": "kin+fra",
    "France": "fra",
    "USA": "eng",
    "Chine": "chi_sim+eng",
    "default": "fra+eng"
}

# ═══════════════════════════════════════════════════════════════
# PRÉTRAITEMENT IMAGE
# ═══════════════════════════════════════════════════════════════

def preprocess_image(image_path):
    """Améliore l'image pour l'OCR"""
    img = cv2.imread(image_path)
    if img is None:
        return None
    
    # Convertir en niveaux de gris
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    
    # Améliorer contraste (CLAHE)
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8))
    enhanced = clahe.apply(gray)
    
    # Débruitage léger
    denoised = cv2.fastNlMeansDenoising(enhanced, h=10)
    
    # Binarisation Otsu (mieux pour texte bien contrasté)
    _, binary = cv2.threshold(denoised, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
    
    return binary

# ═══════════════════════════════════════════════════════════════
# OCR
# ═══════════════════════════════════════════════════════════════

def run_ocr(image_path, lang="fra"):
    """OCR complet sur une image"""
    processed = preprocess_image(image_path)
    
    if processed is None:
        img = Image.open(image_path)
    else:
        temp_path = str(Path(image_path).parent / "_temp_processed.jpg")
        cv2.imwrite(temp_path, processed)
        img = Image.open(temp_path)
    
    # Configuration Tesseract optimisée
    config = '--psm 6 -c preserve_interword_spaces=1'
    
    # OCR avec data détaillées
    data = pytesseract.image_to_data(
        img, lang=lang, config=config,
        output_type=pytesseract.Output.DICT
    )
    
    # Texte complet
    text = pytesseract.image_to_string(img, lang=lang, config=config)
    
    # Confiance moyenne
    confidences = [int(c) for c in data["conf"] if int(c) > 0]
    avg_conf = sum(confidences) / len(confidences) if confidences else 0
    
    return {
        "text": text.strip(),
        "confidence": round(avg_conf / 100, 3),
        "word_count": len([w for w in text.split() if w.strip()])
    }

# ═══════════════════════════════════════════════════════════════
# NETTOYAGE DES VALEURS
# ═══════════════════════════════════════════════════════════════

def clean_value(value):
    """Nettoie une valeur extraite"""
    if not value:
        return ""
    
    # Enlever les deux-points, virgules, espaces en début/fin
    value = value.strip()
    value = re.sub(r'^[\s:,;.\-]+', '', value)  # Début
    value = re.sub(r'[\s:,;.\-]+$', '', value)  # Fin
    value = re.sub(r'\s+', ' ', value)           # Espaces multiples
    value = re.sub(r':+', '', value)             # Deux-points internes
    
    return value.strip()

# ═══════════════════════════════════════════════════════════════
# EXTRACTION DES CHAMPS (AMÉLIORÉE)
# ═══════════════════════════════════════════════════════════════

def extract_field(text, labels, value_pattern=None):
    """
    Extrait un champ après un label.
    value_pattern : regex optionnelle pour la valeur
    """
    for label in labels:
        # Pattern flexible : LABEL suivi de : ou espaces
        pattern = rf"{label}\s*:?\s*:?\s*([^\n]+)"
        match = re.search(pattern, text, re.IGNORECASE)
        if match:
            value = match.group(1).strip()
            
            # Nettoyer
            value = clean_value(value)
            
            # Appliquer le pattern de valeur si fourni
            if value_pattern:
                vm = re.search(value_pattern, value)
                if vm:
                    return vm.group(0)
            
            if value:
                return value
    return ""

def extract_date(text):
    """Extrait une date avec plusieurs formats"""
    patterns = [
        r'\d{1,2}[/\-\.]\d{1,2}[/\-\.]\d{2,4}',  # 01/06/1996
        r'\d{4}[/\-\.]\d{1,2}[/\-\.]\d{1,2}',    # 1996/06/01
        r'\d{1,2}\s+\w+\s+\d{4}',                # 01 juin 1996
        r'\d{1,2}\s+(?:janvier|février|mars|avril|mai|juin|juillet|août|septembre|octobre|novembre|décembre)\s+\d{4}',
    ]
    
    for pattern in patterns:
        match = re.search(pattern, text, re.IGNORECASE)
        if match:
            return match.group(0)
    
    return ""

def extract_fields(text):
    """Extrait tous les champs standards"""
    fields = {}
    
    # Nom
    nom = extract_field(text, ["Nom", "NOM", "Nom de famille"])
    if nom:
        fields["nom"] = nom
    
    # Prénom
    prenom = extract_field(text, ["Prenom", "Prénom", "PRENOM", "Prénoms"])
    if prenom:
        fields["prenom"] = prenom
    
    # Date de naissance (pattern date obligatoire)
    date_match = re.search(r'(?:Date\s*(?:de\s*)?naissance|Né[e]?\s*le)\s*:?\s*([^\n]+)', text, re.IGNORECASE)
    if date_match:
        date_val = extract_date(date_match.group(1))
        if date_val:
            fields["date_naissance"] = date_val
    else:
        # Fallback : chercher n'importe quelle date
        date_val = extract_date(text)
        if date_val:
            fields["date_naissance"] = date_val
    
    # Lieu de naissance
    lieu = extract_field(text, ["Lieu naissance", "Lieu de naissance", "Né[e]? à"])
    if lieu:
        # Enlever les caractères parasites
        lieu = re.sub(r'^[,;:\s]+', '', lieu)
        fields["lieu_naissance"] = lieu
    
    # NIN / Numéro (support multi-tirets et caractères OCR bizarres)
    nin_match = re.search(r'(?:NIN|Num[eé]ro|N°)\s*:?\s*(.+?)(?:\n|$)', text, re.IGNORECASE)
    if nin_match:
        nin_raw = nin_match.group(1).strip()
        # Garder uniquement les chiffres
        nin = re.sub(r'[^\d]', '', nin_raw)
        # Formater : groupes de 4 (si 12 chiffres)
        if len(nin) == 12:
            nin = f"{nin[:4]}-{nin[4:8]}-{nin[8:]}"
        if nin:
            fields["nin"] = nin
    
    # Nationalité
    nationalite = extract_field(text, ["Nationalite", "Nationalité"])
    if nationalite:
        fields["nationalite"] = nationalite
    
    # Sexe
    sexe = extract_field(text, ["Sexe", "Genre"])
    if sexe:
        fields["sexe"] = sexe
    
    # Adresse
    adresse = extract_field(text, ["Adresse", "Domicile"])
    if adresse:
        fields["adresse"] = adresse
    
    return fields

# ═══════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "Usage: python ocr_tesseract.py <image> [--lang=fra] [--country=RDC]"}))
        sys.exit(1)
    
    image_path = sys.argv[1]
    lang = "fra"
    country = "RDC"
    
    for arg in sys.argv[2:]:
        if arg.startswith("--lang="):
            lang = arg.split("=")[1]
        elif arg.startswith("--country="):
            country = arg.split("=")[1]
            lang = LANG_BY_COUNTRY.get(country, lang)
    
    if not Path(image_path).exists():
        print(json.dumps({"status": "error", "message": f"Image introuvable: {image_path}"}))
        sys.exit(1)
    
    try:
        ocr_result = run_ocr(image_path, lang)
        fields = extract_fields(ocr_result["text"])
        
        result = {
            "status": "success",
            "ocr": {
                "text": ocr_result["text"],
                "confidence": ocr_result["confidence"],
                "word_count": ocr_result["word_count"]
            },
            "fields": fields,
            "language": lang,
            "country": country
        }
        
        print(json.dumps(result, ensure_ascii=False, indent=2))
        
    except Exception as e:
        print(json.dumps({"status": "error", "message": str(e)}))
        sys.exit(1)

if __name__ == "__main__":
    main()
