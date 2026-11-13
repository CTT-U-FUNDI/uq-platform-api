#!/usr/bin/env python3
"""
Face Compare V2 — Comparaison robuste YuNet + SFace
Supporte photos N&B, basse résolution
Usage: python face_compare_v2.py <image1> <image2>
Output: JSON avec score de similarité
"""
import sys
import json
import cv2
import numpy as np
from pathlib import Path

MODELS_DIR = Path(__file__).parent.parent.parent / "models" / "face"
YUNET_PATH = MODELS_DIR / "face_detection_yunet_2023mar.onnx"
SFACE_PATH = MODELS_DIR / "face_recognition_sface_2021dec.onnx"

# Charger les modèles
face_detector = cv2.FaceDetectorYN.create(
    str(YUNET_PATH),
    "",
    (320, 320),
    score_threshold=0.6,
    nms_threshold=0.3,
    top_k=5000
)

face_recognizer = cv2.FaceRecognizerSF.create(str(SFACE_PATH), "")

# ═══════════════════════════════════════════════════════════════
# PRÉTRAITEMENT POUR PHOTOS N&B
# ═══════════════════════════════════════════════════════════════

def preprocess_face_for_sface(face_img):
    """
    Prépare un visage pour SFace.
    - Convertit N&B en 3 canaux
    - Améliore contraste
    - Normalise
    """
    if face_img is None or face_img.size == 0:
        return None
    
    # Si N&B → convertir en BGR
    if len(face_img.shape) == 2:
        face_img = cv2.cvtColor(face_img, cv2.COLOR_GRAY2BGR)
    elif face_img.shape[2] == 1:
        face_img = cv2.cvtColor(face_img, cv2.COLOR_GRAY2BGR)
    
    # Upscale si nécessaire (SFace optimal : 112x112 minimum)
    h, w = face_img.shape[:2]
    if h < 112 or w < 112:
        scale = max(112 / h, 112 / w)
        face_img = cv2.resize(face_img, None, fx=scale, fy=scale, interpolation=cv2.INTER_CUBIC)
    
    # Améliorer contraste (utile pour photos N&B sépia)
    lab = cv2.cvtColor(face_img, cv2.COLOR_BGR2LAB)
    l, a, b = cv2.split(lab)
    clahe = cv2.createCLAHE(clipLimit=3.0, tileGridSize=(8, 8))
    l = clahe.apply(l)
    enhanced = cv2.merge([l, a, b])
    face_img = cv2.cvtColor(enhanced, cv2.COLOR_LAB2BGR)
    
    return face_img

# ═══════════════════════════════════════════════════════════════
# DÉTECTION VISAGE
# ═══════════════════════════════════════════════════════════════

def detect_face(img):
    """Détecte le visage le plus grand"""
    h, w = img.shape[:2]
    face_detector.setInputSize((w, h))
    
    _, faces = face_detector.detect(img)
    
    if faces is None or len(faces) == 0:
        return None
    
    return max(faces, key=lambda f: f[2] * f[3])

def get_embedding(img, face, margin=0.2):
    """
    Extrait l'embedding d'un visage.
    Utilise face_recognizer.alignCrop puis feature().
    """
    try:
        # alignCrop aligne et recadre le visage
        aligned = face_recognizer.alignCrop(img, face)
        
        # Améliorer pour N&B
        aligned = preprocess_face_for_sface(aligned)
        
        if aligned is None:
            return None
        
        # Extraire feature (embedding 128D)
        feat = face_recognizer.feature(aligned)
        return feat
    except Exception as e:
        print(f"Error in get_embedding: {e}", file=sys.stderr)
        return None

# ═══════════════════════════════════════════════════════════════
# COMPARAISON
# ═══════════════════════════════════════════════════════════════

def compare_faces(img1_path, img2_path):
    """Compare 2 visages (image complète ou déjà croppée)"""
    img1 = cv2.imread(img1_path)
    img2 = cv2.imread(img2_path)
    
    if img1 is None or img2 is None:
        return {"status": "error", "message": "Image(s) introuvable(s)"}
    
    face1 = detect_face(img1)
    face2 = detect_face(img2)
    
    if face1 is None:
        return {"status": "error", "message": "Aucun visage dans image1"}
    if face2 is None:
        return {"status": "error", "message": "Aucun visage dans image2"}
    
    emb1 = get_embedding(img1, face1)
    emb2 = get_embedding(img2, face2)
    
    if emb1 is None or emb2 is None:
        return {"status": "error", "message": "Impossible d'extraire les embeddings"}
    
    # Similarité cosinus
    score = face_recognizer.match(emb1, emb2, cv2.FaceRecognizerSF_FR_COSINE)
    
    # Similarité L2 (plus permissive)
    score_l2 = face_recognizer.match(emb1, emb2, cv2.FaceRecognizerSF_FR_NORM_L2)
    
    # Moyenne pondérée
    final_score = float(score) * 0.7 + (1.0 - float(score_l2)) * 0.3
    
    # Verdict (seuils adaptés pour photos N&B)
    if final_score > 0.5:
        verdict = "MATCH"
    elif final_score > 0.35:
        verdict = "PARTIAL"
    else:
        verdict = "NO_MATCH"
    
    return {
        "status": "success",
        "similarity": round(final_score, 4),
        "cosine_score": round(float(score), 4),
        "l2_score": round(float(score_l2), 4),
        "verdict": verdict,
        "face1_confidence": round(float(face1[14]), 3) if len(face1) > 14 else 0.0,
        "face2_confidence": round(float(face2[14]), 3) if len(face2) > 14 else 0.0,
        "face1_box": face1[:4].tolist(),
        "face2_box": face2[:4].tolist()
    }

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(json.dumps({"status": "error", "message": "Usage: python face_compare_v2.py <image1> <image2>"}))
        sys.exit(1)
    
    result = compare_faces(sys.argv[1], sys.argv[2])
    print(json.dumps(result, ensure_ascii=False, indent=2))
