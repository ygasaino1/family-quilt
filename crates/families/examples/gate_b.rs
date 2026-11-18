//! Gate B helper. `cargo run -p family-families --example gate_b -- cases` prints the test-case table (markdown);
//! `... -- mockup` prints the derived columns of the mock-up scenario as JSON.
use family_families::derive_families;
use family_families::testing::{build_graph, render, CASES};
use family_store::Sex;

const PEOPLE: &[(&str, Sex)] = &[
    ("Karim", Sex::Male),
    ("Leila", Sex::Female),
    ("Sara", Sex::Female),
    ("Omid", Sex::Male),
    ("Hamid", Sex::Male),
    ("Nasrin", Sex::Female),
    ("Dara", Sex::Male),
    ("Neda", Sex::Female),
    ("Farid", Sex::Male),
    ("Zohreh", Sex::Female),
    ("Reza", Sex::Male),
    ("Parisa", Sex::Female),
    ("Mina", Sex::Female),
    ("Shirin", Sex::Female),
    ("Arman", Sex::Male),
];
const LINKS: &[&str] = &[
    "Karim+Leila:m",
    "Karim>Sara",
    "Leila>Sara",
    "Karim>Omid",
    "Leila>Omid",
    "Hamid+Nasrin:m",
    "Hamid>Dara",
    "Nasrin>Dara",
    "Hamid>Neda:adopted",
    "Nasrin>Neda:adopted",
    "Farid+Zohreh:u",
    "Farid>Mina",
    "Zohreh>Mina",
    "Reza+Parisa:m",
    "Reza>Mina:adopted",
    "Parisa>Mina:adopted",
    "Shirin>Arman",
];

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("cases") => {
            println!("| # | Case | What it shows | Input (parent>child[:kind], A+B:state) | Expected columns |\n|---|---|---|---|---|");
            for (i, c) in CASES.iter().enumerate() {
                let exp = if c.expect.is_empty() { "(none)".to_string() } else { c.expect.iter().map(|e| format!("`{}`", e.replace('|', "\\|"))).collect::<Vec<_>>().join("<br>") };
                let mut extra = String::new();
                if c.expect_warnings > 0 {
                    extra.push_str(&format!("<br>⚠ {} warning", c.expect_warnings));
                }
                if !c.expect_unlinked.is_empty() {
                    extra.push_str(&format!("<br>unlinked: {}", c.expect_unlinked.join(", ")));
                }
                let links = if c.links.is_empty() { "(no links)".to_string() } else { c.links.iter().map(|l| format!("`{l}`")).collect::<Vec<_>>().join(" ") };
                println!("| {} | {} | {} | {} | {}{} |", i + 1, c.name, c.what, links, exp, extra);
            }
        }
        Some("mockup") => {
            let names: Vec<&str> = PEOPLE.iter().map(|p| p.0).collect();
            let mut g = build_graph(&names, LINKS);
            for (i, p) in PEOPLE.iter().enumerate() {
                g.people[i].sex = p.1;
            }
            let d = derive_families(&g);
            for l in render(&d, &g) {
                eprintln!("{l}");
            }
            let fams: Vec<_> = d
                .families
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "parents": f.parents.iter().map(|&i| g.people[i as usize].display_name.clone()).collect::<Vec<_>>(),
                        "state": f.partnership.as_ref().map(|p| format!("{:?}", p.state)),
                        "children": f.children.iter().map(|c| serde_json::json!({"name": g.people[c.child as usize].display_name, "outlined": c.outlined})).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let people: Vec<_> = PEOPLE.iter().map(|p| serde_json::json!({"name": p.0, "sex": format!("{:?}", p.1)})).collect();
            println!("{}", serde_json::json!({"people": people, "families": fams}));
        }
        _ => eprintln!("usage: gate_b cases|mockup"),
    }
}
