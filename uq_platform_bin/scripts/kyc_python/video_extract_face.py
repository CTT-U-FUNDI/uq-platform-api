#!/usr/bin/env python3
"""
Video Extract Face — Extrait le meilleur visage d'une vidéo
Usage: python video_extract_face.py <video_path> <output_path>
Output: JSON avec la meilleure frame + visage
"""
import sys
import json
import cv2
import numpy as np
from pathlib import Path

MODELS_DIR = Path(__file__).parent.parent.parent / "models" / "face"
YUNET_PATH = MODELS_DIR / "face_detection_yunet_2023mar.onnx"

face_detector = cv2.FaceDetectorYN.create(
    str(YUNET_PATH),
    "",
    (320, 320),
    score_threshold=0.7,
    nms_threshold=0.3,
    top_k=5000
)

def laplacian_variance(img):
    """Mesure la netteté (variance du Laplacien)"""
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    return cv2.Laplacian(gray, cv2.CV_64F).var()

def extract_best_face(video_path, output_path):
    """Parcourt la vidéo et extrait la meilleure frame avec visage"""
    cap = cv2.VideoCapture(video_path)
    
    if not cap.isOpened():
        return {"status": "error", "message": "Vidéo introuvable"}
    
    best_score = 0
    best_frame = None
    best_face = None
    frames_analyzed = 0
    frames_with_face = 0
    
    while True:
        ret, frame = cap.read()
        if not ret:
            break
        
        frames_analyzed += 1
        
        # Analyser 1 frame sur 5 (performance)
        if frames_analyzed % 5 != 0:
            continue
        
        h, w = frame.shape[:2]
        face_detector.setInputSize((w, h))
        
        _, faces = face_detector.detect(frame)
        
        if faces is not None and len(faces) > 0:
            frames_with_face += 1
            
            # Prendre le plus grand visage
            largest = max(faces, key=lambda f: f[2] * f[3])
            x, y, fw, fh = largest[:4]
            
            # Vérifier la taille minimum
            if fw < 80 or fh < 80:
                continue
            
            # Détecter les yeux (indicateur de bonne qualité)
            confidence = float(largest[14]) if len(largest) > 14 else 0.0
            
            # Score composite : confiance + netteté + taille
            sharpness = laplacian_variance(frame)
            size_score = min(fw * fh / (200 * 200), 1.0)
            
            score = confidence * 0.5 + min(sharpness / 500, 1.0) * 0.3 + size_score * 0.2
            
            if score > best_score:
                best_score = score
                best_frame = frame.copy()
                best_face = largest.copy()
    
    cap.release()
    
    if best_frame is None:
        return {
            "status": "error",
            "message": "Aucun visage de qualité détecté dans la vidéo",
            "frames_analyzed": frames_analyzed,
            "frames_with_face": frames_with_face
        }
    
    # Extraire le visage avec marge
    x, y, fw, fh = best_face[:4]
    margin = 0.3
    mx = int(fw * margin)
    my = int(fh * margin)
    
    h, w = best_frame.shape[:2]
    x1 = max(0, int(x) - mx)
    y1 = max(0, int(y) - my)
    x2 = min(w, int(x + fw) + mx)
    y2 = min(h, int(y + fh) + my)
    
    face_crop = best_frame[y1:y2, x1:x2]
    
    # Upscale si nécessaire
    if face_crop.shape[0] < 200:
        scale = 200 / face_crop.shape[0]
        face_crop = cv2.resize(face_crop, None, fx=scale, fy=scale, interpolation=cv2.INTER_CUBIC)
    
    # Sauvegarder
    cv2.imwrite(output_path, face_crop)
    
    confidence = float(best_face[14]) if len(best_face) > 14 else 0.0
    
    return {
        "status": "success",
        "output": output_path,
        "quality_score": round(best_score, 3),
        "confidence": round(confidence, 3),
        "bbox": [int(x), int(y), int(fw), int(fh)],
        "size": [int(face_crop.shape[1]), int(face_crop.shape[0])],
        "frames_analyzed": frames_analyzed,
        "frames_with_face": frames_with_face,
        "frames_ratio": round(frames_with_face / max(frames_analyzed, 1), 3)
    }

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print(json.dumps({"status": "error", "message": "Usage: python video_extract_face.py <video> <output>"}))
        sys.exit(1)
    
    result = extract_best_face(sys.argv[1], sys.argv[2])
    print(json.dumps(result, ensure_ascii=False, indent=2))
