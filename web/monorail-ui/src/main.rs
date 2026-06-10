//! Leptos CSR dashboard shell (ADR 0011). Build with `trunk serve` /
//! `trunk build --release`; the release bundle is served by `monorail-sink`.

use leptos::prelude::*;
use monorail_core::plan::Zone;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    // Proof of shared types: the same `monorail_core` structs the pipeline
    // serializes deserialize here, no codegen step in between.
    let default_zone = Zone::Ut2;

    view! {
        <main>
            <h1>"monorail"</h1>
            <p>"PM5 telemetry, plans, and predictions."</p>
            <p>"Default training zone: " {format!("{default_zone:?}")}</p>
        </main>
    }
}
