use super::*;

fn row(id: i64, name: &str, family: &str) -> Row {
    Row {
        id,
        header: Header {
            name: name.to_owned(),
            family: family.to_owned(),
            version: "1".to_owned(),
            pins: 1,
            hash: String::new(),
            catalog: None,
            edition: EDITION.to_owned(),
            lease: None,
            hides_new: false,
            priority: exec::Priority::Regular,
        },
        owner: "owner".to_owned(),
        renewed: 0,
        state: State::Active,
    }
}

#[test]
fn a_file_recording_no_family_owners_gives_the_schema_owner_every_family_it_holds() {
    let mut stored = Stored::default();
    for (id, name, family) in [
        (1, "web-1", "web"),
        (2, "web-2", "web"),
        (3, "ios-1", "ios"),
    ] {
        stored
            .bundles
            .insert(name.to_owned(), row(id, name, family));
    }
    assert_eq!(
        vec![
            Fact::Deployed,
            Fact::Family("ios".to_owned(), OWNER_ID),
            Fact::Family("web".to_owned(), OWNER_ID),
        ],
        legacy(&stored)
    );
    assert!(stored.deployed.registered.is_empty());
    stored.deployed.record(&legacy(&stored));
    assert_eq!(Vec::<Fact>::new(), legacy(&stored));
}

#[test]
fn a_new_file_records_family_owners_from_its_first_open() {
    assert_eq!(vec![Fact::Deployed], legacy(&Stored::default()));
}
