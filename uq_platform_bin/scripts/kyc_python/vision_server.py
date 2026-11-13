"""Moteur de vision du KYC : process persistant, protocole JSON ligne par ligne.

Chaque requete sur stdin est une ligne JSON, chaque reponse est une ligne JSON
sur stdout. Le detecteur YuNet n'est charge qu'une fois au demarrage, ce qui
fait passer une image de guidage de ~900 ms (process neuf) a ~85 ms.

Commandes :
  ping                          -> {ready, cv2, yunet}
  challenge {text, width}       -> JPEG du defi + rect normalise ou le coller
  face {img, crop?}             -> metrics du visage pour le guide temps reel
  doc {img, warp?}              -> quad du document + qualite, redresse si demande
  check {frames, template, ...} -> verification finale de la sequence de vivacite

stdout ne porte QUE le protocole ; les diagnostics partent sur stderr.
"""

import base64
import json
import math
import sys

import cv2
import numpy as np
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent.parent
YUNET_PATH = BASE_DIR / "models" / "face" / "face_detection_yunet_2023mar.onnx"

# Sortie de cv2.FaceDetectorYN (build 4.11) : bbox (4) + 5 points (10) + score.
# Les angles tete/nas/roulis ne sont PAS exposes par cette API : ils sont derives
# des points, ce qui suffit au guidage et reste coherent avec la verification.
IDX = {
    "x": 0, "y": 1, "w": 2, "h": 3,
    "eye_r": (4, 5), "eye_l": (6, 7), "nose": (8, 9),
    "mouth_r": (10, 11), "mouth_l": (12, 13),
    "score": 14,
}

MIN_SCORE = 0.6
# Zone de visee du selfie : le visage doit remplir environ un tiers de la largeur.
FACE_SIZE_MIN = 0.22
FACE_SIZE_MAX = 0.55


def _decode(img_b64):
    raw = img_b64.split(",")[-1] if img_b64.startswith("data:") else img_b64
    buf = np.frombuffer(base64.b64decode(raw), np.uint8)
    image = cv2.imdecode(buf, cv2.IMREAD_COLOR)
    return image


def _encode(image, quality=78):
    ok, buf = cv2.imencode(".jpg", image, [cv2.IMWRITE_JPEG_QUALITY, quality])
    if not ok:
        return ""
    return base64.b64encode(buf.tobytes()).decode("ascii")


def _lumiere(gray):
    return float(np.mean(gray))


def _netnete(gray):
    return float(cv2.Laplacian(gray, cv2.CV_32F).var())


def _reflets(gray, seuil=242):
    return float(np.mean(gray >= seuil))


_detector = cv2.FaceDetectorYN.create(str(YUNET_PATH), "", (320, 320), score_threshold=0.55)


def _plus_grand_visage(faces):
    if faces is None or len(faces) == 0:
        return None
    return max(faces, key=lambda f: float(f[IDX["w"]]) * float(f[IDX["h"]]))


def cmd_ping(_):
    return {"ready": True, "cv2": cv2.__version__, "yunet": str(YUNET_PATH), "exists": YUNET_PATH.exists()}


