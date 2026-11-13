#!/usr/bin/env python3
"""
Face Extract — Extrait le visage d'une image (doc ou selfie)
Usage: python face_extract.py <image_path> <output_path>
Output: JSON avec info visage + image croppée
"""
import sys
import json
import cv2
import numpy as np
from pathlib import Path

MODELS_DIR = Path(__file__).parent.parent.parent / "models" / "face"
YUNET_PATH = MODELS_DIR / "face_detection_yunet_2023mar.onnx"

# Charger YuNet
face_detector = cv2.FaceDetectorYN.create(
    str(YUNET_PATH),
    "",
    (320, 320),
    score_threshold=0.6,
    nms_threshold=0.3,
    top_k=5000
)

def extract_face(image_path, output_path, margin=0.3):
    """
    Détecte et extrait le visage le plus grand.
    margin : pourcentage d'extension autour du visage (0.3 = 30%)
    """
    img = cv2.imread(image_path)
    if img is None:
        return {"status": "error", "message": "Image introuvable"}
    
    h, w = img.shape[:2]
    face_detector.setInputSize((w, h))
    
    _, faces = face_detector.detect(img)
    
    if faces is None or len(faces) == 0:
        return {"status": "error", "message": "Aucun visage détecté"}
    
    # Prendre le plus grand visage
    largest = max(faces, key=lambda f: f[2] * f[3])
    x, y, fw, fh = largest[:4]
    
    # Ajouter une marge
    mx = int(fw * margin)
    my = int(fh * margin)
    
    x1 = max(0, int(x) - mx)
    y1 = max(0, int(y) - my)
    x2 = min(w, int(x + fw) + mx)
    y2 = min(h, int(y + fh) + my)
    
    # Extraire le visage
    face_crop = img[y1:y2, x1:x2]
    
    # Améliorer la qualité (pour photos N&B des CIN)
    # 1. Upscale si trop petit
    if face_crop.shape[0] < 200:
        scale = 200 / face_crop.shape[0]
        face_crop = cv2.resize(face_crop, None, fx=scale, fy=scale, interpolation=cv2.INTER_CUBIC)
    
    # 2. Améliorer contraste (CLAHE) pour photos N&B
    gray = cv2.cvtColor(face_crop, cv2.COLOR_BGR2GRAY)
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8))
    enhanced = clahe.apply(gray)
    
    # 3. Reconvertir en BGR (SFace attend 3 canaux)
    face_crop_enhanced = cv2.cvtColor(enhanced, cv2.COLOR_GRAY2BGR)
    
    # Sauvegarder
    cv2.imwrite(output_path, face_crop_enhanced)
    
    # Confiance
    confidence = float(largest[14]) if len(largest) > 14 else 0.0
    
    return {
        "status": "success",
        "output": output_path,
        "bbox": [int(x), int(y), int(fw), int(fh)],
        "confidence": round(confidence, 3),
        "size": [int(face_crop.shape[1]), int(face_crop.shape[0])]
    }

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(json.dumps({"status": "error", "message": "Usage: python face_extract.py <image> <output>"}))
        sys.exit(1)
    
    result = extract_face(sys.argv[1], sys.argv[2])
    print(json.dumps(result, ensure_ascii=False, indent=2))
