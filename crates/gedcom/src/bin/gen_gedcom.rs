//! `gen_gedcom [people] [seed]` writes a synthetic GEDCOM tree to stdout.
use family_gedcom::gen::{generate_gedcom, GenOptions};

fn main() {
    let mut a = std::env::args().skip(1);
    let people = a.next().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let seed = a.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    print!("{}", generate_gedcom(&GenOptions { people, seed, ..Default::default() }));
}
