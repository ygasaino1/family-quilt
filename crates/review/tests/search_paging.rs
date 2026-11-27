//! Search pages with a keyset cursor (like list_people): nothing is skipped or repeated, a change between pages
//! is reported, and a cursor from a different search is refused.

use family_review::query::{search_page, Scope};
use family_store::*;

const CTX: AccessContext = AccessContext::Everything;

/// Names where "kim" matches at different positions, so the order is by position then id.
fn tree(names: &[&str]) -> Store {
    let mut s = Store::open_in_memory().unwrap();
    for n in names {
        s.create_person(&CTX, NewPerson { sex: Sex::Unknown, names: vec![NewName::full(n)], ..Default::default() }, &Origin::human()).unwrap();
    }
    s
}

fn all_pages(s: &Store, q: &str, limit: usize) -> (Vec<String>, usize) {
    let (mut ids, mut pages, mut after): (Vec<String>, usize, Option<String>) = (vec![], 0, None);
    loop {
        let p = search_page(s, q, limit, Scope::Verified, after.as_deref()).unwrap();
        pages += 1;
        assert!(p.people.len() <= limit);
        ids.extend(p.people.iter().map(|x| x.id.clone()));
        match p.next_cursor {
            Some(c) => after = Some(c),
            None => return (ids, pages),
        }
        assert!(pages < 50, "paging does not end");
    }
}

#[test]
fn pages_cover_every_match_once_in_the_unpaged_order() {
    let s = tree(&["Kim A", "Kim B", "Zed Kim", "Kim C", "Ann Kim", "Kimiya D", "Kim E"]);
    let whole: Vec<String> = search_page(&s, "kim", 100, Scope::Verified, None).unwrap().people.into_iter().map(|p| p.id).collect();
    assert_eq!(whole.len(), 7);
    for limit in [1, 2, 3, 7] {
        let (ids, _) = all_pages(&s, "kim", limit);
        assert_eq!(ids, whole, "limit {limit}");
    }
}

#[test]
fn an_exact_multiple_has_no_empty_last_page_and_a_short_list_has_no_cursor() {
    let s = tree(&["Kim A", "Kim B", "Kim C", "Kim D"]);
    assert_eq!(all_pages(&s, "kim", 2).1, 2, "4 people, 2 a page: two pages, not three");
    let p = search_page(&s, "kim", 10, Scope::Verified, None).unwrap();
    assert!(p.next_cursor.is_none() && p.changed_since_start.is_none(), "no cursor given, no change flag");
    let p = search_page(&s, "nobody", 10, Scope::Verified, None).unwrap();
    assert!(p.people.is_empty() && p.next_cursor.is_none());
}

#[test]
fn a_change_between_pages_is_reported_and_nothing_already_seen_repeats() {
    let mut s = tree(&["Kim A", "Kim B", "Kim C", "Kim D", "Kim E"]);
    let p1 = search_page(&s, "kim", 2, Scope::Verified, None).unwrap();
    let seen: Vec<String> = p1.people.iter().map(|p| p.id.clone()).collect();
    // Someone who sorts BEFORE everything on page 1 (match at position 1, smallest id first is not guaranteed, but
    // an earlier position is) arrives between the pages: an offset cursor would repeat a person here.
    s.create_person(&CTX, NewPerson { sex: Sex::Unknown, names: vec![NewName::full("Kim Newcomer")], ..Default::default() }, &Origin::human()).unwrap();
    let p2 = search_page(&s, "kim", 2, Scope::Verified, p1.next_cursor.as_deref()).unwrap();
    assert_eq!(p2.changed_since_start, Some(true));
    assert!(p2.people.iter().all(|p| !seen.contains(&p.id)), "a person from page 1 came back on page 2");
    // With no change the flag is false.
    let s2 = tree(&["Kim A", "Kim B", "Kim C"]);
    let q1 = search_page(&s2, "kim", 2, Scope::Verified, None).unwrap();
    assert_eq!(search_page(&s2, "kim", 2, Scope::Verified, q1.next_cursor.as_deref()).unwrap().changed_since_start, Some(false));
}

#[test]
fn a_cursor_from_another_search_or_status_or_a_made_up_one_is_refused() {
    let s = tree(&["Kim A", "Kim B", "Kim C"]);
    let c = search_page(&s, "kim", 1, Scope::Verified, None).unwrap().next_cursor.unwrap();
    let wrong_query = search_page(&s, "ann", 1, Scope::Verified, Some(&c)).unwrap_err().to_string();
    assert!(wrong_query.contains("different search"), "{wrong_query}");
    let wrong_status = search_page(&s, "kim", 1, Scope::All, Some(&c)).unwrap_err().to_string();
    assert!(wrong_status.contains("different search"), "{wrong_status}");
    for junk in ["", "abc", "1~2~3", "v~verified~x~id~kim"] {
        let e = search_page(&s, "kim", 1, Scope::Verified, Some(junk)).unwrap_err().to_string();
        assert!(e.contains("not a cursor"), "{junk}: {e}");
    }
}
