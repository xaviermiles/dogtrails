const form = document.getElementById("trail-form");
const resultsEl = document.getElementById("results");
const resultCountEl = document.getElementById("result-count");
const providersEl = document.getElementById("providers");

const regionBboxes = {
  wellington: { min_lat: -41.35, min_lon: 174.72, max_lat: -41.24, max_lon: 174.82 },
  auckland: { min_lat: -36.93, min_lon: 174.63, max_lat: -36.77, max_lon: 174.84 },
  queenstown: { min_lat: -45.08, min_lon: 168.56, max_lat: -44.95, max_lon: 168.79 },
  christchurch: { min_lat: -43.60, min_lon: 172.50, max_lat: -43.45, max_lon: 172.77 },
};

async function fetchProviders() {
  const response = await fetch("/api/providers");
  const providers = await response.json();
  providersEl.innerHTML = providers
    .map(
      (provider) => `
      <li>
        <strong>${provider.name}</strong><br />
        <span>${provider.api_status}</span><br />
        <em>${provider.notes}</em><br />
        <a href="${provider.website}" target="_blank" rel="noreferrer">${provider.website}</a>
      </li>`
    )
    .join("");
}

function buildQuery() {
  const data = new FormData(form);
  const params = new URLSearchParams();
  const region = data.get("region");
  if (region && regionBboxes[region]) {
    const bbox = regionBboxes[region];
    params.set("min_lat", bbox.min_lat);
    params.set("min_lon", bbox.min_lon);
    params.set("max_lat", bbox.max_lat);
    params.set("max_lon", bbox.max_lon);
  }
  for (const [key, value] of data.entries()) {
    if (key === "region") {
      continue;
    }
    if (value) {
      params.set(key, value.toString());
    }
  }
  return params.toString();
}

function formatLabel(value) {
  return value.replaceAll("_", " ");
}

function renderTrails(trails) {
  resultsEl.innerHTML = trails
    .map(
      (trail) => `
      <article class="trail">
        <h3>${trail.name}</h3>
        <div class="trail-meta">
          <span class="tag">${trail.location}</span>
          <span class="tag">${trail.distance_km.toFixed(1)} km</span>
          <span class="tag">${trail.elevation_m} m gain</span>
          <span class="tag">${formatLabel(trail.difficulty)}</span>
          <span class="tag">${trail.provider}</span>
        </div>
        <div class="trail-meta">
          <span class="tag">Dog policy: ${formatLabel(trail.dog_policy)}</span>
          <span class="tag">Surface: ${trail.surface}</span>
        </div>
        <div>
          <a href="${trail.map_url}" target="_blank" rel="noreferrer">View map</a>
        </div>
        ${
          trail.dog_policy !== "allowed"
            ? `<div class="warning">${trail.dog_notes ?? "Dog access has restrictions."}</div>`
            : ""
        }
      </article>`
    )
    .join("");

  resultCountEl.textContent = `${trails.length} route${trails.length === 1 ? "" : "s"}`;
}

async function fetchTrails() {
  const query = buildQuery();
  const response = await fetch(`/api/trails?${query}`);
  if (!response.ok) {
    resultsEl.innerHTML = `<div class="warning">Could not load live trails. ${
      response.statusText
    }</div>`;
    resultCountEl.textContent = "0 routes";
    return;
  }
  const trails = await response.json();
  renderTrails(trails);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  fetchTrails();
});

fetchProviders();
fetchTrails();
