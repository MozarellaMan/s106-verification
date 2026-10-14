//! Results explorer: London permissions of 10+ homes on a map, as boroughs recorded them in the
//! Planning London Datahub. Data comes from `s106 explore` (see `explorer/README.md`).
//!
//! Leptos draws the page; Leaflet (loaded by index.html) draws the map through the `leaflet`
//! crate's bindings. Everything runs in the browser from static files.

use leaflet::{
    CircleMarker, FeatureGroup, GeoJson, LatLng, LayerGroup, Map, MapOptions, PathOptions, Svg,
};
use leptos::html;
use leptos::prelude::*;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

#[derive(Clone, Deserialize)]
struct Site {
    borough: String,
    #[serde(rename = "ref")]
    reference: String,
    site: String,
    status: String,
    year: i32,
    homes: u32,
    affordable: u32,
    known: u32,
    completed: u32,
    completion_date: String,
    updated: String,
    lat: f64,
    lon: f64,
}

impl Site {
    /// Affordable share of homes with a recorded tenure; None when no tenure was recorded.
    fn affordable_share(&self) -> Option<f64> {
        (self.known > 0).then(|| self.affordable as f64 / self.known as f64)
    }

    fn name(&self) -> String {
        if self.site.trim().is_empty() {
            "(no site name)".into()
        } else {
            self.site.clone()
        }
    }
}

#[derive(Clone, Deserialize)]
struct Sources {
    applications: String,
    snapshot: String,
    boundaries: String,
}

#[derive(Clone, Deserialize)]
struct Bundle {
    sources: Sources,
    sites: Vec<Site>,
}

#[derive(Clone, Copy, PartialEq)]
enum Colour {
    Affordable,
    Status,
}

/// Mark colours, mid-tones that read on both the light and dark page.
const LOW: &str = "#d0654c";
const MID: &str = "#d9a73e";
const HIGH: &str = "#2f8f7a";
const UNKNOWN: &str = "#8e8e99";

fn colour(site: &Site, by: Colour) -> &'static str {
    match by {
        Colour::Affordable => match site.affordable_share() {
            None => UNKNOWN,
            Some(a) if a < 0.2 => LOW,
            Some(a) if a < 0.35 => MID,
            Some(_) => HIGH,
        },
        Colour::Status => match site.status.to_lowercase().as_str() {
            "completed" => HIGH,
            "commenced" => MID,
            "lapsed" => LOW,
            _ => UNKNOWN,
        },
    }
}

fn legend(by: Colour) -> [(&'static str, &'static str); 4] {
    match by {
        Colour::Affordable => [
            (LOW, "under 20%"),
            (MID, "20 to 35%"),
            (HIGH, "35% or more"),
            (UNKNOWN, "tenure not recorded"),
        ],
        Colour::Status => [
            (HIGH, "Completed"),
            (MID, "Commenced"),
            (LOW, "Lapsed"),
            (UNKNOWN, "Approved, other or blank"),
        ],
    }
}

/// 12345 -> "12,345"
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        "n/a".into()
    } else {
        format!("{:.1}%", 100.0 * part as f64 / whole as f64)
    }
}

async fn load() -> Result<(Bundle, serde_json::Value), String> {
    let get = |url: &'static str| async move {
        gloo_net::http::Request::get(url)
            .send()
            .await
            .map_err(|e| format!("{url}: {e}"))
    };
    let bundle = get("data/sites.json")
        .await?
        .json::<Bundle>()
        .await
        .map_err(|e| e.to_string())?;
    let boroughs = get("data/boroughs.geojson")
        .await?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok((bundle, boroughs))
}

fn main() {
    console_error_panic_hook_lite();
    mount_to_body(App);
}

/// Panics go to the browser console instead of failing silently.
fn console_error_panic_hook_lite() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
}

