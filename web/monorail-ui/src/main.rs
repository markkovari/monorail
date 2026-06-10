//! Leptos CSR dashboard (ADR 0011). Build with `trunk serve` /
//! `trunk build --release`; the release bundle is served by `monorail-sink`.
//!
//! Live data arrives over SSE (`/api/v1/live/{rower}`) as the exact wire
//! envelopes the pipeline publishes — the same `monorail_core` structs,
//! deserialized in the browser with no codegen step in between.

use leptos::prelude::*;
use monorail_core::telemetry::MonitorSample;
use monorail_core::wire::Envelope;
use wasm_bindgen::prelude::*;
use web_sys::{EventSource, MessageEvent};

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

#[component]
fn App() -> impl IntoView {
    let (latest, set_latest) = signal::<Option<Envelope<MonitorSample>>>(None);
    let (connected, set_connected) = signal(false);

    // EventSource auto-reconnects; closures must outlive the component, so
    // they are intentionally leaked (one dashboard, one subscription).
    let source =
        EventSource::new(&format!("/api/v1/live/{ROWER_ID}")).expect("EventSource construction");

    let on_monitor = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(text) = event.data().as_string() else {
            return;
        };
        match serde_json::from_str::<Envelope<MonitorSample>>(&text) {
            Ok(envelope) => {
                set_connected.set(true);
                set_latest.set(Some(envelope));
            }
            Err(error) => web_sys::console::warn_1(&format!("bad envelope: {error}").into()),
        }
    });
    source
        .add_event_listener_with_callback("monitor", on_monitor.as_ref().unchecked_ref())
        .expect("listener");
    on_monitor.forget();
    std::mem::forget(source);

    view! {
        <main>
            <h1>"monorail"</h1>
            <p>
                "rower " <code>{ROWER_ID}</code> " — "
                {move || if connected.get() { "live" } else { "waiting for data…" }}
            </p>
            {move || {
                latest
                    .get()
                    .map(|env| {
                        let s = env.payload;
                        view! {
                            <section>
                                <p><strong>{format_split(s.split_s_per_500m)}</strong> " /500m"</p>
                                <p>{format!("{:.0} W", s.power_watts)}</p>
                                <p>{format!("{:.1} spm", s.stroke_rate_spm)}</p>
                                <p>{format!("{:.0} m", s.distance_m)}</p>
                                <p>{format!("{:.0} s elapsed", s.elapsed_s)}</p>
                                <p>
                                    {s
                                        .heart_rate_bpm
                                        .map(|hr| format!("{hr} bpm"))
                                        .unwrap_or_else(|| "no HR".to_string())}
                                </p>
                            </section>
                        }
                    })
            }}
        </main>
    }
}
