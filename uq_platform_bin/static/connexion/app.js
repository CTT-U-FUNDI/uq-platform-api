const tabs = [...document.querySelectorAll('[role="tab"]')];
const panels = [...document.querySelectorAll('[role="tabpanel"]')];
const pending = new Map();
const offerDetails = new Map();
let position = null;
let sequence = 0;

const elements = {
  network: document.querySelector('#network'),
  offers: document.querySelector('#offers'),
  buyStatus: document.querySelector('#buy-status'),
  sellStatus: document.querySelector('#sell-status'),
  mine: document.querySelector('#mine'),
  mineStatus: document.querySelector('#mine-status'),
  trust: document.querySelector('#trust'),
  trustStatus: document.querySelector('#trust-status'),
  dialog: document.querySelector('#offer-dialog'),
  dialogBody: document.querySelector('#dialog-body'),
  locationValue: document.querySelector('#location-value'),
};

function selectTab(tab) {
  tabs.forEach((item) => {
    const selected = item === tab;
    item.setAttribute('aria-selected', String(selected));
    item.toggleAttribute('focusgroupstart', selected);
  });
  panels.forEach((panel) => {
    panel.hidden = panel.id !== `panel-${tab.id.slice(4)}`;
  });
  tab.focus();
  if (tab.id === 'tab-buy') loadNearby();
  if (tab.id === 'tab-mine') loadMine();
  if (tab.id === 'tab-trust') loadTrust();
}

tabs.forEach((tab) => tab.addEventListener('click', () => selectTab(tab)));

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error || 'Erreur Connexion.');
}

function api(method, path, body) {
  return new Promise((resolve, reject) => {
    const bridge = window.UqConnexionBridge;
    if (!bridge || typeof bridge.postMessage !== 'function') {
      reject(new Error('La boutique Connexion doit être ouverte dans l’application UQ Android.'));
      return;
    }
    const id = `${Date.now()}-${sequence++}`;
    pending.set(id, { resolve, reject });
    bridge.postMessage(JSON.stringify({
      id,
      method,
      path,
      body: body == null ? '{}' : JSON.stringify(body),
    }));
    window.setTimeout(() => {
      if (!pending.has(id)) return;
      pending.delete(id);
      reject(new Error('Le serveur Connexion ne répond pas. Réessaie plus tard.'));
    }, 35000);
  });
}

function receiveBridgeMessage(event) {
  let response;
  try {
    response = JSON.parse(event.data);
  } catch {
    return;
  }
  const request = pending.get(response.id);
  if (!request) return;
  pending.delete(response.id);
  elements.network.textContent = response.online === false
    ? 'Hors ligne — dernières données en cache'
    : (response.cached ? `Données en cache${response.message ? ` — ${response.message}` : ''}` : 'En ligne');
  const body = response.body || {};
  if (response.queued) {
    request.resolve({ ...body, queued: true, message: response.message });
  } else if (response.status >= 200 && response.status < 300 && body.status !== 'error') {
    request.resolve(body);
  } else {
    request.reject(new Error(body.message || response.error || 'Connexion serveur indisponible.'));
  }
}

if (window.UqConnexionBridge) {
  window.UqConnexionBridge.onmessage = receiveBridgeMessage;
}
window.addEventListener('load', () => {
  if (window.UqConnexionBridge) {
    window.UqConnexionBridge.onmessage = receiveBridgeMessage;
  } else {
    elements.network.textContent = 'Module Android indisponible — cette page ne simule aucune donnée.';
  }
});

function card(offer) {
  offerDetails.set(offer.id, offer);
  const article = document.createElement('article');
  article.className = 'card';
  const title = document.createElement('h3');
  title.textContent = offer.nom || 'Connexion';
  const description = document.createElement('p');
  description.className = 'muted';
  description.textContent = `${offer.reseau === 'starlink' ? 'Routeur / satellite' : 'Mobile data'}${offer.distance_km != null ? ` · ${Number(offer.distance_km).toFixed(1)} km` : ''}`;
  const metrics = document.createElement('div');
  metrics.className = 'metrics';
  [
    `${Number(offer.debit_mbps || 0)} Mbps`,
    `${Number(offer.latence_ms || 0)} ms`,
    `${Math.round(Number(offer.puissance || 0) * 100)} % puissance`,
    offer.mesure_peu_fraiche ? 'mesure ancienne' : 'mesure récente',
  ].forEach((label) => {
    const badge = document.createElement('span');
    badge.className = 'badge';
    badge.textContent = label;
    metrics.append(badge);
  });
  const warning = document.createElement('p');
  warning.className = 'muted';
  warning.textContent = 'Débit estimé par le vendeur, non garanti à ton emplacement.';
  const actions = document.createElement('div');
  actions.className = 'actions';
  const details = document.createElement('button');
  details.type = 'button';
  details.className = 'secondary';
  details.textContent = 'Détails';
  details.addEventListener('click', () => openDetails(offer.id));
  actions.append(details);
  article.append(title, description, metrics, warning, actions);
  return article;
}

async function loadNearby() {
  elements.buyStatus.classList.remove('error');
  elements.buyStatus.textContent = 'Chargement des offres…';
  let query = '';
  try {
    position ??= await capturePosition(false);
  } catch {}
  if (position) {
    query = `?latitude=${encodeURIComponent(position.latitude)}&longitude=${encodeURIComponent(position.longitude)}`;
  }
  try {
    const result = await api('GET', `/api/connexion/nearby${query}`);
    const offers = [result.en_avant, ...(result.suivants || [])].filter(Boolean);
    elements.offers.replaceChildren(...offers.map(card));
    elements.buyStatus.textContent = offers.length
      ? `${offers.length} offre(s), triées par ${result.trie_par === 'puissance' ? 'puissance' : 'proximité et puissance'}.`
      : 'Aucune connexion publiée près de toi.';
  } catch (error) {
    elements.buyStatus.classList.add('error');
    elements.buyStatus.textContent = errorMessage(error);
    elements.offers.replaceChildren();
  }
}

