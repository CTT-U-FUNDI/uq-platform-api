#!/usr/bin/env python3
"""
Liveness Check — Anti-spoofing vidéo
Usage: python liveness_check.py <video_path>
Output: JSON avec score de vivacité
"""
import sys
import json
import numpy as np
import cv2
from pathlib import Path

BASE_DIR = Path(__file__).parent.parent.parent
MODELS_DIR = BASE_DIR / "models" / "face"
YUNET_PATH = MODELS_DIR / "face_detection_yunet_2023mar.onnx"

face_detector = cv2.FaceDetectorYN.create(
    str(YUNET_PATH),
    "",
    (320, 320),
    score_threshold=0.7
)

def analyze_video(video_path):
    """Analyse une vidéo pour la vivacité"""
    cap = cv2.VideoCapture(video_path)
    
    if not cap.isOpened():
        return {"status": "error", "message": "Vidéo introuvable"}
    
    frame_count = 0
    face_detections = 0
    face_positions = []
    face_sizes = []
    
    while True:
        ret, frame = cap.read()
        if not ret:
            break
        
        frame_count += 1
        h, w = frame.shape[:2]
        face_detector.setInputSize((w, h))
        
        _, faces = face_detector.detect(frame)
        
        if faces is not None and len(faces) > 0:
            face_detections += 1
            largest = max(faces, key=lambda f: f[2] * f[3])
            
            # Position du centre
            cx = largest[0] + largest[2] / 2
            cy = largest[1] + largest[3] / 2
            face_positions.append([cx, cy])
            
            # Taille du visage
            face_sizes.append(largest[2] * largest[3])
    
    cap.release()
    
    if frame_count == 0:
        return {"status": "error", "message": "Vidéo vide"}
    
    # Calculs
    detection_rate = face_detections / frame_count
    
    # Mouvement (variation des positions)
    movement = 0.0
    if len(face_positions) > 1:
        positions = np.array(face_positions)
        diffs = np.diff(positions, axis=0)
        movement = float(np.mean(np.linalg.norm(diffs, axis=1)))
    
    # Variation de taille (profondeur)
    size_variation = 0.0
    if len(face_sizes) > 1:
        size_variation = float(np.std(face_sizes) / np.mean(face_sizes))
    
    # Score de vivacité
    score = 0.0
    score += min(detection_rate, 1.0) * 0.4      # Visage présent
    score += min(movement / 5.0, 1.0) * 0.3       # Mouvement
    score += min(size_variation, 1.0) * 0.3       # Profondeur
    
    # Verdict
    if score > 0.7:
        verdict = "LIVE"
    elif score > 0.4:
        verdict = "SUSPICIOUS"
    else:
        verdict = "SPOOF"
    
    return {
        "status": "success",
        "liveness_score": round(score, 3),
        "verdict": verdict,
        "frames": frame_count,
        "face_detections": face_detections,
        "detection_rate": round(detection_rate, 3),
        "movement": round(movement, 2),
        "size_variation": round(size_variation, 3)
    }

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "Usage: python liveness_check.py <video_path>"}))
        sys.exit(1)
    
    result = analyze_video(sys.argv[1])
    print(json.dumps(result, ensure_ascii=False, indent=2))
