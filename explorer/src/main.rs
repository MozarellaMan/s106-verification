//! Results explorer: London permissions of 10+ homes on a map, as boroughs recorded them in the
//! Planning London Datahub. Data comes from `s106 explore` (see `explorer/README.md`).
//!
//! Leptos draws the page; Leaflet (loaded by index.html) draws the map through the `leaflet`
//! crate's bindings. Everything runs in the browser from static files.

mod site;

use leaflet::{
    CircleMarker, FeatureGroup, GeoJson, LatLng, LayerGroup, Map, MapOptions, PathOptions, Svg,
};
use leptos::html;
use leptos::prelude::*;
use site::{Bundle, Site, Sources, Status};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The decision years shown when the page opens.
const DEFAULT_YEARS: (i32, i32) = (2015, 2024);

/// Before this year the Datahub rarely records tenure (under 80% of homes have one).
const TENURE_RECORDED_FROM: i32 = 2003;

/// Narrower than this the map and the panel stack (keep in step with style.css).
const NARROW: &str = "(max-width: 820px)";

impl Site {
    /// Affordable share of homes with a recorded tenure; None when no tenure was recorded.
    fn affordable_share(&self) -> Option<f64> {
        (self.known > 0).then(|| self.affordable as f64 / self.known as f64)
    }

    fn name(&self) -> &str {
        self.site.as_deref().unwrap_or("(no site name)")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Colour {
    Affordable,
    Status,
}

/// Affordable share: one hue, light to dark, in even 10-point bins. No bin edge sits on a policy
/// threshold, so no colour reads as a pass or a fail.
const SHARE: [(&str, &str); 5] = [
    ("#d3e4ec", "under 10%"),
    ("#a3c6d6", "10 to 20%"),
    ("#6fa3bd", "20 to 30%"),
    ("#3f7f9f", "30 to 40%"),
    ("#1d5674", "40% or more"),
];
const UNKNOWN: &str = "#9a9aa3";

/// Datahub status: neutral hues, none red or green.
fn status_colour(s: Status) -> &'static str {
    match s {
        Status::Completed => "#4e79a7",
        Status::Commenced => "#e0b445",
        Status::Approved => "#b07aa1",
        Status::Lapsed => "#9c755f",
        Status::Superseded => "#7fb7c9",
        Status::Other => UNKNOWN,
    }
}

const STATUSES: [Status; 6] = [
    Status::Completed,
    Status::Commenced,
    Status::Approved,
    Status::Lapsed,
    Status::Superseded,
    Status::Other,
];

fn colour(site: &Site, by: Colour) -> &'static str {
    match by {
        Colour::Affordable => match site.affordable_share() {
            None => UNKNOWN,
            Some(a) => SHARE[((a * 10.0) as usize).min(4)].0,
        },
        Colour::Status => status_colour(site.status),
    }
}

fn legend(by: Colour) -> Vec<(&'static str, &'static str)> {
    match by {
        Colour::Affordable => SHARE
            .into_iter()
            .chain([(UNKNOWN, "tenure not recorded")])
            .collect(),
        Colour::Status => STATUSES
            .into_iter()
            .map(|s| (status_colour(s), s.label()))
            .collect(),
    }
}

