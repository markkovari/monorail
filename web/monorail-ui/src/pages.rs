//! Dashboard pages: sessions (with compliance detail), plans (create/push),
//! and athlete settings.

use leptos::prelude::*;
use leptos::task::spawn_local;
use monorail_core::api::{ComplianceRow, PlanRow, SessionSummaryRow};
use monorail_core::metrics::AthleteProfile;
use monorail_core::plan::{Extent, PlanRequest, WorkoutGoal, WorkoutPlan, Zone};
use monorail_core::wire::CommandReply;
use monorail_core::RowerId;

use crate::api;

fn fmt_opt(value: Option<f64>, unit: &str) -> String {
    value
        .map(|v| format!("{v:.0}{unit}"))
        .unwrap_or_else(|| "—".to_string())
}

/// Parse "M:SS" or plain seconds into split seconds.
fn parse_split(input: &str) -> Option<f32> {
    if let Some((m, s)) = input.split_once(':') {
        let minutes: f32 = m.trim().parse().ok()?;
        let seconds: f32 = s.trim().parse().ok()?;
        Some(minutes * 60.0 + seconds)
    } else {
        input.trim().parse().ok()
    }
}

#[component]
pub fn SessionsPage() -> impl IntoView {
    let sessions =
        LocalResource::new(|| api::get_json::<Vec<SessionSummaryRow>>("/api/v1/sessions"));
    let (selected, set_selected) = signal::<Option<String>>(None);
    let compliance = LocalResource::new(move || {
        let session = selected.get();
        async move {
            match session {
                None => Ok(Vec::new()),
                Some(id) => {
                    api::get_json::<Vec<ComplianceRow>>(&format!(
                        "/api/v1/sessions/{id}/compliance"
                    ))
                    .await
                    // 404 just means unscored/unplanned.
                    .or_else(|e| {
                        if e.starts_with("404") {
                            Ok(Vec::new())
                        } else {
                            Err(e)
                        }
                    })
                }
            }
        }
    });

    view! {
        <section>
            <h2>"Sessions"</h2>
            <button on:click=move |_| sessions.refetch()>"refresh"</button>
            {move || match sessions.get().as_deref() {
                None => view! { <p>"loading…"</p> }.into_any(),
                Some(Err(error)) => view! { <p class="error">{error.clone()}</p> }.into_any(),
                Some(Ok(rows)) if rows.is_empty() => {
                    view! { <p>"no sessions yet — row something"</p> }.into_any()
                }
                Some(Ok(rows)) => {
                    let rows = rows.to_vec();
                    view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"started"</th>
                                    <th>"distance"</th>
                                    <th>"duration"</th>
                                    <th>"avg power"</th>
                                    <th>"kcal (PM)"</th>
                                    <th>"kcal (you)"</th>
                                    <th>"strokes"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .into_iter()
                                    .map(|row| {
                                        let id = row.session_id.clone();
                                        view! {
                                            <tr
                                                class:selected=move || {
                                                    selected.get().as_deref() == Some(id.as_str())
                                                }
                                                on:click={
                                                    let id = row.session_id.clone();
                                                    move |_| set_selected.set(Some(id.clone()))
                                                }
                                            >
                                                <td>{row.started_at.clone()}</td>
                                                <td>{fmt_opt(row.last_distance_m, " m")}</td>
                                                <td>{fmt_opt(row.duration_s, " s")}</td>
                                                <td>{fmt_opt(row.avg_power_watts, " W")}</td>
                                                <td>{fmt_opt(row.kcal_pm, "")}</td>
                                                <td>{fmt_opt(row.kcal_adjusted, "")}</td>
                                                <td>{row.strokes}</td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                            </tbody>
                        </table>
                    }
                        .into_any()
                }
            }}
            {move || {
                selected
                    .get()
                    .map(|_| match compliance.get().as_deref() {
                        None => view! { <p>"loading compliance…"</p> }.into_any(),
                        Some(Err(error)) => {
                            view! { <p class="error">{error.clone()}</p> }.into_any()
                        }
                        Some(Ok(rows)) if rows.is_empty() => {
                            view! { <p>"no plan compliance for this session"</p> }.into_any()
                        }
                        Some(Ok(rows)) => {
                            let rows = rows.to_vec();
                            view! {
                                <h3>"Plan compliance"</h3>
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"segment"</th>
                                            <th>"samples"</th>
                                            <th>"split in band"</th>
                                            <th>"spm in band"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {rows
                                            .into_iter()
                                            .map(|c| {
                                                view! {
                                                    <tr>
                                                        <td>{format!("{} ({})", c.segment_index, c.intent)}</td>
                                                        <td>{c.sample_count}</td>
                                                        <td>{band_bar(c.split_in_band)}</td>
                                                        <td>{band_bar(c.spm_in_band)}</td>
                                                    </tr>
                                                }
                                            })
                                            .collect_view()}
                                    </tbody>
                                </table>
                            }
                                .into_any()
                        }
                    })
            }}
        </section>
    }
}

/// Textual percentage bar: "████░░░░░░ 42%".
fn band_bar(fraction: f32) -> String {
    let pct = (fraction * 100.0).round() as u32;
    let filled = (fraction * 10.0).round() as usize;
    format!("{}{} {pct}%", "█".repeat(filled), "░".repeat(10 - filled))
}

#[component]
pub fn PlansPage() -> impl IntoView {
    let plans = LocalResource::new(|| api::get_json::<Vec<PlanRow>>("/api/v1/plans"));
    let (minutes, set_minutes) = signal(40.0_f32);
    let (split_text, set_split_text) = signal("2:00".to_string());
    let (spm, set_spm) = signal(20_u8);
    let (status, set_status) = signal(String::new());

    let create = move |_| {
        let Some(target_split_s) = parse_split(&split_text.get()) else {
            set_status.set("bad split — use M:SS".to_string());
            return;
        };
        let request = PlanRequest {
            rower_id: RowerId::new("erg-1").expect("static id"),
            goal: WorkoutGoal {
                zone: Zone::Ut2,
                extent: Extent::Time {
                    seconds: (minutes.get() * 60.0) as u32,
                },
                target_split_s,
                target_spm: spm.get(),
                hr_cap_bpm: None,
            },
        };
        spawn_local(async move {
            match api::send_json::<_, WorkoutPlan>("POST", "/api/v1/plans", &request).await {
                Ok(plan) => {
                    set_status.set(format!(
                        "created {} ({} segments, {:?})",
                        plan.plan_id,
                        plan.segments.len(),
                        plan.feasibility
                    ));
                    plans.refetch();
                }
                Err(error) => set_status.set(error),
            }
        });
    };

    let push = move |plan_id: String| {
        spawn_local(async move {
            match api::post_empty::<CommandReply>(&format!("/api/v1/plans/{plan_id}/push")).await {
                Ok(CommandReply::Ack { programmed, .. }) => {
                    set_status.set(format!("pushed: {programmed:?}"));
                    plans.refetch();
                }
                Ok(CommandReply::Nack { reason, detail }) => {
                    set_status.set(format!("rower refused: {reason:?} {detail:?}"));
                }
                Err(error) => set_status.set(error),
            }
        });
    };

    view! {
        <section>
            <h2>"Plans"</h2>
            <div class="form">
                <label>
                    "minutes "
                    <input
                        type="number"
                        prop:value=move || minutes.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse() {
                                set_minutes.set(v);
                            }
                        }
                    />
                </label>
                <label>
                    "split /500m "
                    <input
                        prop:value=move || split_text.get()
                        on:input=move |ev| set_split_text.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    "spm "
                    <input
                        type="number"
                        prop:value=move || spm.get().to_string()
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse() {
                                set_spm.set(v);
                            }
                        }
                    />
                </label>
                <button on:click=create>"create UT2 plan"</button>
            </div>
            <p class="status">{move || status.get()}</p>
            {move || match plans.get().as_deref() {
                None => view! { <p>"loading…"</p> }.into_any(),
                Some(Err(error)) => view! { <p class="error">{error.clone()}</p> }.into_any(),
                Some(Ok(rows)) if rows.is_empty() => view! { <p>"no plans yet"</p> }.into_any(),
                Some(Ok(rows)) => {
                    let rows = rows.to_vec();
                    view! {
                        <table>
                            <thead>
                                <tr>
                                    <th>"created"</th>
                                    <th>"plan"</th>
                                    <th>"status"</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                {rows
                                    .into_iter()
                                    .map(|row| {
                                        let id = row.plan_id.clone();
                                        view! {
                                            <tr>
                                                <td>{row.created_at.clone()}</td>
                                                <td><code>{row.plan_id.clone()}</code></td>
                                                <td>{row.status.clone()}</td>
                                                <td>
                                                    <button on:click=move |_| push(id.clone())>
                                                        "push to erg"
                                                    </button>
                                                </td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                            </tbody>
                        </table>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let (weight_text, set_weight_text) = signal(String::new());
    let (status, set_status) = signal(String::new());

    // Load current weight once on mount.
    spawn_local(async move {
        if let Ok(profile) = api::get_json::<AthleteProfile>("/api/v1/athlete").await {
            set_weight_text.set(format!("{}", profile.weight_kg));
            set_status.set("loaded".to_string());
        } else {
            set_status.set("no weight set — adjusted calories are off".to_string());
        }
    });

    let save = move |_| {
        let Ok(weight_kg) = weight_text.get().trim().parse::<f32>() else {
            set_status.set("bad weight".to_string());
            return;
        };
        spawn_local(async move {
            match api::send_json::<_, AthleteProfile>(
                "PUT",
                "/api/v1/athlete",
                &AthleteProfile { weight_kg },
            )
            .await
            {
                Ok(profile) => set_status.set(format!(
                    "saved {} kg — adjusted calories active",
                    profile.weight_kg
                )),
                Err(error) => set_status.set(error),
            }
        });
    };

    view! {
        <section>
            <h2>"Settings"</h2>
            <div class="form">
                <label>
                    "body weight (kg) "
                    <input
                        prop:value=move || weight_text.get()
                        on:input=move |ev| set_weight_text.set(event_target_value(&ev))
                    />
                </label>
                <button on:click=save>"save"</button>
            </div>
            <p class="status">{move || status.get()}</p>
            <p>
                "Weight feeds the Concept2 calorie correction (ADR 0012): the PM5 assumes "
                "a 175 lb / 79.4 kg athlete; your true burn scales with body weight."
            </p>
        </section>
    }
}