#[component]
fn App() -> impl IntoView {
    let data = LocalResource::new(load);
    view! {
        <header>
            <h1>"London housing permissions"</h1>
            <p class="caveat">
                <b>"Datahub figures as boroughs recorded them, not checked against any S106 agreement. "</b>
                "Permissions of 10 or more homes in the Planning London Datahub, final permissions only. "
                "Dots sit at the Datahub site centroid. A gap between what was proposed and what is "
                "recorded as built is a prompt to read the documents, never a finding."
            </p>
        </header>
        {move || match data.get() {
            None => view! { <p class="hint">"Loading sites…"</p> }.into_any(),
            Some(Err(e)) => view! { <p class="hint">"Could not load the data: " {e}</p> }.into_any(),
            Some(Ok((bundle, boroughs))) => view! { <Explorer bundle boroughs /> }.into_any(),
        }}
    }
}

#[component]
fn Explorer(bundle: Bundle, boroughs: serde_json::Value) -> impl IntoView {
    let Bundle { sources, sites } = bundle;
    let sites = StoredValue::new(sites);
    let (from, set_from) = signal(2015);
    let (to, set_to) = signal(2024);
    let (by, set_by) = signal(Colour::Affordable);
    let (borough, set_borough) = signal(String::new());
    let (selected, set_selected) = signal(None::<usize>);
    let (min_year, max_year) = sites.with_value(|s| {
        (
            s.iter().map(|x| x.year).min().unwrap_or(2000),
            s.iter().map(|x| x.year).max().unwrap_or(2025),
        )
    });
    let boroughs_list = sites.with_value(|s| {
        let mut b: Vec<String> = s
            .iter()
            .map(|x| x.borough.clone())
            .filter(|b| !b.is_empty())
            .collect();
        b.sort();
        b.dedup();
        b
    });

    // Indices of the sites the filters keep.
    let visible = Memo::new(move |_| {
        let (a, b) = (from.get().min(to.get()), from.get().max(to.get()));
        let borough = borough.get();
        sites.with_value(|s| {
            s.iter()
                .enumerate()
                .filter(|(_, x)| {
                    x.year >= a && x.year <= b && (borough.is_empty() || x.borough == borough)
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        })
    });

    view! {
        <div class="controls">
            <label>"Decided from "
                <input type="range" min=min_year max=max_year prop:value=from
                    on:input=move |e| set_from.set(event_target_value(&e).parse().unwrap_or(2015)) />
            </label>
            <label>"to "
                <input type="range" min=min_year max=max_year prop:value=to
                    on:input=move |e| set_to.set(event_target_value(&e).parse().unwrap_or(2024)) />
            </label>
            <span class="years">{move || format!("{} to {}", from.get().min(to.get()), from.get().max(to.get()))}</span>
            <label>"Colour by "
                <select on:change=move |e| set_by.set(if event_target_value(&e) == "status" { Colour::Status } else { Colour::Affordable })>
                    <option value="affordable">"Proposed affordable share"</option>
                    <option value="status">"Datahub status"</option>
                </select>
            </label>
            <label>"Borough "
                <select on:change=move |e| set_borough.set(event_target_value(&e))>
                    <option value="">"All London"</option>
                    {boroughs_list.into_iter().map(|b| view! { <option value=b.clone()>{b.clone()}</option> }).collect_view()}
                </select>
            </label>
        </div>
        <Stats sites visible />
        <div class="main">
            <div class="mapcol">
                <MapView sites visible by boroughs on_pick=set_selected />
                <div class="legend">
                    {move || legend(by.get()).into_iter().map(|(c, t)| view! {
                        <span><i style:background=c></i>{t}</span>
                    }).collect_view()}
                    <span>"Dot area scales with proposed homes. Shares are of homes with a recorded tenure."</span>
                </div>
            </div>
            <aside>
                <Detail sites selected />
                <Largest sites visible on_pick=set_selected />
            </aside>
        </div>
        <footer>
            "Data: Planning London Datahub, Greater London Authority (Open Government Licence v3), "
            "snapshot " <code>{sources.snapshot}</code> ", counted by " <code>{sources.applications}</code>
            ". Borough boundaries: ONS (OGL v3), " <code>{sources.boundaries}</code> "."
        </footer>
    }
}

#[component]
fn Stats(sites: StoredValue<Vec<Site>>, visible: Memo<Vec<usize>>) -> impl IntoView {
    let totals = Memo::new(move |_| {
        sites.with_value(|s| {
            visible.with(|v| {
                v.iter()
                    .map(|&i| &s[i])
                    .fold((0u64, 0u64, 0u64, 0u64), |t, x| {
                        (
                            t.0 + x.homes as u64,
                            t.1 + x.known as u64,
                            t.2 + x.affordable as u64,
                            t.3 + x.completed as u64,
                        )
                    })
            })
        })
    });
    let stat = |k: &'static str, v: Signal<String>| {
        view! {
            <div class="stat"><div class="k">{k}</div><div class="v">{v}</div></div>
        }
    };
    view! {
        <div class="stats">
            {stat("Schemes", Signal::derive(move || thousands(visible.with(|v| v.len()) as u64)))}
            {stat("Homes proposed", Signal::derive(move || thousands(totals.get().0)))}
            {stat("Affordable, of recorded tenure", Signal::derive(move || { let t = totals.get(); percent(t.2, t.1) }))}
            {stat("Tenure not recorded", Signal::derive(move || { let t = totals.get(); percent(t.0 - t.1, t.0) }))}
            {stat("Homes with a completion date", Signal::derive(move || thousands(totals.get().3)))}
        </div>
    }
}

#[component]
fn MapView(
    sites: StoredValue<Vec<Site>>,
    visible: Memo<Vec<usize>>,
    by: ReadSignal<Colour>,
    boroughs: serde_json::Value,
    on_pick: WriteSignal<Option<usize>>,
) -> impl IntoView {
    let node = NodeRef::<html::Div>::new();
    let layers = StoredValue::new_local(None::<(Map, LayerGroup)>);

    // Create the map once the element exists.
    Effect::new(move |_| {
        let Some(el) = node.get() else { return };
        if layers.with_value(|l| l.is_some()) {
            return;
        }
        let options = MapOptions::new();
        options.set_prefer_canvas(true);
        options.set_zoom_snap(0.25);
        // Sources are credited in the page footer.
        options.set_attribution_control(false);
        let map = Map::new_with_element(el.unchecked_ref(), &options).expect("Leaflet is loaded");
        map.set_view(&LatLng::new(51.49, -0.11), 10.0);
        // Borough outlines as SVG, so the stylesheet can colour them for light and dark pages.
        let style = PathOptions::new();
        style.set_class_name("borough".into());
        // In the tile pane, below the dots, which draw on a canvas in the overlay pane.
        let renderer = js_sys::Object::new();
        js_sys::Reflect::set(&renderer, &"pane".into(), &"tilePane".into()).unwrap();
        style.set_renderer(Svg::with_options(renderer.unchecked_ref()).into());
        style.set_interactive(false);
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"style".into(), &style).unwrap();
        let outlines = geo_json(&to_js(&boroughs), &options);
        outlines.add_to(&map);
        map.fit_bounds(&outlines.unchecked_ref::<FeatureGroup>().get_bounds());
        let dots = LayerGroup::new();
        dots.add_to(&map);
        layers.set_value(Some((map, dots)));
    });

    // Redraw the dots whenever the filters or the colouring change.
    Effect::new(move |_| {
        let by = by.get();
        let mut keep = visible.get();
        // Largest first, so smaller sites draw on top and stay clickable.
        sites.with_value(|s| keep.sort_by_key(|&i| std::cmp::Reverse(s[i].homes)));
        layers.with_value(|l| {
            let Some((_, dots)) = l else { return };
            dots.clear_layers();
            sites.with_value(|s| {
                for i in keep {
                    let x = &s[i];
                    let style = PathOptions::new();
                    style.set_fill_color(colour(x, by).into());
                    style.set_fill_opacity(0.85);
                    style.set_color("#ffffff".into());
                    style.set_weight(0.6);
                    let marker = CircleMarker::new_with_options(&LatLng::new(x.lat, x.lon), &style);
                    marker.set_radius((x.homes as f64).sqrt().max(5.5) / 2.2);
                    let tip = format!(
                        "{} {}: {} homes",
                        x.borough,
                        x.reference,
                        thousands(x.homes as u64)
                    );
                    marker.unchecked_ref::<Tooltipped>().bind_tooltip(&tip);
                    let pick = Closure::<dyn Fn()>::new(move || on_pick.set(Some(i)));
                    marker.on("click", &pick.into_js_value());
                    dots.add_layer(&marker);
                }
            });
        });
    });

    view! { <div id="map" node_ref=node role="application" aria-label="Map of London permissions"></div> }
}