/// Dot radius in pixels: area scales with homes; every site under about 30 homes shares the
/// smallest size so it stays visible.
fn radius(homes: u32) -> f64 {
    (homes as f64).sqrt().max(5.5) / 2.2
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

fn matches_media(query: &str) -> bool {
    window()
        .match_media(query)
        .ok()
        .flatten()
        .is_some_and(|m| m.matches())
}

async fn load() -> Result<(Bundle, JsValue), String> {
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
    // Leaflet reads the boundaries; Rust never looks inside them.
    let text = get("data/boroughs.geojson")
        .await?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    let boroughs = js_sys::JSON::parse(&text).map_err(|_| "boroughs.geojson is not JSON")?;
    Ok((bundle, boroughs))
}

fn main() {
    // Panics go to the browser console instead of failing silently.
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&info.to_string().into())
    }));
    mount_to_body(App);
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
                "Dots sit at the Datahub site centroid. A gap between what was permitted and what is "
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
fn Explorer(bundle: Bundle, boroughs: JsValue) -> impl IntoView {
    let Bundle { sources, sites } = bundle;
    let sites = StoredValue::new(sites);
    let (from, set_from) = signal(DEFAULT_YEARS.0);
    let (to, set_to) = signal(DEFAULT_YEARS.1);
    let (by, set_by) = signal(Colour::Affordable);
    let (borough, set_borough) = signal(String::new());
    // The picked site, and whether the map should move to it (a pick from the list).
    let (selected, set_selected) = signal(None::<(usize, bool)>);
    let years = sites.with_value(|s| {
        let min = s.iter().map(|x| x.year).min().unwrap_or(DEFAULT_YEARS.0);
        let max = s.iter().map(|x| x.year).max().unwrap_or(DEFAULT_YEARS.1);
        (min..=max).collect::<Vec<_>>()
    });
    let boroughs_list = sites.with_value(|s| {
        let mut b: Vec<String> = s.iter().map(|x| x.borough.clone()).collect();
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

    // On a phone the panel sits below the map: bring it into view when a site is picked.
    let panel = NodeRef::<html::Aside>::new();
    Effect::new(move |_| {
        if selected.get().is_some()
            && matches_media(NARROW)
            && let Some(el) = panel.get_untracked()
        {
            el.scroll_into_view();
        }
    });

    let year_options = move |current: ReadSignal<i32>| {
        years
            .iter()
            .map(|&y| view! { <option value=y selected=move || current.get() == y>{y}</option> })
            .collect_view()
    };

    view! {
        <div class="controls">
            <label>"Decided from "
                <select on:change=move |e| { if let Ok(y) = event_target_value(&e).parse() { set_from.set(y) } }>
                    {year_options(from)}
                </select>
            </label>
            <label>"to "
                <select on:change=move |e| { if let Ok(y) = event_target_value(&e).parse() { set_to.set(y) } }>
                    {year_options(to)}
                </select>
            </label>
            <label>"Colour by "
                <select on:change=move |e| set_by.set(if event_target_value(&e) == "status" { Colour::Status } else { Colour::Affordable })>
                    <option value="affordable">"Affordable share"</option>
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
                <MapView sites visible by boroughs selected on_pick=set_selected />
                <div class="legend">
                    {move || legend(by.get()).into_iter().map(|(c, t)| view! {
                        <span><i style:background=c></i>{t}</span>
                    }).collect_view()}
                </div>
                <p class="hint">
                    {move || match by.get() {
                        Colour::Affordable => "Affordable share is of homes (units) with a recorded tenure, on the current Datahub record.",
                        Colour::Status => "Status as on the current Datahub record.",
                    }}
                    {move || (from.get().min(to.get()) < TENURE_RECORDED_FROM).then_some(
                        " Tenure is rarely recorded for decisions before 2003, so earlier sites mostly show as not recorded.")}
                </p>
                <SizeKey />
            </div>
            <aside node_ref=panel>
                <Detail sites selected />
                <Largest sites visible on_pick=set_selected />
            </aside>
        </div>
        <Footer sources />
    }
}

/// Totals over the sites in view.
#[derive(Clone, Copy, Default, PartialEq)]
struct Totals {
    homes: u64,
    known: u64,
    affordable: u64,
    completed: u64,
}

#[component]
fn Stats(sites: StoredValue<Vec<Site>>, visible: Memo<Vec<usize>>) -> impl IntoView {
    let totals = Memo::new(move |_| {
        sites.with_value(|s| {
            visible.with(|v| {
                v.iter()
                    .map(|&i| &s[i])
                    .fold(Totals::default(), |t, x| Totals {
                        homes: t.homes + x.homes as u64,
                        known: t.known + x.known as u64,
                        affordable: t.affordable + x.affordable as u64,
                        completed: t.completed + x.completed as u64,
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
            {stat("Permissions", Signal::derive(move || thousands(visible.with(|v| v.len()) as u64)))}
            {stat("Homes on current records", Signal::derive(move || thousands(totals.get().homes)))}
            {stat("Affordable, of recorded tenure", Signal::derive(move || { let t = totals.get(); percent(t.affordable, t.known) }))}
            {stat("Tenure not recorded", Signal::derive(move || { let t = totals.get(); percent(t.homes - t.known, t.homes) }))}
            {stat("With a completion date", Signal::derive(move || thousands(totals.get().completed)))}
        </div>
        <p class="hint">
            "Recent permissions have had little time to be built, so a low completion count for recent "
            "years is mostly time, not shortfall."
        </p>
    }
}

#[component]
fn MapView(
    sites: StoredValue<Vec<Site>>,
    visible: Memo<Vec<usize>>,
    by: ReadSignal<Colour>,
    boroughs: JsValue,
    selected: ReadSignal<Option<(usize, bool)>>,
    on_pick: WriteSignal<Option<(usize, bool)>>,
) -> impl IntoView {
    let node = NodeRef::<html::Div>::new();
    // The map, its dots, and the ring that marks the picked site.
    let layers = StoredValue::new_local(None::<(Map, LayerGroup, LayerGroup)>);

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
        let outlines = geo_json(&boroughs, &options);
        outlines.add_to(&map);
        map.fit_bounds(&outlines.unchecked_ref::<FeatureGroup>().get_bounds());
        let dots = LayerGroup::new();
        dots.add_to(&map);
        let ring = LayerGroup::new();
        ring.add_to(&map);
        layers.set_value(Some((map, dots, ring)));
    });

    // Redraw the dots whenever the filters or the colouring change.
    Effect::new(move |_| {
        let by = by.get();
        let mut keep = visible.get();
        // Largest first, so smaller sites draw on top and stay clickable.
        sites.with_value(|s| keep.sort_by_key(|&i| std::cmp::Reverse(s[i].homes)));
        layers.with_value(|l| {
            let Some((_, dots, _)) = l else { return };
            dots.clear_layers();
            sites.with_value(|s| {
                for i in keep {
                    let x = &s[i];
                    let style = PathOptions::new();
                    style.set_fill_color(colour(x, by).into());
                    style.set_fill_opacity(0.9);
                    style.set_color("#33424a".into());
                    style.set_opacity(0.6);
                    style.set_weight(0.5);
                    let marker = CircleMarker::new_with_options(&LatLng::new(x.lat, x.lon), &style);
                    marker.set_radius(radius(x.homes));
                    let tip = format!(
                        "{} {}: {} homes",
                        x.borough,
                        x.reference,
                        thousands(x.homes as u64)
                    );
                    marker.unchecked_ref::<Tooltipped>().bind_tooltip(&tip);
                    let pick = Closure::<dyn Fn()>::new(move || on_pick.set(Some((i, false))));
                    marker.on("click", &pick.into_js_value());
                    dots.add_layer(&marker);
                }
            });
        });
    });

    // Ring the picked site; move the map to it when it was picked from the list.
    Effect::new(move |_| {
        let picked = selected.get();
        layers.with_value(|l| {
            let Some((map, _, ring)) = l else { return };
            ring.clear_layers();
            let Some((i, go)) = picked else { return };
            sites.with_value(|s| {
                let x = &s[i];
                let at = LatLng::new(x.lat, x.lon);
                let dark = matches_media("(prefers-color-scheme: dark)");
                let style = PathOptions::new();
                style.set_fill(false);
                style.set_color((if dark { "#ffffff" } else { "#111111" }).into());
                style.set_weight(2.5);
                style.set_interactive(false);
                let marker = CircleMarker::new_with_options(&at, &style);
                marker.set_radius(radius(x.homes) + 4.0);
                ring.add_layer(&marker);
                if go {
                    map.set_view(&at, map.get_zoom().max(13.0));
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

#[component]
fn Detail(
    sites: StoredValue<Vec<Site>>,
    selected: ReadSignal<Option<(usize, bool)>>,
) -> impl IntoView {
    move || {
        let Some((i, _)) = selected.get() else {
            return view! {
                <h2>"Pick a site"</h2>
                <p class="hint">"Click a dot or a row below to see what is known about each stage of that permission. Stages with no data yet are shown as not collected."</p>
            }
            .into_any();
        };
        let x = sites.with_value(|s| s[i].clone());
        let current = match x.affordable_share() {
            Some(a) => format!(
                "{} homes, {} affordable ({:.0}% of recorded tenure). Status: {}",
                thousands(x.homes as u64),
                thousands(x.affordable as u64),
                100.0 * a,
                x.status.label()
            ),
            None => format!(
                "{} homes, tenure not recorded. Status: {}",
                thousands(x.homes as u64),
                x.status.label()
            ),
        };
        let completion = if x.completed > 0 {
            let date = x
                .completion_date
                .as_deref()
                .map(|d| format!(", completion date on record {d}"))
                .unwrap_or_default();
            format!(
                "{} of {} homes have a completion date{date}",
                thousands(x.completed as u64),
                thousands(x.homes as u64)
            )
        } else {
            "No completion date recorded".into()
        };
        let as_of = format!("current Datahub record, as of {}", x.updated);
        view! {
            <h2>{x.name().to_string()}</h2>
            <div class="ref">{format!("{} · {} · decided {}", x.borough, x.reference, x.year)}</div>
            <p class="current">{current}<span class="src">{as_of.clone()}</span></p>
            <p class="hint">"The Datahub keeps one record per permission and updates it in place, so this is the record as it stands now, not what was decided."</p>
            <div class="lineage">
                <Stage name="Target" found=false body="Not collected" source="policy target" />
                <Stage name="Offer" found=false body="Not collected" source="committee report" />
                <Stage name="Decision" found=false body="Not collected" source="committee report, decision notice" />
                <Stage name="Obligation" found=false body="Not collected" source="S106 agreement" />
                <Stage name="Variation" found=false body="Not collected" source="deeds of variation, section 73" />
                <Stage name="Completion" found={x.completed > 0} body=completion source=as_of />
            </div>
        }
        .into_any()
    }
}

#[component]
fn Stage(
    name: &'static str,
    found: bool,
    #[prop(into)] body: String,
    #[prop(into)] source: String,
) -> impl IntoView {
    view! {
        <div class="stage">
            <div class="name">{name}</div>
            <div class="chip" class:found=found>{body}<span class="src">{source}</span></div>
        </div>
    }
}

/// Three reference dots, drawn with the same radius as the map's.
#[component]
fn SizeKey() -> impl IntoView {
    view! {
        <div class="legend sizes">
            {[50, 250, 1000].into_iter().map(|h| {
                let d = format!("{:.1}px", 2.0 * radius(h));
                view! { <span><i class="ring" style:width=d.clone() style:height=d></i>{format!("{} homes", thousands(h as u64))}</span> }
            }).collect_view()}
            <span>"Dot area scales with homes; sites under about 30 homes share the smallest size."</span>
        </div>
    }
}

#[component]
fn Largest(
    sites: StoredValue<Vec<Site>>,
    visible: Memo<Vec<usize>>,
    on_pick: WriteSignal<Option<(usize, bool)>>,
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
                    <button on:click=move |_| on_pick.set(Some((i, true)))>
                        <span>{x.name().to_string()} " " <span class="ref">{format!("{} {}", x.borough, x.reference)}</span></span>
                        <span class="n">{thousands(x.homes as u64)}</span>
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn Footer(sources: Sources) -> impl IntoView {
    let left_out = format!(
        "Left out of {} Datahub records: {} not permitted, {} permitted with no decision year, \
         {} wholly superseded by a later permission, {} without a site centroid.",
        thousands(sources.records),
        thousands(sources.not_permitted),
        thousands(sources.no_decision_year),
        thousands(sources.wholly_superseded),
        thousands(sources.without_centroid),
    );
    // Paths in the (private) data repository, kept for provenance but out of the reader's way.
    let provenance = format!(
        "{}\n{}\n{}",
        sources.snapshot, sources.applications, sources.boundaries
    );
    view! {
        <footer>
            <p>
                {left_out}
                " Completion dates are missing for about 8% of homes on records marked Completed "
                "(permitted since 2010), and outline and reserved-matters records for the same site can "
                "both carry completed homes (up to 4,114 counted twice)."
            </p>
            <p title=provenance>
                "Planning London Datahub, Greater London Authority, as on " {sources.snapshot_date}
                " (Open Government Licence v3). Borough boundaries: Office for National Statistics, "
                "December 2023 (Open Government Licence v3). Contains OS data © Crown copyright and "
                "database right 2023."
            </p>
        </footer>
    }
}