function openDetails(id) {
  const offer = offerDetails.get(id);
  if (!offer) return;
  elements.dialogBody.replaceChildren();
  const description = document.createElement('p');
  description.textContent = `${offer.nom} — ${offer.reseau}.`;
  const facts = document.createElement('p');
  facts.className = 'muted';
  facts.textContent = `${offer.debit_mbps || 0} Mbps estimés, ${offer.latence_ms || 0} ms, ${offer.avis_total || 0} avis.`;
  elements.dialogBody.append(description, facts);
  document.querySelector('#dialog-title').textContent = offer.nom || 'Détails';
  elements.dialog.showModal();
}

document.querySelector('#close-dialog').addEventListener('click', () => elements.dialog.close());
document.querySelector('#refresh-nearby').addEventListener('click', loadNearby);

function capturePosition(explicit = true) {
  if (!navigator.geolocation) return Promise.reject(new Error('GPS indisponible.'));
  return new Promise((resolve, reject) => {
    navigator.geolocation.getCurrentPosition(
      (value) => {
        position = { latitude: value.coords.latitude, longitude: value.coords.longitude };
        elements.locationValue.textContent = `${position.latitude.toFixed(5)}, ${position.longitude.toFixed(5)}`;
        resolve(position);
      },
      (error) => reject(new Error(error.code === 1 ? 'Position refusée.' : 'Position indisponible.')),
      { enableHighAccuracy: false, timeout: explicit ? 15000 : 8000, maximumAge: 60000 }
    );
  });
}

document.querySelector('#capture-location').addEventListener('click', async () => {
  if (!document.querySelector('#location-consent').checked) {
    elements.locationValue.textContent = 'Donne ton consentement avant de capter la position.';
    return;
  }
  try {
    await capturePosition();
  } catch (error) {
    elements.locationValue.textContent = errorMessage(error);
  }
});

document.querySelector('#publish-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  if (!form.reportValidity()) return;
  if (!document.querySelector('#location-consent').checked || !position) {
    elements.sellStatus.classList.add('error');
    elements.sellStatus.textContent = 'Le consentement et une position sont requis pour publier.';
    return;
  }
  const data = new FormData(form);
  elements.sellStatus.classList.remove('error');
  elements.sellStatus.textContent = 'Publication serveur en cours…';
  const payload = {
    nom: data.get('nom'),
    reseau: data.get('reseau'),
    latitude: position.latitude,
    longitude: position.longitude,
    debit_mbps: Number(data.get('debit_mbps')),
    latence_ms: Number(data.get('latence_ms')),
    signal: Number(data.get('signal')),
    mesuree_a: Math.floor(Date.now() / 1000),
  };
  try {
    const result = await api('POST', '/api/connexion/publish', payload);
    elements.sellStatus.textContent = result.queued ? result.message : (result.message || 'Fiche publiée.');
    await loadMine();
  } catch (error) {
    elements.sellStatus.classList.add('error');
    elements.sellStatus.textContent = errorMessage(error);
  }
});

async function loadMine() {
  elements.mineStatus.classList.remove('error');
  elements.mineStatus.textContent = 'Lecture de ta fiche…';
  try {
    const result = await api('GET', '/api/connexion/mine');
    elements.mine.replaceChildren();
    if (!result.annonce) {
      elements.mineStatus.textContent = 'Aucune fiche publiée sur ce compte.';
      return;
    }
    elements.mineStatus.textContent = result.publiee ? 'Fiche active.' : 'Fiche dépubliée et conservée.';
    const element = card(result.annonce);
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'secondary';
    button.textContent = result.publiee ? 'Dépublier' : 'Republier depuis Vendre';
    button.disabled = !result.publiee;
    button.addEventListener('click', async () => {
      try {
        const response = await api('POST', '/api/connexion/unpublish', {});
        elements.mineStatus.textContent = response.message || 'Fiche retirée.';
        await loadMine();
      } catch (error) {
        elements.mineStatus.classList.add('error');
        elements.mineStatus.textContent = errorMessage(error);
      }
    });
    element.querySelector('.actions')?.append(button);
    elements.mine.append(element);
  } catch (error) {
    elements.mineStatus.classList.add('error');
    elements.mineStatus.textContent = errorMessage(error);
  }
}

document.querySelector('#refresh-mine').addEventListener('click', loadMine);

async function loadTrust() {
  elements.trustStatus.textContent = 'Lecture des avis…';
  elements.trust.replaceChildren();
  try {
    const mine = await api('GET', '/api/connexion/mine');
    if (!mine.annonce) {
      elements.trustStatus.textContent = 'Publie une fiche pour consulter ses avis.';
      return;
    }
    const reviews = await api('GET', `/api/connexion/avis?annonce_id=${encodeURIComponent(mine.annonce.id)}`);
    elements.trustStatus.textContent = `${reviews.avis_total || 0} avis reçu(s). Aucun score Connexion sur 100 n'est inventé.`;
    for (const review of reviews.avis || []) {
      const box = document.createElement('article');
      box.className = 'card';
      const score = document.createElement('strong');
      score.textContent = `${review.note}/5`;
      const text = document.createElement('p');
      text.className = 'muted';
      text.textContent = review.texte || 'Avis sans commentaire.';
      box.append(score, text);
      elements.trust.append(box);
    }
  } catch (error) {
    elements.trustStatus.classList.add('error');
    elements.trustStatus.textContent = errorMessage(error);
  }
}

loadNearby();
