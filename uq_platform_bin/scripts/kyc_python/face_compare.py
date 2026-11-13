#!/usr/bin/env python3
"""
Face Compare — YuNet (détection) + SFace (reconnaissance)
Usage: python face_compare.py <image1> <image2>
Output: JSON avec score de similarité
"""
import sys
import json
import numpy as np
import cv2
from pathlib import Path

# Chemins
BASE_DIR = Path(__file__).parent.parent.parent
MODELS_DIR = BASE_DIR / "models" / "face"

YUNET_PATH = MODELS_DIR / "face_detection_yunet_2023mar.onnx"
SFACE_PATH = MODELS_DIR / "face_recognition_sface_2021dec.onnx"

# ═══════════════════════════════════════════════════════════════
# CHARGEMENT
# ═══════════════════════════════════════════════════════════════

face_detector = cv2.FaceDetectorYN.create(
    str(YUNET_PATH),
    "",
    (320, 320),
    score_threshold=0.7,
    nms_threshold=0.3,
    top_k=5000
)

face_recognizer = cv2.FaceRecognizerSF.create(
    str(SFACE_PATH),
    ""
)

# ═══════════════════════════════════════════════════════════════
# FONCTIONS
# ═══════════════════════════════════════════════════════════════

def detect_face(img):
    """Détecte le visage le plus grand"""
    h, w = img.shape[:2]
    face_detector.setInputSize((w, h))
    
    _, faces = face_detector.detect(img)
    
    if faces is None or len(faces) == 0:
        return None
    
    # Prendre le plus grand
    largest = max(faces, key=lambda f: f[2] * f[3])
    return largest

def extract_embedding(img, face):
    """Extrait l'embedding du visage"""
    aligned = face_recognizer.alignCrop(img, face)
    feat = face_recognizer.feature(aligned)
    return feat

def compare_faces(img1_path, img2_path):
    """Compare 2 visages"""
    img1 = cv2.imread(img1_path)
    img2 = cv2.imread(img2_path)
    
    if img1 is None or img2 is None:
        return {"status": "error", "message": "Image introuvable"}
    
    face1 = detect_face(img1)
    face2 = detect_face(img2)
    
    if face1 is None:
        return {"status": "error", "message": "Aucun visage détecté dans image1"}
    if face2 is None:
        return {"status": "error", "message": "Aucun visage détecté dans image2"}
    
    emb1 = extract_embedding(img1, face1)
    emb2 = extract_embedding(img2, face2)
    
    # Similarité cosinus
    score = face_recognizer.match(emb1, emb2, cv2.FaceRecognizerSF_FR_COSINE)
    
    # Interprétation
    if score > 0.6:
        verdict = "MATCH"
    elif score > 0.4:
        verdict = "PARTIAL"
    else:
        verdict = "NO_MATCH"
    
    return {
        "status": "success",
        "similarity": round(float(score), 4),
        "verdict": verdict,
        "face1_box": face1[:4].tolist(),
        "face2_box": face2[:4].tolist()
    }

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(json.dumps({"status": "error", "message": "Usage: python face_compare.py <image1> <image2>"}))
        sys.exit(1)
    
    result = compare_faces(sys.argv[1], sys.argv[2])
    print(json.dumps(result, ensure_ascii=False, indent=2))