#[wasm_bindgen]
extern "C" {
    // `L.geoJSON`; the leaflet crate declares it in a private module.
    #[wasm_bindgen(js_namespace = L, js_name = geoJSON)]
    fn geo_json(data: &JsValue, options: &JsValue) -> GeoJson;

    /// A Leaflet layer, typed locally so we can bind `bindTooltip(text)`: the crate's own
    /// binding names a method Leaflet doesn't have.
    #[wasm_bindgen(js_name = Object)]
    type Tooltipped;

    #[wasm_bindgen(method, js_name = bindTooltip)]
    fn bind_tooltip(this: &Tooltipped, content: &str);
}

fn to_js(value: &serde_json::Value) -> JsValue {
    js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL)
}

#[component]
fn Detail(sites: StoredValue<Vec<Site>>, selected: ReadSignal<Option<usize>>) -> impl IntoView {
    move || {
        let Some(i) = selected.get() else {
            return view! {
                <h2>"Pick a site"</h2>
                <p class="hint">"Click a dot or a row below. The panel shows what the Datahub records for each stage of the site's history. Stages the Datahub cannot speak to are shown as not collected rather than hidden."</p>
            }
            .into_any();
        };
        let x = sites.with_value(|s| s[i].clone());
        let snapshot = "Datahub, as recorded by the borough";
        let decision = match x.affordable_share() {
            Some(a) => format!(
                "{} homes, {} affordable ({:.0}% of recorded tenure)",
                thousands(x.homes as u64),
                thousands(x.affordable as u64),
                100.0 * a
            ),
            None => format!("{} homes, tenure not recorded", thousands(x.homes as u64)),
        };
        let completion = if x.completed > 0 {
            let date = if x.completion_date.is_empty() {
                String::new()
            } else {
                format!(", date on record {}", x.completion_date)
            };
            format!(
                "{} of {} homes have a completion date{date}",
                thousands(x.completed as u64),
                thousands(x.homes as u64)
            )
        } else {
            format!(
                "No completion date recorded. Status: {}",
                if x.status.is_empty() {
                    "blank"
                } else {
                    &x.status
                }
            )
        };
        view! {
            <h2>{x.name()}</h2>
            <div class="ref">{format!("{} · {} · decided {} · record updated {}", x.borough, x.reference, x.year, x.updated)}</div>
            <div class="lineage">
                <Stage name="Offer" found=false body="Not collected".into() source="committee report" />
                <Stage name="Decision" found=true body=decision source=snapshot />
                <Stage name="Obligation" found=false body="Not collected".into() source="S106 agreement" />
                <Stage name="Variation" found=false body="Not collected".into() source="deeds of variation, section 73" />
                <Stage name="Completion" found={x.completed > 0} body=completion source=snapshot />
            </div>
        }
        .into_any()
    }
}

