//! Synthetic GEDCOM trees for tests and benchmarks (no real data).
//! Includes birth families, adoptions (with and without a known birth family), fosters,
//! remarriages with half-siblings, cousin marriages, deaths, fuzzy dates and Persian names.

use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct GenOptions {
    /// Approximate number of people (the last generation may overshoot a little).
    pub people: usize,
    pub seed: u64,
    /// Percent of children who are adopted into their family.
    pub adoption_pct: u64,
    /// Percent of children who are fostered into a second family.
    pub foster_pct: u64,
    /// Percent of couples where one partner remarries.
    pub remarriage_pct: u64,
    /// Percent of marriages between two descendants of the same founders (cousins).
    pub cousin_marriage_pct: u64,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions { people: 1000, seed: 1, adoption_pct: 4, foster_pct: 2, remarriage_pct: 12, cousin_marriage_pct: 10 }
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
    fn pct(&mut self, p: u64) -> bool {
        self.next(100) < p
    }
}

struct P {
    male: bool,
    given: String,
    surname: String,
    birth: i32,
    famc: Vec<(usize, &'static str)>,
    fams: Vec<usize>,
}

struct F {
    husb: usize,
    wife: usize,
    chil: Vec<usize>,
    marr: i32,
    div: bool,
}

const MALE: [&str; 24] =
    ["Ali", "Reza", "Hamid", "Omid", "Daniel", "David", "John", "Peter", "Karim", "Arash", "Nima", "Kian", "Sam", "Leo", "Max", "Paul", "Hugo", "Ivan", "Omar", "Yusuf", "Tom", "Ben", "Jon", "Rob"];
const FEMALE: [&str; 24] =
    ["Sara", "Mina", "Laleh", "Nora", "Parisa", "Zari", "Maryam", "Anna", "Eva", "Mia", "Lila", "Tara", "Noor", "Yasmin", "Rose", "Iris", "Ava", "Eli", "Dara", "Nina", "Kate", "Jane", "Sue", "Amy"];
const SURNAMES: [&str; 20] =
    ["Adams", "Karimi", "Moore", "Habibi", "Ahmadi", "Rezaei", "Tehrani", "Zand", "Smith", "Brown", "Novak", "Rossi", "Silva", "Kowal", "Berg", "Lund", "Costa", "Vogel", "Fischer", "Nasser"];
const FA_MALE: [&str; 6] = ["علی", "رضا", "حمید", "امید", "کیان", "نیما"];
const FA_FEMALE: [&str; 6] = ["سارا", "مینا", "لاله", "نورا", "پریسا", "مریم"];
const FA_SUR: [&str; 4] = ["احمدی", "کریمی", "رضایی", "حبیبی"];

fn person(r: &mut Rng, male: bool, surname: Option<&str>, birth: i32) -> P {
    let persian = r.pct(5);
    let (given, sur) = if persian {
        (if male { FA_MALE[r.next(6) as usize] } else { FA_FEMALE[r.next(6) as usize] }.to_string(), surname.map(str::to_string).unwrap_or_else(|| FA_SUR[r.next(4) as usize].to_string()))
    } else {
        (if male { MALE[r.next(24) as usize] } else { FEMALE[r.next(24) as usize] }.to_string(), surname.map(str::to_string).unwrap_or_else(|| SURNAMES[r.next(20) as usize].to_string()))
    };
    P { male, given, surname: sur, birth, famc: vec![], fams: vec![] }
}

pub fn generate_gedcom(o: &GenOptions) -> String {
    let mut r = Rng(o.seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut ps: Vec<P> = Vec::new();
    let mut fs: Vec<F> = Vec::new();
    let marry = |ps: &mut Vec<P>, fs: &mut Vec<F>, h: usize, w: usize, year: i32| -> usize {
        fs.push(F { husb: h, wife: w, chil: vec![], marr: year, div: false });
        let fi = fs.len() - 1;
        ps[h].fams.push(fi);
        ps[w].fams.push(fi);
        fi
    };

    let founders = (o.people / 150).clamp(2, 40);
    let mut current: Vec<usize> = Vec::new();
    for _ in 0..founders {
        let by = 1840 + r.next(15) as i32;
        let h = person(&mut r, true, None, by);
        let sur = h.surname.clone();
        ps.push(h);
        let (ws, wb) = (SURNAMES[r.next(20) as usize], ps.last().unwrap().birth + r.next(5) as i32 - 2);
        let w = person(&mut r, false, Some(ws), wb);
        let _ = sur;
        ps.push(w);
        let (hi, wi) = (ps.len() - 2, ps.len() - 1);
        current.push(marry(&mut ps, &mut fs, hi, wi, 1865));
    }

    while ps.len() < o.people && !current.is_empty() {
        let mut next_gen: Vec<usize> = Vec::new();
        let mut gen_children: Vec<(usize, usize)> = Vec::new(); // (child, birth family)
        for &fi in &current {
            let kids = r.next(5) as usize + if r.pct(30) { 1 } else { 0 };
            for k in 0..kids {
                if ps.len() >= o.people + 20 {
                    break;
                }
                let birth = fs[fi].marr + 1 + (k as i32) * 2 + r.next(2) as i32;
                let male = r.pct(50);
                let c = person(&mut r, male, Some(&ps[fs[fi].husb].surname.clone()), birth);
                ps.push(c);
                let ci = ps.len() - 1;
                let roll = r.next(100);
                if roll < o.adoption_pct && !current.is_empty() {
                    // adopted into fi; sometimes with a known birth family
                    ps[ci].famc.push((fi, "adopted"));
                    fs[fi].chil.push(ci);
                    if r.pct(50) {
                        let bf = current[r.next(current.len() as u64) as usize];
                        if bf != fi {
                            ps[ci].famc.push((bf, ""));
                            fs[bf].chil.push(ci);
                        }
                    }
                } else if roll < o.adoption_pct + o.foster_pct {
                    // foster child: birth family is fi, fostered by another family
                    ps[ci].famc.push((fi, ""));
                    fs[fi].chil.push(ci);
                    let ff = current[r.next(current.len() as u64) as usize];
                    if ff != fi {
                        ps[ci].famc.push((ff, "foster"));
                        fs[ff].chil.push(ci);
                    }
                } else {
                    ps[ci].famc.push((fi, ""));
                    fs[fi].chil.push(ci);
                }
                gen_children.push((ci, fi));
                next_gen.push(ci);
            }
        }
        // marriages in the new generation
        let free: Vec<usize> = next_gen.clone();
        let mut new_couples: Vec<usize> = Vec::new();
        let mut i = 0;
        while i < free.len() {
            let p = free[i];
            i += 1;
            if !ps[p].fams.is_empty() || !r.pct(80) {
                continue;
            }
            // cousin / other descendant of the same generation, not a sibling
            let mut partner = None;
            if r.pct(o.cousin_marriage_pct) {
                for _ in 0..10 {
                    let q = free[r.next(free.len() as u64) as usize];
                    let share = ps[p].famc.iter().any(|(f, _)| ps[q].famc.iter().any(|(g, _)| g == f));
                    if q != p && ps[q].male != ps[p].male && ps[q].fams.is_empty() && !share {
                        partner = Some(q);
                        break;
                    }
                }
            }
            let q = partner.unwrap_or_else(|| {
                let birth = ps[p].birth + r.next(7) as i32 - 3;
                let sur = if ps[p].male { SURNAMES[r.next(20) as usize].to_string() } else { ps[p].surname.clone() };
                let m = !ps[p].male;
                let np = person(&mut r, m, Some(&sur), birth);
                ps.push(np);
                ps.len() - 1
            });
            let year = ps[p].birth.max(ps[q].birth) + 22 + r.next(5) as i32;
            let (h, w) = if ps[p].male { (p, q) } else { (q, p) };
            let f1 = marry(&mut ps, &mut fs, h, w, year);
            new_couples.push(f1);
            if r.pct(o.remarriage_pct) {
                fs[f1].div = true;
                let male = !ps[p].male;
                let nb = ps[p].birth + r.next(8) as i32;
                let np = person(&mut r, male, None, nb);
                ps.push(np);
                let n = ps.len() - 1;
                let (h2, w2) = if ps[p].male { (p, n) } else { (n, p) };
                new_couples.push(marry(&mut ps, &mut fs, h2, w2, year + 10 + r.next(8) as i32));
            }
        }
        current = new_couples;
    }

    let mut out = String::new();
    out.push_str("0 HEAD\n1 SOUR gen_gedcom\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    for (i, p) in ps.iter().enumerate() {
        let _ = writeln!(out, "0 @I{}@ INDI\n1 NAME {} /{}/\n1 SEX {}", i + 1, p.given, p.surname, if p.male { "M" } else { "F" });
        let d = match i % 7 {
            0 => format!("ABT {}", p.birth),
            1 => format!("{} MAR {}", 1 + i % 27, p.birth),
            2 => format!("BEF {}", p.birth),
            _ => format!("{}", p.birth),
        };
        let _ = writeln!(out, "1 BIRT\n2 DATE {d}");
        if p.birth < 1930 + (i % 40) as i32 && p.birth + 40 < 2025 && i % 3 != 0 {
            let _ = writeln!(out, "1 DEAT\n2 DATE {}", p.birth + 55 + (i % 30) as i32);
        }
        for (f, pedi) in &p.famc {
            let _ = writeln!(out, "1 FAMC @F{}@", f + 1);
            if !pedi.is_empty() {
                let _ = writeln!(out, "2 PEDI {pedi}");
            }
        }
        for f in &p.fams {
            let _ = writeln!(out, "1 FAMS @F{}@", f + 1);
        }
    }
    for (i, f) in fs.iter().enumerate() {
        let _ = writeln!(out, "0 @F{}@ FAM\n1 HUSB @I{}@\n1 WIFE @I{}@", i + 1, f.husb + 1, f.wife + 1);
        for c in &f.chil {
            let _ = writeln!(out, "1 CHIL @I{}@", c + 1);
        }
        let _ = writeln!(out, "1 MARR\n2 DATE {}", f.marr);
        if f.div {
            out.push_str("1 DIV Y\n");
        }
    }
    out.push_str("0 TRLR\n");
    out
}
