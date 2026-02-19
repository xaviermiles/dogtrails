use wasm_bindgen::prelude::*;
use yew::prelude::*;

mod leaflet;

use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use wasm_bindgen::JsCast;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bbox {
    min_lat: f64,
    min_lon: f64,
    max_lat: f64,
    max_lon: f64,
}

impl Bbox {
    fn to_query(&self) -> Vec<(String, String)> {
        vec![
            ("min_lat".to_string(), self.min_lat.to_string()),
            ("min_lon".to_string(), self.min_lon.to_string()),
            ("max_lat".to_string(), self.max_lat.to_string()),
            ("max_lon".to_string(), self.max_lon.to_string()),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Difficulty {
    Easy,
    Moderate,
    Hard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DogFilter {
    AllowedOnly,
    AllowedOrPartial,
    Any,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Effort {
    Easy,
    Steady,
    Hard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Length {
    Short,
    Medium,
    Long,
}

#[derive(Clone, Debug, PartialEq)]
struct Filters {
    effort: Effort,
    length: Length,
    dog: DogFilter,
    difficulty: Option<Difficulty>,
    min_km: f32,
    max_km: f32,
    autorefresh: bool,
    bbox: Bbox,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            effort: Effort::Steady,
            length: Length::Medium,
            dog: DogFilter::AllowedOrPartial,
            difficulty: None,
            min_km: 0.0,
            max_km: 70.0,
            autorefresh: true,
            bbox: Bbox::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ResultsState {
    trails: Vec<Trail>,
    loading: bool,
    error: Option<String>,
}

impl Default for ResultsState {
    fn default() -> Self {
        Self {
            trails: Vec::new(),
            loading: false,
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Trail {
    id: String,
    name: String,
    provider: String,
    location: String,
    distance_km: f32,
    elevation_m: Option<f32>,
    difficulty: Difficulty,
    dog_policy: String,
    dog_notes: Option<String>,
    surface: String,
    map_url: String,
    lat: f64,
    lon: f64,
    #[serde(default)]
    line: Vec<[f64; 2]>,
}

impl Default for Bbox {
    fn default() -> Self {
        Self {
            min_lat: -43.60,
            min_lon: 172.50,
            max_lat: -43.45,
            max_lon: 172.77,
        }
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    yew::Renderer::<App>::new().render();
}

#[function_component(App)]
fn app() -> Html {
    let filters = use_state(Filters::default);
    let results = use_state(ResultsState::default);
    let map_ref = use_node_ref();
    let map_handle = use_mut_ref(|| None::<leaflet::MapHandle>);
    let slider_min = use_state(|| 0.0f32);
    let slider_max = use_state(|| 70.0f32);
    let selected_trail = use_state(|| None::<String>);

    // Keep a ref in sync with the latest filters so the map callback can read it
    // without suffering from stale-closure captures.
    let filters_ref = use_mut_ref(|| (*filters).clone());
    *filters_ref.borrow_mut() = (*filters).clone();

    {
        let filters = filters.clone();
        let filters_ref = filters_ref.clone();
        let map_ref = map_ref.clone();
        let map_handle = map_handle.clone();
        let selected_trail = selected_trail.clone();
        use_effect_with(
            (),
            move |_| {
                if let Some(element) = map_ref.cast::<web_sys::HtmlElement>() {
                    let bbox = (*filters).bbox;
                    let on_select: Rc<dyn Fn(Option<String>)> = {
                        let selected_trail = selected_trail.clone();
                        Rc::new(move |id| {
                            selected_trail.set(id);
                        })
                    };
                    let handle = leaflet::init_map(element, bbox, move |bounds| {
                        let mut next = filters_ref.borrow().clone();
                        next.bbox = bounds;
                        filters.set(next);
                    }, on_select);
                    *map_handle.borrow_mut() = Some(handle);
                }
                || ()
            },
        );
    }

    {
        let results = results.clone();
        use_effect_with(
            (*filters).clone(),
            move |current| {
                if current.autorefresh {
                    fetch_trails(current.clone(), results.clone());
                }
                || ()
            },
        );
    }

    {
        let map_handle = map_handle.clone();
        let trails = (*results).trails.clone();
        use_effect_with(
            trails,
            move |trails| {
                if let Some(ref handle) = *map_handle.borrow() {
                    leaflet::update_markers(handle, trails);
                }
                || ()
            },
        );
    }

    {
        let selected_id = (*selected_trail).clone();
        use_effect_with(
            selected_id,
            move |id| {
                if let Some(id) = id {
                    let code = format!(
                        "document.getElementById('trail-{}')?.scrollIntoView({{behavior:'smooth',block:'center'}})",
                        id
                    );
                    let _ = js_sys::eval(&code);
                }
                || ()
            },
        );
    }

    let on_effort = change_select(filters.clone(), |value, next| {
        next.effort = match value.as_str() {
            "easy" => Effort::Easy,
            "hard" => Effort::Hard,
            _ => Effort::Steady,
        };
    });

    let on_length = change_select(filters.clone(), |value, next| {
        next.length = match value.as_str() {
            "short" => Length::Short,
            "long" => Length::Long,
            _ => Length::Medium,
        };
    });

    let on_dog = change_select(filters.clone(), |value, next| {
        next.dog = match value.as_str() {
            "allowed_only" => DogFilter::AllowedOnly,
            "any" => DogFilter::Any,
            _ => DogFilter::AllowedOrPartial,
        };
    });

    let on_difficulty = change_select(filters.clone(), |value, next| {
        next.difficulty = match value.as_str() {
            "easy" => Some(Difficulty::Easy),
            "moderate" => Some(Difficulty::Moderate),
            "hard" => Some(Difficulty::Hard),
            _ => None,
        };
    });

    let on_min_input = {
        let slider_min = slider_min.clone();
        let slider_max = slider_max.clone();
        Callback::from(move |event: InputEvent| {
            let value = event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                .map(|i| i.value())
                .unwrap_or_default();
            if let Ok(parsed) = value.parse::<f32>() {
                slider_min.set(parsed.min(*slider_max));
            }
        })
    };

    let on_max_input = {
        let slider_min = slider_min.clone();
        let slider_max = slider_max.clone();
        Callback::from(move |event: InputEvent| {
            let value = event
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                .map(|i| i.value())
                .unwrap_or_default();
            if let Ok(parsed) = value.parse::<f32>() {
                slider_max.set(parsed.max(*slider_min));
            }
        })
    };

    let on_min_change = {
        let filters = filters.clone();
        let slider_min = slider_min.clone();
        Callback::from(move |_event: Event| {
            let mut next = (*filters).clone();
            next.min_km = *slider_min;
            filters.set(next);
        })
    };

    let on_max_change = {
        let filters = filters.clone();
        let slider_max = slider_max.clone();
        Callback::from(move |_event: Event| {
            let mut next = (*filters).clone();
            next.max_km = *slider_max;
            filters.set(next);
        })
    };

    let on_autorefresh = {
        let filters = filters.clone();
        Callback::from(move |event: Event| {
            let target = event.target().unwrap();
            let input = target.dyn_into::<web_sys::HtmlInputElement>().unwrap();
            let mut next = (*filters).clone();
            next.autorefresh = input.checked();
            filters.set(next);
        })
    };

    let loading = (*results).loading;
    let error = (*results).error.clone();
    let trails = (*results).trails.clone();
    let min_percent = (*slider_min / 70.0 * 100.0).clamp(0.0, 100.0);
    let max_percent = (*slider_max / 70.0 * 100.0).clamp(0.0, 100.0);
    let fill_style = format!(
        "left: {:.2}%; right: {:.2}%;",
        min_percent,
        (100.0 - max_percent).max(0.0)
    );

    html! {
        <div class="app">
            <header>
                <div>
                    <p class="eyebrow">{"dogtrails"}</p>
                    <h1>{"For you + dog"}</h1>
                </div>
            </header>
            <main>
                <section class="card form-card">
                    <div class="grid">
                        <label>
                            {"Effort"}
                            <select name="effort" onchange={on_effort}>
                                <option value="easy" selected={(*filters).effort == Effort::Easy}>{"Easy"}</option>
                                <option value="steady" selected={(*filters).effort == Effort::Steady}>{"Steady"}</option>
                                <option value="hard" selected={(*filters).effort == Effort::Hard}>{"Hard"}</option>
                            </select>
                        </label>
                        <label>
                            {"Length"}
                            <select name="length" onchange={on_length}>
                                <option value="short" selected={(*filters).length == Length::Short}>{"Short (2-6 km)"}</option>
                                <option value="medium" selected={(*filters).length == Length::Medium}>{"Medium (6-12 km)"}</option>
                                <option value="long" selected={(*filters).length == Length::Long}>{"Long (12-24 km)"}</option>
                            </select>
                        </label>
                        <label>
                            {"Dog access"}
                            <select name="dog" onchange={on_dog}>
                                <option value="allowed_only" selected={(*filters).dog == DogFilter::AllowedOnly}>{"Dogs allowed only"}</option>
                                <option value="allowed_or_partial" selected={(*filters).dog == DogFilter::AllowedOrPartial}>{"Allowed or partial (with notes)"}</option>
                                <option value="any" selected={(*filters).dog == DogFilter::Any}>{"Show all (include no-dog)"}</option>
                            </select>
                        </label>
                        <label>
                            {"Difficulty"}
                            <select name="difficulty" onchange={on_difficulty}>
                                <option value="" selected={(*filters).difficulty.is_none()}>{"Any"}</option>
                                <option value="easy" selected={(*filters).difficulty == Some(Difficulty::Easy)}>{"Easy"}</option>
                                <option value="moderate" selected={(*filters).difficulty == Some(Difficulty::Moderate)}>{"Moderate"}</option>
                                <option value="hard" selected={(*filters).difficulty == Some(Difficulty::Hard)}>{"Hard"}</option>
                            </select>
                        </label>
                        <div class="range-field">
                            <span class="range-label">{"Distance (km)"}</span>
                            <div class="range-values">
                                <span>{*slider_min}</span>
                                <span>{"–"}</span>
                                <span>{*slider_max}</span>
                            </div>
                            <div class="range-sliders">
                                <div class="range-track"></div>
                                <div class="range-fill" style={fill_style}></div>
                                <input class="range-input range-input-min" type="range" min="0" max="70" step="1" value={slider_min.to_string()} oninput={on_min_input} onchange={on_min_change} />
                                <input class="range-input range-input-max" type="range" min="0" max="70" step="1" value={slider_max.to_string()} oninput={on_max_input} onchange={on_max_change} />
                            </div>
                        </div>
                        <label class="checkbox">
                            <input type="checkbox" checked={(*filters).autorefresh} onchange={on_autorefresh} />
                            {"Autorefresh"}
                        </label>
                    </div>
                </section>

                <section class="card map-card">
                    <div class="results-layout">
                        <div class="map-panel">
                            <div id="map" ref={map_ref}></div>
                        </div>
                        <div class="results">
                            {render_results(loading, error, trails, (*selected_trail).clone())}
                        </div>
                    </div>
                </section>
            </main>
        </div>
    }
}

fn render_results(loading: bool, error: Option<String>, trails: Vec<Trail>, selected_id: Option<String>) -> Html {
    if loading {
        return html! { <div class="note">{"Loading trails…"}</div> };
    }
    if let Some(message) = error {
        return html! { <div class="warning">{message}</div> };
    }
    if trails.is_empty() {
        return html! { <div class="warning">{"No trails matched your filters."}</div> };
    }

    html! {
        for trails.iter().map(|trail| {
            let is_selected = selected_id.as_deref() == Some(&trail.id);
            let class = if is_selected { "trail selected" } else { "trail" };
            let warning = if trail.dog_policy != "allowed" {
                html! { <div class="warning">{trail.dog_notes.clone().unwrap_or_else(|| "Dog access has restrictions.".to_string())}</div> }
            } else {
                html! {}
            };
            let distance_label = if trail.distance_km == 0.0 {
                "Unknown".to_string()
            } else {
                format!("{:.1} km", trail.distance_km)
            };
            let elevation_label = if let Some(elevation) = trail.elevation_m {
                format!("{} m", elevation)
            } else {
                "Unknown".to_string()
            };
            html! {
                <article class={class} id={format!("trail-{}", trail.id)}>
                    <h3>{trail.name.clone()}</h3>
                    <dl class="trail-detail">
                        <dt>{"Distance"}</dt>
                        <dd>{distance_label}</dd>
                        <dt>{"Elevation"}</dt>
                        <dd>{elevation_label}</dd>
                        <dt>{"Difficulty"}</dt>
                        <dd>{format_label(&format!("{:?}", trail.difficulty).to_lowercase())}</dd>
                        <dt>{"Dogs"}</dt>
                        <dd>{format_label(&trail.dog_policy)}</dd>
                        <dt>{"Surface"}</dt>
                        <dd>{trail.surface.clone()}</dd>
                        <dt>{"Area"}</dt>
                        <dd>{trail.location.clone()}</dd>
                        <dt>{"Source"}</dt>
                        <dd><a href={trail.map_url.clone()} target="_blank" rel="noreferrer">{trail.provider.clone()}</a></dd>
                        <dt>{"ID"}</dt>
                        <dd>{trail.id.clone()}</dd>
                    </dl>
                    {warning}
                </article>
            }
        })
    }
}

fn change_select(
    state: UseStateHandle<Filters>,
    update: impl Fn(String, &mut Filters) + 'static,
) -> Callback<Event> {
    Callback::from(move |event: Event| {
        let value = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlSelectElement>().ok())
            .map(|input| input.value())
            .unwrap_or_default();
        let mut next = (*state).clone();
        update(value, &mut next);
        state.set(next);
    })
}

fn fetch_trails(filters: Filters, results: UseStateHandle<ResultsState>) {
    wasm_bindgen_futures::spawn_local(async move {
        let mut next = (*results).clone();
        next.loading = true;
        next.error = None;
        results.set(next);

        let mut params = filters.bbox.to_query();
        params.push(("effort".to_string(), to_query_effort(filters.effort)));
        params.push(("length".to_string(), to_query_length(filters.length)));
        params.push(("dog".to_string(), to_query_dog(filters.dog)));
        params.push(("min_km".to_string(), filters.min_km.to_string()));
        params.push(("max_km".to_string(), filters.max_km.to_string()));
        if let Some(difficulty) = filters.difficulty {
            params.push(("difficulty".to_string(), to_query_difficulty(difficulty)));
        }

        let query_string = params
            .iter()
            .map(|(key, value)| format!("{}={}", key, urlencoding::encode(value)))
            .collect::<Vec<_>>()
            .join("&");

        match Request::get(&format!("/api/trails?{}", query_string)).send().await {
            Ok(response) => match response.json::<Vec<Trail>>().await {
                Ok(trails) => {
                    let mut next = (*results).clone();
                    next.trails = trails;
                    next.loading = false;
                    results.set(next);
                }
                Err(err) => {
                    let mut next = (*results).clone();
                    next.loading = false;
                    next.error = Some(err.to_string());
                    results.set(next);
                }
            },
            Err(err) => {
                let mut next = (*results).clone();
                next.loading = false;
                next.error = Some(err.to_string());
                results.set(next);
            }
        }
    });
}

fn to_query_effort(value: Effort) -> String {
    match value {
        Effort::Easy => "easy".to_string(),
        Effort::Steady => "steady".to_string(),
        Effort::Hard => "hard".to_string(),
    }
}

fn to_query_length(value: Length) -> String {
    match value {
        Length::Short => "short".to_string(),
        Length::Medium => "medium".to_string(),
        Length::Long => "long".to_string(),
    }
}

fn to_query_dog(value: DogFilter) -> String {
    match value {
        DogFilter::AllowedOnly => "allowed_only".to_string(),
        DogFilter::AllowedOrPartial => "allowed_or_partial".to_string(),
        DogFilter::Any => "any".to_string(),
    }
}

fn to_query_difficulty(value: Difficulty) -> String {
    match value {
        Difficulty::Easy => "easy".to_string(),
        Difficulty::Moderate => "moderate".to_string(),
        Difficulty::Hard => "hard".to_string(),
    }
}

fn format_label(value: &str) -> String {
    value.replace('_', " ")
}
