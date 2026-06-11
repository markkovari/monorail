//! Leptos CSR dashboard (ADR 0011). Build with `trunk serve` /
//! `trunk build --release`; the release bundle is served by `monorail-sink`.
//!
//! All data crosses the wire as `monorail_core` types — the same structs the
//! pipeline serializes, deserialized in the browser with no codegen step.

mod api;
mod pages;

use leptos::prelude::*;
use monorail_core::metrics;
use monorail_core::telemetry::MonitorSample;
use monorail_core::wire::Envelope;
use wasm_bindgen::prelude::*;
use web_sys::{EventSource, MessageEvent};

use pages::{PlansPage, SessionsPage, SettingsPage};

/// Until multi-rower UI lands, the dashboard watches the default rower id.
const ROWER_ID: &str = "erg-1";

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// Format seconds-per-500m as M:SS.s.
fn format_split(split_s: f32) -> String {
    let minutes = (split_s / 60.0).floor() as u32;
    let seconds = split_s - minutes as f32 * 60.0;
    format!("{minutes}:{seconds:04.1}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Live,
    Sessions,
    Plans,
    Settings,
}

#[component]
fn App() -> impl IntoView {
    let (tab, set_tab) = signal(Tab::Live);
    let (latest, set_latest) = signal::<Option<Envelope<MonitorSample>>>(None);

    // One EventSource for the whole app lifetime (it auto-reconnects);
    // listener closure is intentionally leaked — one dashboard, one
    // subscription, lives as long as the page.
    let source =
        EventSource::new(&format!("/api/v1/live/{ROWER_ID}")).expect("EventSource construction");
    let on_monitor = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(text) = event.data().as_string() else {
            return;
        };
        match serde_json::from_str::<Envelope<MonitorSample>>(&text) {
            Ok(envelope) => set_latest.set(Some(envelope)),
            Err(error) => web_sys::console::warn_1(&format!("bad envelope: {error}").into()),
        }
    });
    source
        .add_event_listener_with_callback("monitor", on_monitor.as_ref().unchecked_ref())
        .expect("listener");
    on_monitor.forget();
    std::mem::forget(source);

    let tab_button = move |this: Tab, label: &'static str| {
        view! {
            <button class:active=move || tab.get() == this on:click=move |_| set_tab.set(this)>
                {label}
            </button>
        }
    };

    view! {
        <header>
            <h1>"monorail"</h1>
            <nav>
                {tab_button(Tab::Live, "Live")}
                {tab_button(Tab::Sessions, "Sessions")}
                {tab_button(Tab::Plans, "Plans")}
                {tab_button(Tab::Settings, "Settings")}
            </nav>
        </header>
        <main>
            {move || match tab.get() {
                Tab::Live => view! { <LivePage latest=latest /> }.into_any(),
                Tab::Sessions => view! { <SessionsPage /> }.into_any(),
                Tab::Plans => view! { <PlansPage /> }.into_any(),
                Tab::Settings => view! { <SettingsPage /> }.into_any(),
            }}
        </main>
    }
}

#[component]
fn LivePage(latest: ReadSignal<Option<Envelope<MonitorSample>>>) -> impl IntoView {
    view! {
        <section>
            <h2>"Live — " <code>{ROWER_ID}</code></h2>
            {move || match latest.get() {
                None => view! { <p>"waiting for data…"</p> }.into_any(),
                Some(env) => {
                    let s = env.payload;
                    let kcal_hr = metrics::pm_kcal_per_hr(s.power_watts as f64);
                    view! {
                        <div class="live-grid">
                            <div class="metric">
                                <strong>{format_split(s.split_s_per_500m)}</strong>
                                <span>"/500m"</span>
                            </div>
                            <div class="metric">
                                <strong>{format!("{:.0}", s.power_watts)}</strong>
                                <span>"watts"</span>
                            </div>
                            <div class="metric">
                                <strong>{format!("{:.1}", s.stroke_rate_spm)}</strong>
                                <span>"spm"</span>
                            </div>
                            <div class="metric">
                                <strong>{format!("{:.0}", s.distance_m)}</strong>
                                <span>"meters"</span>
                            </div>
                            <div class="metric">
                                <strong>{format!("{:.0}", s.elapsed_s)}</strong>
                                <span>"seconds"</span>
                            </div>
                            <div class="metric">
                                <strong>{format!("{kcal_hr:.0}")}</strong>
                                <span>"kcal/hr (PM)"</span>
                            </div>
                            <div class="metric">
                                <strong>
                                    {s
                                        .heart_rate_bpm
                                        .map(|hr| hr.to_string())
                                        .unwrap_or_else(|| "—".to_string())}
                                </strong>
                                <span>"bpm"</span>
                            </div>
                        </div>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}
