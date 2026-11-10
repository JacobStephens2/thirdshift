//! The blind pairwise page. Before the reader's preference it names no Arm,
//! build, Harness or Model and shows no model call.
use crate::console::filters::escaped as esc;
use crate::evals::experiment::pairs::Side;
use crate::evals::experiment::pairwise::{self, Next, Page as PairPage, Preference};

pub(super) fn page(page: &PairPage, csrf: &str, can_act: bool) -> String {
    let id = esc(&page.experiment);
    let pair = esc(&page.pair);
    let mut html = format!(
        "<p><a href=\"/experiments/{id}\">← Experiment</a></p><div class=\"page-title\"><h1>Blind pair: {} #{}</h1><code>thirdshift experiments {id} --pair {pair} --json</code></div><p>The two changes sit on sides drawn at random for this pair. Which Arm made each, and the Grader panel's calls, stay hidden until you give your preference.</p>",
        esc(&page.task),
        page.number
    );
    html.push_str(&format!(
        "<section aria-label=\"Ticket\"><h2>Ticket</h2>{}</section><section aria-label=\"Acceptance checklist\"><h2>Acceptance checklist</h2>",
        page.ticket.as_ref().map_or_else(
            || "<p>The Ticket is unavailable.</p>".to_owned(),
            |ticket| format!("<pre class=\"pair-ticket\">{}</pre>", esc(ticket))
        )
    ));
    if page.checklist.is_empty() {
        html.push_str("<p>The Ticket has no acceptance checklist.</p>");
    } else {
        html.push_str("<ol>");
        for item in &page.checklist {
            html.push_str(&format!("<li>{}</li>", esc(item)));
        }
        html.push_str("</ol>");
    }
    html.push_str("</section><section class=\"pair-changes\" aria-label=\"Changes\">");
    for (label, side) in [("A", &page.a), ("B", &page.b)] {
        html.push_str(&change(label, side));
    }
    html.push_str("</section>");
    html.push_str(&calls(page, csrf, can_act));
    html.push_str(&revealed(page));
    html
}

/// One side's diff, code before tests, then its evidence.
fn change(label: &str, side: &Side) -> String {
    let mut html = format!("<article aria-label=\"Change {label}\"><h2>Change {label}</h2>");
    if side.diff.is_empty() {
        html.push_str("<p>No retained diff.</p>");
    }
    for file in &side.diff {
        html.push_str(&format!(
            "<details open><summary><code>{}</code>{}</summary><pre class=\"pair-diff\">{}</pre></details>",
            esc(&file.path),
            if file.test { " · test" } else { "" },
            esc(&file.patch)
        ));
    }
    html.push_str(&format!(
        "<h3>Evidence</h3><p>{}</p>",
        esc(&pairwise::evidence(side))
    ));
    let checks: Vec<_> = side
        .base
        .iter()
        .map(|check| ("On the base", check))
        .chain(
            side.cross
                .iter()
                .map(|check| ("On the other change", check)),
        )
        .collect();
    if !checks.is_empty() {
        html.push_str("<ul>");
        for (place, check) in checks {
            html.push_str(&format!(
                "<li>{place}: {} — {}</li>",
                if check.passed { "passed" } else { "failed" },
                esc(&check.reason)
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str("</article>");
    html
}

fn calls(page: &PairPage, csrf: &str, can_act: bool) -> String {
    let base = format!(
        "/experiments/{}/pairs/{}",
        esc(&page.experiment),
        esc(&page.pair)
    );
    let mut html =
        String::from("<section class=\"pair-calls\" aria-label=\"Your calls\"><h2>Your calls</h2>");
    if !page.mine.is_empty() {
        html.push_str("<ul>");
        for record in &page.mine {
            html.push_str(&format!(
                "<li><time>{}</time> · {}</li>",
                esc(&record.recorded_at),
                esc(&pairwise::call(&record.call))
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str(&format!(
        "<p><code>{}</code></p>",
        esc(&pairwise::next_command(page))
    ));
    if !can_act {
        html.push_str("<p>Recording calls needs the act scope.</p></section>");
        return html;
    }
    match page.next {
        Next::Merge => {
            html.push_str(&format!(
                "<p>First, a sealed merge call for each side. It can't be changed once saved.</p><form class=\"judgment-form\" method=\"post\" action=\"{base}/merge\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">",
                esc(csrf)
            ));
            for side in ["a", "b"] {
                let label = side.to_ascii_uppercase();
                html.push_str(&format!(
                    "<fieldset><legend>Would you merge change {label}?</legend><label><input type=\"radio\" name=\"{side}\" value=\"ready\" required> Ready to merge</label> <label><input type=\"radio\" name=\"{side}\" value=\"not-ready\"> Not ready</label></fieldset>"
                ));
            }
            html.push_str("<button>Seal both merge calls</button></form>");
        }
        Next::Preference => {
            html.push_str(&format!(
                "<p>Now your preference, and optionally the rubric.</p><form class=\"judgment-form\" method=\"post\" action=\"{base}/preference\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><fieldset><legend>Which change is better?</legend>",
                esc(csrf)
            ));
            for (_, value, label) in Preference::ALL {
                html.push_str(&format!(
                    "<label><input type=\"radio\" name=\"preference\" value=\"{value}\" required> {label}</label> "
                ));
            }
            html.push_str("</fieldset><fieldset><legend>Rubric (optional)</legend>");
            for criterion in &page.rubric {
                html.push_str(&format!(
                    "<label>{} <select name=\"rubric_{}\"><option value=\"\">Skip</option>",
                    esc(criterion.question),
                    criterion.name
                ));
                for (_, value, label) in Preference::ALL {
                    html.push_str(&format!("<option value=\"{value}\">{label}</option>"));
                }
                html.push_str("</select></label><br>");
            }
            html.push_str("</fieldset><button>Save preference</button></form>");
        }
    }
    html.push_str("</section>");
    html
}

fn revealed(page: &PairPage) -> String {
    let Some(reveal) = &page.revealed else {
        return String::new();
    };
    let mut html = String::from("<section aria-label=\"Revealed\"><h2>Revealed</h2><dl>");
    for (label, arm, setup) in [
        ("A", reveal.a, &reveal.a_setup),
        ("B", reveal.b, &reveal.b_setup),
    ] {
        html.push_str(&format!(
            "<dt>Change {label}</dt><dd>{}</dd>",
            esc(&pairwise::revealed_side(arm, setup))
        ));
    }
    html.push_str(&format!(
        "</dl><h3>Grader panel</h3><p>{}</p><h3>Every reader's calls</h3><ul>",
        reveal.panel.as_ref().map_or_else(
            || "No Grader panel calls are recorded for this pair.".to_owned(),
            |panel| esc(&pairwise::panel_line(panel))
        )
    ));
    for record in &reveal.records {
        html.push_str(&format!(
            "<li>{} · <time>{}</time> · {}</li>",
            esc(&record.reader.to_string()),
            esc(&record.recorded_at),
            esc(&pairwise::call(&record.call))
        ));
    }
    html.push_str("</ul></section>");
    html
}