#[component]
fn Stage(name: &'static str, found: bool, body: String, source: &'static str) -> impl IntoView {
    view! {
        <div class="stage">
            <div class="name">{name}</div>
            <div class="chip" class:found=found>{body}<span class="src">{source}</span></div>
        </div>
    }
}

#[component]
fn Largest(
    sites: StoredValue<Vec<Site>>,
    visible: Memo<Vec<usize>>,
    on_pick: WriteSignal<Option<usize>>,
) -> impl IntoView {
    let top = Memo::new(move |_| {
        sites.with_value(|s| {
            let mut v = visible.get();
            v.sort_by_key(|&i| std::cmp::Reverse(s[i].homes));
            v.truncate(60);
            v
        })
    });
    view! {
        <div class="hint">{move || format!("Largest {} of {} in view", top.with(|t| t.len()), thousands(visible.with(|v| v.len()) as u64))}</div>
        <div class="list">
            {move || top.get().into_iter().map(|i| {
                let x = sites.with_value(|s| s[i].clone());
                view! {
                    <button on:click=move |_| on_pick.set(Some(i))>
                        <span>{x.name()} " " <span class="ref">{format!("{} {}", x.borough, x.reference)}</span></span>
                        <span class="n">{thousands(x.homes as u64)}</span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}