def cmd_challenge(args):
    """Dessine le defi (nonce + horodatage) et fixe la geometrie de collage.

    Contrat partage avec le client : le gabarit est pose centre en (cx, cy) de
    l'image, etale sur une fraction w de la largeur. La hauteur en decoule via
    `ratio`. La verification retrouve donc la bande exacte sans OCR.
    """
    texte = str(args.get("text", ""))[:64]
    largeur = int(args.get("width", 640))
    hauteur = max(48, int(largeur * 0.22))
    toile = np.zeros((hauteur, largeur, 3), np.uint8)
    scale = 0.9 * largeur / 1000.0
    epaisseur = max(2, int(hauteur * 0.045))
    taille = cv2.getTextSize(texte, cv2.FONT_HERSHEY_SIMPLEX, scale, epaisseur)[0]
    coin = (max(4, (largeur - taille[0]) // 2), max(epaisseur, (hauteur + taille[1]) // 2))
    cv2.putText(toile, texte, coin, cv2.FONT_HERSHEY_SIMPLEX, scale,
                (255, 255, 255), epaisseur, cv2.LINE_AA)
    return {
        "jpeg": _encode(toile, 100),
        "largeur": largeur,
        "hauteur": hauteur,
        "ratio": round(hauteur / largeur, 4),
        "rect": {"cx": 0.5, "cy": 0.82, "w": 0.88},
    }


def _points(visage):
    return {
        "oeil_droit": (float(visage[IDX["eye_r"][0]]), float(visage[IDX["eye_r"][1]])),
        "oeil_gauche": (float(visage[IDX["eye_l"][0]]), float(visage[IDX["eye_l"][1]])),
        "nez": (float(visage[IDX["nose"][0]]), float(visage[IDX["nose"][1]])),
        "bouche": ((float(visage[IDX["mouth_r"][0]]) + float(visage[IDX["mouth_l"][0]])) / 2,
                   (float(visage[IDX["mouth_r"][1]]) + float(visage[IDX["mouth_l"][1]])) / 2),
    }


def cmd_face(args):
    image = _decode(args.get("img", ""))
    if image is None:
        raise ValueError("image illegible")
    h, w = image.shape[:2]
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)

    _detector.setInputSize((w, h))
    _, faces = _detector.detect(image)
    visage = _plus_grand_visage(faces)
    if visage is None:
        # Lumiere et reflets restent mesures sans visage : le guide doit pouvoir
        # dire « eclairer » ou « ecarte-toi de la fenetre » avant la detection.
        return {
            "detected": False,
            "frame": {"w": w, "h": h},
            "light": round(_lumiere(gray), 1),
            "blur": round(_netnete(gray), 1),
            "glare": round(_reflets(gray), 4),
        }

    x, y, fw, fh = (float(visage[i]) for i in (IDX["x"], IDX["y"], IDX["w"], IDX["h"]))
    cx, cy = (x + fw / 2) / w, (y + fh / 2) / h
    pts = _points(visage)
    ecart_yeux = math.dist(pts["oeil_droit"], pts["oeil_gauche"]) or 1.0
    ligne_yeux_y = (pts["oeil_droit"][1] + pts["oeil_gauche"][1]) / 2

    # Roulis : pente de la ligne des yeux (0 = tete droite).
    roulis = math.degrees(math.atan2(pts["oeil_gauche"][1] - pts["oeil_droit"][1],
                                     pts["oeil_gauche"][0] - pts["oeil_droit"][0]))
    # Lacet : decompensation du nez par rapport au milieu des yeux, en ecart d'yeux.
    lacet = (pts["nez"][0] - (pts["oeil_droit"][0] + pts["oeil_gauche"][0]) / 2) / ecart_yeux
    # Tangage : le nez doit tomber entre les yeux et la bouche, aux deux tiers environ.
    bande = pts["bouche"][1] - ligne_yeux_y
    tangage = (pts["nez"][1] - ligne_yeux_y) / bande if abs(bande) > 1 else 0.0

    return {
        "detected": True,
        "frame": {"w": w, "h": h},
        "box": {"x": round(x / w, 4), "y": round(y / h, 4), "w": round(fw / w, 4),
                "h": round(fh / h, 4)},
        "center": {"cx": round(cx, 4), "cy": round(cy, 4)},
        "size": round(fw / w, 4),
        "offset": {"dx": round(cx - 0.5, 4), "dy": round(cy - 0.5, 4)},
        "angles": {"roulis": round(roulis, 1), "lacet": round(lacet, 3), "tangage": round(tangage, 3)},
        "frontal": abs(lacet) < 0.22 and 0.55 < tangage < 0.95,
        "yeux_et_bouche": round(bande / max(fh, 1.0), 3),
        "score": round(float(visage[IDX["score"]]), 3),
        "light": round(_lumiere(gray), 1),
        "blur": round(_netnete(gray), 1),
        "glare": round(_reflets(gray), 4),
        "points": {k: [round(v[0] / w, 4), round(v[1] / h, 4)] for k, v in pts.items()},
    }


def _coins_ordonnes(pts):
    """Quatre points -> HL, HR, BR, BL.

    En coords image (y vers le bas) : HL minimise x+y, BR les maximise.
    HR a la plus petite valeur de y-x (x grand, y petit), BL la plus grande.
    """
    pts = np.asarray(pts, dtype="float32").reshape(-1, 2)
    somme = pts.sum(axis=1)
    ecart = pts[:, 0] - pts[:, 1]
    return np.array([pts[np.argmin(somme)], pts[np.argmax(ecart)],
                     pts[np.argmax(somme)], pts[np.argmin(ecart)]], dtype="float32")


def _pente_degres(a, b):
    """Ecart a l'horizontale, replie dans [0, 90]."""
    angle = abs(math.degrees(math.atan2(float(b[1] - a[1]), float(b[0] - a[0]))))
    return angle % 180


def _inclinaison(coins):
    """Pente maximale des deux bords horizontaux, ramenee a un angle aigus."""
    pentes = (_pente_degres(coins[0], coins[1]), _pente_degres(coins[3], coins[2]))
    return max(min(p, 180 - p) for p in pentes)


def cmd_doc(args):
    image = _decode(args.get("img", ""))
    if image is None:
        raise ValueError("image illegible")
    h, w = image.shape[:2]
    gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
    floute = cv2.GaussianBlur(gray, (5, 5), 0)
    contours = cv2.findContours(cv2.Canny(floute, 60, 160), cv2.RETR_LIST, cv2.CHAIN_APPROX_SIMPLE)[-2]

    meilleur, meilleur_air = None, 0.0
    aire_image = float(w * h)
    # 8 contours au plus : au-dela, le tri ne change plus le resultat et on perd du temps.
    for contour in sorted(contours, key=cv2.contourArea, reverse=True)[:8]:
        peri = cv2.arcLength(contour, True)
        approx = cv2.approxPolyDP(contour, 0.02 * peri, True)
        aire = cv2.contourArea(approx)
        if len(approx) != 4 or aire <= 0.06 * aire_image or aire <= meilleur_air:
            continue
        if not cv2.isContourConvex(approx):
            continue
        meilleur, meilleur_air = approx.reshape(4, 2), aire

    base = {
        "frame": {"w": w, "h": h},
        "light": round(_lumiere(gray), 1),
        "blur": round(_netnete(gray), 1),
        "glare": round(_reflets(gray), 4),
        "coverage": round(meilleur_air / aire_image, 4) if meilleur is not None else 0.0,
    }
    if meilleur is None:
        return {**base, "found": False}

    coins = _coins_ordonnes(meilleur)
    normalise = [[round(float(p[0]) / w, 4), round(float(p[1]) / h, 4)] for p in coins]
    large = (np.linalg.norm(coins[1] - coins[0]) + np.linalg.norm(coins[2] - coins[3])) / 2
    haut = (np.linalg.norm(coins[3] - coins[0]) + np.linalg.norm(coins[2] - coins[1])) / 2
    sortie = {
        **base,
        "found": True,
        "quad": normalise,
        "aspect": round(float(large) / max(float(haut), 1.0), 3),
        "tilt": round(float(_inclinaison(coins)), 1),
        "corners_px": [[round(float(p[0]), 1), round(float(p[1]), 1)] for p in coins],
    }
    if args.get("warp"):
        cible_l, cible_h = 860, 540
        destination = np.array([[0, 0], [cible_l, 0], [cible_l, cible_h], [0, cible_h]], "float32")
        matrice = cv2.getPerspectiveTransform(coins, destination)
        redresse = cv2.warpPerspective(image, matrice, (cible_l, cible_h))
        gris = cv2.cvtColor(redresse, cv2.COLOR_BGR2GRAY)
        sortie["warped"] = _encode(redresse, 88)
        sortie["warp"] = {"light": round(_lumiere(gris), 1), "blur": round(_netnete(gris), 1),
                          "glare": round(_reflets(gris), 4)}
    return sortie


def _bande(image, rect, ratio):
    """Pixels de la bande ou le defi doit apparaitre, legerement elargie."""
    h, w = image.shape[:2]
    largeur = float(rect.get("w", 0.88)) * w
    hauteur = largeur * ratio
    cx, cy = float(rect.get("cx", 0.5)) * w, float(rect.get("cy", 0.82)) * h
    x0, x1 = int(max(0, cx - largeur * 0.62)), int(min(w, cx + largeur * 0.62))
    y0, y1 = int(max(0, cy - hauteur * 0.85)), int(min(h, cy + hauteur * 0.85))
    return image[y0:y1, x0:x1]


def _correspondance(image, gabarit, rect, ratio):
    """Similarite (0..1) du gabarit dans la bande attendue, a legeres echelles pres."""
    zone = _bande(image, rect, ratio)
    if zone.size == 0:
        return 0.0
    gris = cv2.cvtColor(zone, cv2.COLOR_BGR2GRAY)
    meilleur = 0.0
    for facteur in (0.9, 0.95, 1.0, 1.05, 1.1):
        cible = (max(8, int(gris.shape[1] * float(rect.get("w", 0.88)) * facteur)),
                 max(4, int(gris.shape[1] * float(rect.get("w", 0.88)) * ratio * facteur)))
        if cible[0] >= gris.shape[1] or cible[1] >= gris.shape[0]:
            continue
        mise = cv2.resize(gabarit, cible, interpolation=cv2.INTER_AREA)
        valeur = float(cv2.matchTemplate(gris, mise, cv2.TM_CCOEFF_NORMED).max())
        meilleur = max(meilleur, valeur)
    return meilleur


def cmd_check(args):
    """Verifie qu'une sequence de vivacite contient bien le defi et un visage vivant."""
    rect = args.get("rect") if isinstance(args.get("rect"), dict) else {}
    ratio = float(args.get("ratio", 0.22))
    gabarit_b64 = args.get("template", "")
    gabarit = _decode(gabarit_b64) if gabarit_b64 else None
    if gabarit is not None:
        gabarit = cv2.cvtColor(gabarit, cv2.COLOR_BGR2GRAY)

    seuil_gabarit = float(args.get("template_min", 0.55))
    frames = args.get("frames") or []
    total = len(frames)
    if total == 0:
        return {"error": "aucune image"}

    hits = 0
    visages = 0
    centres = []
    tailles = []
    scores = []
    flou = []
    for brut in frames:
        image = _decode(brut.get("img", brut) if isinstance(brut, dict) else brut)
        if image is None:
            continue
        gris = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        flou.append(_netnete(gris))
        if gabarit is not None and _correspondance(image, gabarit, rect, ratio) >= seuil_gabarit:
            hits += 1
        h, w = image.shape[:2]
        _detector.setInputSize((w, h))
        _, faces = _detector.detect(image)
        visage = _plus_grand_visage(faces)
        if visage is None:
            continue
        visages += 1
        x, y, fw, fh = (float(visage[i]) for i in (0, 1, 2, 3))
        centres.append(((x + fw / 2) / w, (y + fh / 2) / h))
        tailles.append(fw / w)
        scores.append(float(visage[IDX["score"]]))

    mouvement = 0.0
    etendue = 0.0
    if len(centres) >= 2:
        tab = np.asarray(centres)
        mouvement = float(np.hypot(*(tab.max(axis=0) - tab.min(axis=0))))
        etendue = float(np.std(tab, axis=0).mean())
    variation_taille = float(np.std(tailles)) if len(tailles) >= 2 else 0.0

    ratio_visage = visages / total
    ratio_defi = hits / total
    nettete = float(np.median(flou)) if flou else 0.0
    raisons = []
    if ratio_visage < 0.6:
        raisons.append("visage pas assez present")
    if gabarit is not None and ratio_defi < 0.5:
        raisons.append("defi absent des images")
    if mouvement < 0.03 and variation_taille < 0.008:
        raisons.append("aucun mouvement : image trop statique")
    if nettete < 12.0:
        raisons.append("images trop floues")
    if scores and float(np.mean(scores)) < 0.72:
        raisons.append("visage peu assure")

    return {
        "frames": total,
        "faces": visages,
        "nonce_hits": hits,
        "face_ratio": round(ratio_visage, 3),
        "nonce_ratio": round(ratio_defi, 3),
        "motion": round(mouvement, 4),
        "spread": round(etendue, 4),
        "size_variation": round(variation_taille, 4),
        "blur_median": round(nettete, 1),
        "score_mean": round(float(np.mean(scores)), 3) if scores else 0.0,
        "ok": not raisons,
        "reasons": raisons,
    }


COMMANDES = {"ping": cmd_ping, "challenge": cmd_challenge, "face": cmd_face,
             "doc": cmd_doc, "check": cmd_check}


def main():
    entree = sys.stdin
    for ligne in entree:
        ligne = ligne.strip()
        if not ligne:
            continue
        identifiant = None
        try:
            requete = json.loads(ligne)
            identifiant = requete.get("id")
            commande = COMMANDES.get(str(requete.get("cmd", "")))
            if commande is None:
                raise ValueError("commande inconnue: %s" % requete.get("cmd"))
            reponse = {"id": identifiant, "ok": True, "data": commande(requete)}
        except Exception as erreur:  # le process doit survivre a une image invalide
            reponse = {"id": identifiant, "ok": False, "error": "%s: %s" % (type(erreur).__name__, erreur)}
        sys.stdout.write(json.dumps(reponse) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
