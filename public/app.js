const form = document.getElementById("trail-form");
const resultsEl = document.getElementById("results");
const resultCountEl = document.getElementById("result-count");
const providersEl = document.getElementById("providers");

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
  for (const [key, value] of data.entries()) {
    if (value) {
      params.set(key, value.toString());
    }
  }
  return params.toString();
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
          <span class="tag">${trail.difficulty}</span>
          <span class="tag">${trail.provider}</span>
        </div>
        <div class="trail-meta">
          <span class="tag">Dog policy: ${trail.dog_policy}</span>
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
  const trails = await response.json();
  renderTrails(trails);
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  fetchTrails();
});

fetchProviders();
fetchTrails();
