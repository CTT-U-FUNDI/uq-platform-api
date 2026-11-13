#!/usr/bin/env python3
"""
OCR Pipeline — PaddleOCR (det + rec + dict)
Usage: python ocr_pipeline.py <image_path>
Output: JSON avec les textes détectés
"""
import sys
import json
import numpy as np
import cv2
from pathlib import Path

# Chemins
BASE_DIR = Path(__file__).parent.parent.parent
MODELS_DIR = BASE_DIR / "models" / "small"

DET_PATH = MODELS_DIR / "det.onnx"
REC_PATH = MODELS_DIR / "rec.onnx"
DICT_PATH = MODELS_DIR / "dict.txt"

# Config PaddleOCR
DET_LIMIT_SIDE = 960
DET_THRESH = 0.3
DET_BOX_THRESH = 0.6
DET_UNCLIP_RATIO = 1.5
REC_HEIGHT = 48
REC_BATCH = 6

# ═══════════════════════════════════════════════════════════════
# CHARGEMENT
# ═══════════════════════════════════════════════════════════════

import onnxruntime as ort

sess_opts = ort.SessionOptions()
sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

det_session = ort.InferenceSession(str(DET_PATH), sess_opts, providers=["CPUExecutionProvider"])
rec_session = ort.InferenceSession(str(REC_PATH), sess_opts, providers=["CPUExecutionProvider"])

# Charger dict
with open(DICT_PATH, "r", encoding="utf-8") as f:
    dict_lines = f.read().splitlines()

charset = ["[blank]"] + dict_lines + [""] * 324  # Padding pour 18710

# ═══════════════════════════════════════════════════════════════
# DÉTECTION
# ═══════════════════════════════════════════════════════════════

def preprocess_det(img):
    """Prépare l'image pour det.onnx (640x640)"""
    h, w = img.shape[:2]
    ratio = 640 / max(h, w)
    new_h, new_w = int(h * ratio), int(w * ratio)
    
    resized = cv2.resize(img, (new_w, new_h))
    
    # Padding à 640x640
    canvas = np.zeros((640, 640, 3), dtype=np.uint8)
    canvas[:new_h, :new_w] = resized
    
    # Normalize
    canvas = canvas.astype(np.float32) / 255.0
    mean = np.array([0.485, 0.456, 0.406])
    std = np.array([0.229, 0.224, 0.225])
    canvas = (canvas - mean) / std
    
    # HWC → CHW
    canvas = canvas.transpose(2, 0, 1)
    canvas = np.expand_dims(canvas, 0)
    canvas = canvas.astype(np.float32)  # ← FIX dtype
    
    return canvas, ratio

def detect_text_boxes(img):
    """Détecte les zones de texte"""
    inp, ratio = preprocess_det(img)
    out = det_session.run(None, {det_session.get_inputs()[0].name: inp.astype(np.float32)})[0]
    
    # Post-traitement
    prob_map = out[0, 0]
    binary = (prob_map > DET_THRESH).astype(np.uint8)
    
    # Trouver contours
    contours, _ = cv2.findContours(binary, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    
    boxes = []
    for cnt in contours:
        x, y, w, h = cv2.boundingRect(cnt)
        if w < 5 or h < 5:
            continue
        
        # Remettre à l'échelle
        x = int(x / ratio)
        y = int(y / ratio)
        w = int(w / ratio)
        h = int(h / ratio)
        
        boxes.append([x, y, x + w, y + h])
    
    return boxes

# ═══════════════════════════════════════════════════════════════
# RECONNAISSANCE
# ═══════════════════════════════════════════════════════════════

def preprocess_rec(img):
    """Prépare une zone pour rec.onnx"""
    h, w = img.shape[:2]
    ratio = REC_HEIGHT / h
    new_w = int(w * ratio)
    new_w = max(new_w, 10)
    
    resized = cv2.resize(img, (new_w, REC_HEIGHT))
    
    # Normalize
    resized = resized.astype(np.float32) / 255.0
    mean = np.array([0.5, 0.5, 0.5])
    std = np.array([0.5, 0.5, 0.5])
    resized = (resized - mean) / std
    
    # HWC → CHW
    resized = resized.transpose(2, 0, 1)
    resized = resized.astype(np.float32)  # ← FIX dtype
    
    return resized

def ctc_decode(pred):
    """Décode la sortie CTC"""
    indices = np.argmax(pred, axis=1)
    texts = []
    confs = []
    
    prev = -1
    for i, idx in enumerate(indices):
        if idx != prev and idx != 0:  # 0 = blank
            if idx < len(charset):
                texts.append(charset[idx])
                confs.append(pred[i, idx])
        prev = idx
    
    text = "".join(texts)
    conf = np.mean(confs) if confs else 0.0
    
    return text, float(conf)

def recognize_text(img, box):
    """Reconnaît le texte dans une zone"""
    x1, y1, x2, y2 = box
    crop = img[y1:y2, x1:x2]
    
    if crop.size == 0:
        return "", 0.0
    
    inp = preprocess_rec(crop)
    inp = np.expand_dims(inp, 0)
    
    out = rec_session.run(None, {rec_session.get_inputs()[0].name: inp.astype(np.float32)})[0]
    
    # out shape: [1, T, 18710]
    pred = out[0]
    
    text, conf = ctc_decode(pred)
    return text, conf

# ═══════════════════════════════════════════════════════════════
# MAIN
# ═══════════════════════════════════════════════════════════════

def run_ocr(image_path):
    """OCR complet sur une image"""
    img = cv2.imread(image_path)
    if img is None:
        return {"status": "error", "message": f"Image introuvable: {image_path}"}
    
    boxes = detect_text_boxes(img)
    
    results = []
    for box in boxes:
        text, conf = recognize_text(img, box)
        if text.strip():
            results.append({
                "text": text,
                "confidence": round(conf, 3),
                "box": box
            })
    
    # Texte complet
    full_text = " ".join([r["text"] for r in results])
    
    return {
        "status": "success",
        "text": full_text,
        "lines": results,
        "count": len(results)
    }

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(json.dumps({"status": "error", "message": "Usage: python ocr_pipeline.py <image_path>"}))
        sys.exit(1)
    
    result = run_ocr(sys.argv[1])
    print(json.dumps(result, ensure_ascii=False, indent=2))
