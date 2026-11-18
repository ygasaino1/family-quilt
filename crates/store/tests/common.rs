#![allow(dead_code)]
use family_store::*;

pub const CTX: AccessContext = AccessContext::Everything;

pub fn store() -> Store {
    Store::open_in_memory().unwrap()
}

pub fn person(s: &mut Store, given: &str, family: &str, sex: Sex) -> String {
    s.create_person(&CTX, NewPerson { sex, names: vec![NewName::new(given, family)], ..Default::default() }, &Origin::human()).unwrap().id
}

pub fn parent(s: &mut Store, p: &str, c: &str, kind: LinkKind) -> Result<Relationship> {
    s.add_relationship(&CTX, NewRelationship::parent_child(p, c, kind), &Origin::human())
}

pub fn partner(s: &mut Store, a: &str, b: &str, st: PartnershipState) -> Result<Relationship> {
    s.add_relationship(&CTX, NewRelationship::partnership(a, b, st), &Origin::human())
}
