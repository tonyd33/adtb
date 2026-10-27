//! File scans of CSV and NDJSON files, loads from them, and ad hoc texts with temp relations,
//! driven through the front end.

use adtb_reference::{Abort, Rejection, Value};
use adtb_testkit::fixture::*;

const SCHEMA: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Via = Email { addr: Text } | Phone { num: Text }
type Level = Low | High
type Point = { x: I32, y: I32 }
type Wide = { id: I64, name: Text, nick: Option Text, owner: Key users, level: Level,
              pos: Point, via: Via, amount: Decimal 2, ratio: F64 }
type Small = { id: I64, name: Text }
type Named = { name: Text }
type Identity { id: Key identities, user: Key users } = Google { gid: Text } | Apple { sub: Text }
relation identities : Identity key id generate sequence
relation outs : Small key id
type Nick = { id: I64, nick: Option Text }
type Contact = { id: I64, via: Via }
type Leveled = { id: I64, level: Level }
type Order = { user: Key users, n: I64 }
type Kinded = { user: Key users, kind: Tag Identity, ident: Text }
type Staged = { line: I64, id: I64, name: Text }
type Measure = { x: F64 }
type Pair = { a: I64, b: I64 }
relation out_rejects : RejectedRow key (load, line)
";

fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" }, User { id: users.key(2), name: "bo" }]"#,
    );
    f
}

fn bad_input(r: Result<Value, Abort>) -> (String, i64, String) {
    match r {
        Err(Abort::Reject(Rejection::BadInput {
            source,
            line,
            detail,
        })) => (source, line, detail),
        other => panic!("not bad input: {other:?}"),
    }
}

fn names(f: &Fixture, scan: &str) -> Value {
    f.value(&format!("from r in {scan} select r.name"))
}

// CSV

#[test]
fn a_csv_scan_reads_columns_by_name_in_any_order_and_ignores_others() {
    let mut f = fixture();
    f.files.put("s.csv", "extra,name,id\nx,ann,1\ny,bo,2\n");
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_csv("s.csv", Small)"#)
    );
    assert_eq!(
        ints([1, 2]),
        f.value(r#"from r in read_csv("s.csv", Small) select r.id"#)
    );
}

#[test]
fn a_csv_scan_reads_every_type_mapping_of_a_cell() {
    let mut f = fixture();
    f.files.put(
        "w.csv",
        "id,name,nick,owner,level,pos.x,pos.y,via.tag,via.Email.addr,via.Phone.num,amount,ratio\n\
         1,ann,,1,High,3,4,Email,a@b.example,,12.50,0.5\n",
    );
    let w = f.one(r#"from r in read_csv("w.csv", Wide) select r"#);
    assert_eq!(Some(&int(1)), w.field("id"));
    assert_eq!(Some(&text("ann")), w.field("name"));
    assert_eq!(Some(&Value::none()), w.field("nick"));
    assert_eq!(Some(&key(1)), w.field("owner"));
    assert_eq!(f.value("High"), w.field("level").cloned().expect("a level"));
    assert_eq!(
        f.value("Point { x: 3, y: 4 }"),
        w.field("pos").cloned().expect("a point")
    );
    assert_eq!(
        f.value(r#"Email { addr: "a@b.example" }"#),
        w.field("via").cloned().expect("a via")
    );
    assert_eq!(
        f.value("12.50"),
        w.field("amount").cloned().expect("an amount")
    );
    assert_eq!(Some(&Value::F64(0.5)), w.field("ratio"));
}

#[test]
fn an_empty_unquoted_cell_is_none_and_a_quoted_one_is_some_empty_text() {
    let mut f = fixture();
    f.files.put("n.csv", "id,nick\n1,\n2,\"\"\n3,cy\n");
    assert_eq!(
        set([
            Value::none(),
            Value::some(text("")),
            Value::some(text("cy"))
        ]),
        f.value(r#"from r in read_csv("n.csv", Nick) select r.nick"#)
    );
}

#[test]
fn a_missing_column_of_an_option_field_reads_none() {
    let mut f = fixture();
    f.files.put("n.csv", "id\n1\n");
    assert_eq!(
        set([Value::none()]),
        f.value(r#"from r in read_csv("n.csv", Nick) select r.nick"#)
    );
}

#[test]
fn a_missing_column_is_bad_input_naming_the_file_row_and_column() {
    let mut f = fixture();
    f.files.put("s.csv", "id\n1\n");
    let (source, line, detail) = bad_input(f.eval(r#"read_csv("s.csv", Small)"#));
    assert_eq!("s.csv", source);
    assert_eq!(1, line);
    assert!(detail.contains("`name`"), "{detail}");
}

#[test]
fn a_cell_that_does_not_convert_is_bad_input_naming_its_row_and_column() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann\nx,bo\n");
    let (source, line, detail) = bad_input(f.eval(r#"read_csv("s.csv", Small)"#));
    assert_eq!(("s.csv", 2), (source.as_str(), line));
    assert!(detail.contains("`id`"), "{detail}");
}

#[test]
fn a_value_level_sum_reads_from_a_text_cell() {
    let mut f = fixture();
    f.files
        .put("v.csv", "id,via\n1,\"Phone { num: \"\"555\"\" }\"\n");
    assert_eq!(
        f.value(r#"[Phone { num: "555" }]"#),
        f.value(r#"from r in read_csv("v.csv", Contact) select r.via"#)
    );
}

#[test]
fn tag_columns_set_for_another_constructor_are_a_conversion_failure() {
    let mut f = fixture();
    f.files.put(
        "v.csv",
        "id,via.tag,via.Email.addr,via.Phone.num\n1,Email,a@b.example,555\n",
    );
    let (_, line, detail) = bad_input(f.eval(r#"read_csv("v.csv", Contact)"#));
    assert_eq!(1, line);
    assert!(detail.contains("via.Phone.num"), "{detail}");
}

#[test]
fn a_tag_naming_no_constructor_is_a_conversion_failure() {
    let mut f = fixture();
    f.files.put("l.csv", "id,level\n1,Middle\n");
    bad_input(f.eval(r#"read_csv("l.csv", Leveled)"#));
}

#[test]
fn a_sum_row_type_reads_its_tag_header_and_constructor_columns() {
    let mut f = fixture();
    f.files.put(
        "i.csv",
        "tag,id,user,Google.gid,Apple.sub\nGoogle,7,1,g-1,\nApple,8,2,,a-2\n",
    );
    assert_eq!(
        f.value(
            r#"[Google { id: identities.key(7), user: users.key(1), gid: "g-1" },
                Apple { id: identities.key(8), user: users.key(2), sub: "a-2" }]"#
        ),
        f.value(r#"read_csv("i.csv", Identity)"#)
    );
}

#[test]
fn quoted_cells_hold_separators_quotes_and_line_breaks() {
    let mut f = fixture();
    f.files.put(
        "q.csv",
        "id,name\r\n1,\"a,b\"\r\n2,\"say \"\"hi\"\"\"\r\n3,\"two\nlines\"\r\n",
    );
    assert_eq!(
        texts(["a,b", "say \"hi\"", "two\nlines"]),
        names(&f, r#"read_csv("q.csv", Small)"#)
    );
}

#[test]
fn a_record_with_another_number_of_fields_than_the_header_does_not_convert() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann,extra\n");
    let (_, line, _) = bad_input(f.eval(r#"read_csv("s.csv", Small)"#));
    assert_eq!(1, line);
}

#[test]
fn a_blank_line_in_a_csv_file_is_no_record() {
    let mut f = fixture();
    f.files.put("one.csv", "name\nann\n\n");
    assert_eq!(texts(["ann"]), names(&f, r#"read_csv("one.csv", Named)"#));
    f.files
        .put("two.csv", "\nid,name\n\n1,ann\r\n\r\n2,bo\n\n\n");
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_csv("two.csv", Small)"#)
    );
}

#[test]
fn records_are_numbered_without_the_blank_lines() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n\n1,ann\n\nx,bo\n");
    let (_, line, _) = bad_input(f.eval(r#"read_csv("s.csv", Small)"#));
    assert_eq!(2, line);
}

#[test]
fn a_bare_carriage_return_ends_a_csv_record() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\r1,ann\r2,\"bo\"\r");
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_csv("s.csv", Small)"#)
    );
}

#[test]
fn a_leading_byte_order_mark_is_not_part_of_the_file() {
    let mut f = fixture();
    f.files.put("s.csv", "\u{feff}id,name\n1,ann\n");
    assert_eq!(texts(["ann"]), names(&f, r#"read_csv("s.csv", Small)"#));
    f.files
        .put("s.ndjson", "\u{feff}{\"id\": \"1\", \"name\": \"ann\"}\n");
    assert_eq!(
        texts(["ann"]),
        names(&f, r#"read_ndjson("s.ndjson", Small)"#)
    );
}

#[test]
fn a_byte_order_mark_after_the_start_is_text() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,\u{feff}ann\n");
    assert_eq!(
        texts(["\u{feff}ann"]),
        names(&f, r#"read_csv("s.csv", Small)"#)
    );
}

#[test]
fn a_glob_reads_every_file_it_matches_as_one_relation() {
    let mut f = fixture();
    f.files.put("logs/a.csv", "id,name\n1,ann\n");
    f.files.put("logs/b.csv", "id,name\n2,bo\n");
    f.files.put("logs/c.txt", "id,name\n3,cy\n");
    f.files.put("other/d.csv", "id,name\n4,dee\n");
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_csv("logs/*.csv", Small)"#)
    );
}

#[test]
fn a_glob_numbers_records_across_its_files_in_name_order() {
    let mut f = fixture();
    f.files.put("logs/a.csv", "id,name\n1,ann\n2,bo\n");
    f.files.put("logs/b.csv", "id,name\nx,cy\n");
    let (source, line, _) = bad_input(f.eval(r#"read_csv("logs/*.csv", Small)"#));
    assert_eq!(("logs/b.csv", 3), (source.as_str(), line));
}

#[test]
fn a_glob_with_many_stars_matches_in_polynomial_time() {
    let mut f = fixture();
    let long = format!("{}.csv", "a".repeat(40));
    f.files.put(&long, "id,name\n1,ann\n");
    assert_eq!(
        set([]),
        f.value(r#"read_csv("*a*a*a*a*a*a*a*a*a*a*a*a*b", Small)"#)
    );
    assert_eq!(
        texts(["ann"]),
        names(&f, r#"read_csv("*a*a*a*a*a*a*a*a*a*.c?v", Small)"#)
    );
}

#[test]
fn a_glob_star_matches_an_empty_run_and_question_one_byte() {
    let mut f = fixture();
    f.files.put("ab.csv", "id,name\n1,ann\n");
    f.files.put("a.csv", "id,name\n2,bo\n");
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_csv("a*.csv", Small)"#)
    );
    assert_eq!(texts(["ann"]), names(&f, r#"read_csv("a?.csv", Small)"#));
    assert_eq!(set([]), f.value(r#"read_csv("a?*b.csv", Small)"#));
}

#[test]
fn a_glob_matching_no_file_reads_no_rows() {
    let f = fixture();
    assert_eq!(set([]), f.value(r#"read_csv("none/*.csv", Small)"#));
}

#[test]
fn a_file_that_cannot_be_read_is_bad_input_naming_it() {
    let mut f = fixture();
    let (source, _, _) = bad_input(f.eval(r#"read_csv("missing.csv", Small)"#));
    assert_eq!("missing.csv", source);
    f.files.put("bin.csv", vec![b'i', b'd', 0xff, b'\n']);
    let (source, _, detail) = bad_input(f.eval(r#"read_csv("bin.csv", Small)"#));
    assert_eq!("bin.csv", source);
    assert!(detail.contains("UTF-8"), "{detail}");
}

#[test]
fn a_file_scan_is_a_set_so_equal_records_are_one_row() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann\n1,ann\n");
    assert_eq!(
        int(1),
        f.value(r#"count(r for r in read_csv("s.csv", Small))"#)
    );
}

#[test]
fn a_scan_joins_relations() {
    let mut f = fixture();
    f.files.put("o.csv", "user,n\n1,10\n1,5\n2,7\n");
    assert_eq!(
        f.value(r#"[{ name: "ann", n: 15 }, { name: "bo", n: 7 }]"#),
        f.value(
            r#"from r in read_csv("o.csv", Order), u in users
               where r.user == key u
               group r by u.name into g
               select { name: g.key, n: sum(x.n for x in g) }"#
        )
    );
}

// Loads

#[test]
fn a_load_inserts_the_rows_of_a_file() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann\n2,bo\n");
    f.ok(r#"insert outs from (from r in read_csv("s.csv", Small) select r)"#);
    assert_eq!(texts(["ann", "bo"]), names(&f, "outs"));
}

#[test]
fn a_load_with_rejects_routes_failed_records_to_the_rejects_relation() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann\nx,bo\n3,\n");
    f.ok(r#"insert outs from (
              from r in read_csv("s.csv", Small)
              select Small { id: r.id, name: if r.name == "" then reject "no name" else r.name })
            rejects into out_rejects"#);
    assert_eq!(texts(["ann"]), names(&f, "outs"));
    assert_eq!(ints([2, 3]), f.value("from j in out_rejects select j.line"));
    assert_eq!(
        texts(["s.csv"]),
        f.value("from j in out_rejects select j.source")
    );
    assert_eq!(
        texts(["x,bo", "3,"]),
        f.value("from j in out_rejects select j.raw")
    );
    let reasons = f.value("from j in out_rejects where j.line == 3 select j.reason");
    assert_eq!(texts(["no name"]), reasons);
    let conversion = f.one("from j in out_rejects where j.line == 2 select j.reason");
    let Value::Text(why) = conversion else {
        panic!("a reason")
    };
    assert!(why.contains("`id`"), "{why}");
}

#[test]
fn a_load_without_rejects_fails_with_bad_input_on_a_failed_record() {
    let mut f = fixture();
    f.files.put("s.csv", "id,name\n1,ann\nx,bo\n");
    assert_eq!(
        "s.csv",
        match f.rejected(r#"insert outs from (from r in read_csv("s.csv", Small) select r)"#) {
            Rejection::BadInput {
                source, line: 2, ..
            } => source,
            other => panic!("{other:?}"),
        }
    );
}

#[test]
fn a_load_routes_a_tag_column_onto_constructors() {
    let mut f = fixture();
    f.files
        .put("i.csv", "user,kind,ident\n1,Google,g-1\n2,Apple,a-2\n");
    f.ok(r#"insert identities from (
              from r in read_csv("i.csv", Kinded)
              select match r.kind
                | Google -> Google { user: r.user, gid: r.ident }
                | Apple  -> Apple { user: r.user, sub: r.ident })"#);
    assert_eq!(
        texts(["g-1"]),
        f.value("from g in identities.Google select g.gid")
    );
    assert_eq!(
        texts(["a-2"]),
        f.value("from a in identities.Apple select a.sub")
    );
}

// NDJSON

#[test]
fn an_ndjson_scan_reads_objects_with_members_in_any_order() {
    let mut f = fixture();
    f.files.put(
        "s.ndjson",
        "{\"name\": \"ann\", \"id\": \"1\", \"extra\": [1, 2]}\n\n{\"id\": \"2\", \"name\": \"bo\"}\n",
    );
    assert_eq!(
        texts(["ann", "bo"]),
        names(&f, r#"read_ndjson("s.ndjson", Small)"#)
    );
}

#[test]
fn an_ndjson_scan_reads_the_json_mapping_of_every_value() {
    let mut f = fixture();
    f.files.put(
        "w.ndjson",
        r#"{"id": "1", "name": "ann", "nick": {"Some": {"value": "a"}}, "owner": "1", "level": "High", "pos": {"x": 3, "y": 4}, "via": {"Email": {"addr": "a@b.example"}}, "amount": "12.50", "ratio": 0.5}"#,
    );
    let w = f.one(r#"from r in read_ndjson("w.ndjson", Wide) select r"#);
    assert_eq!(Some(&Value::some(text("a"))), w.field("nick"));
    assert_eq!(Some(&key(1)), w.field("owner"));
    assert_eq!(f.value("High"), w.field("level").cloned().expect("a level"));
    assert_eq!(
        f.value("Point { x: 3, y: 4 }"),
        w.field("pos").cloned().expect("a point")
    );
    assert_eq!(
        f.value(r#"Email { addr: "a@b.example" }"#),
        w.field("via").cloned().expect("a via")
    );
    assert_eq!(
        f.value("12.50"),
        w.field("amount").cloned().expect("an amount")
    );
    assert_eq!(Some(&Value::F64(0.5)), w.field("ratio"));
}

#[test]
fn an_option_is_none_by_name_or_by_a_missing_member() {
    let mut f = fixture();
    f.files.put(
        "n.ndjson",
        "{\"id\": \"1\", \"nick\": \"None\"}\n{\"id\": \"2\"}\n{\"id\": \"3\", \"nick\": {\"Some\": {\"value\": \"\"}}}\n",
    );
    assert_eq!(
        set([Value::none(), Value::some(text(""))]),
        f.value(r#"from r in read_ndjson("n.ndjson", Nick) select r.nick"#)
    );
}

#[test]
fn an_ndjson_sum_row_is_an_object_of_one_member_named_by_its_constructor() {
    let mut f = fixture();
    f.files.put(
        "i.ndjson",
        r#"{"Google": {"id": "7", "user": "1", "gid": "g-1"}}"#,
    );
    assert_eq!(
        f.value(r#"[Google { id: identities.key(7), user: users.key(1), gid: "g-1" }]"#),
        f.value(r#"read_ndjson("i.ndjson", Identity)"#)
    );
}

#[test]
fn a_float_reads_nan_and_the_infinities_from_strings() {
    let mut f = fixture();
    f.files.put("f.ndjson", "{\"x\": \"NaN\"}\n{\"x\": 1.5}\n");
    assert_eq!(
        int(2),
        f.value(r#"count(r for r in read_ndjson("f.ndjson", Measure))"#)
    );
}

#[test]
fn a_number_for_a_scalar_written_as_a_string_does_not_convert() {
    let mut f = fixture();
    f.files.put("s.ndjson", "{\"id\": 1, \"name\": \"ann\"}\n");
    let (_, line, detail) = bad_input(f.eval(r#"read_ndjson("s.ndjson", Small)"#));
    assert_eq!(1, line);
    assert!(detail.contains("`id`"), "{detail}");
}

#[test]
fn a_line_that_is_not_json_does_not_convert() {
    let mut f = fixture();
    f.files.put(
        "s.ndjson",
        "{\"id\": \"1\", \"name\": \"ann\"}\n{\"id\": \n",
    );
    let (source, line, _) = bad_input(f.eval(r#"read_ndjson("s.ndjson", Small)"#));
    assert_eq!(("s.ndjson", 2), (source.as_str(), line));
}

#[test]
fn json_strings_take_their_escapes() {
    let mut f = fixture();
    f.files
        .put("e.ndjson", r#"{"id": "1", "name": "a\"b\\c\né😀"}"#);
    assert_eq!(
        texts(["a\"b\\c\n\u{e9}\u{1f600}"]),
        names(&f, r#"read_ndjson("e.ndjson", Small)"#)
    );
}

// Temp relations

#[test]
fn a_temp_relation_holds_rows_for_the_rest_of_its_text() {
    let mut f = fixture();
    let out = f
        .text(
            r#"temp relation stage : Small key id
               insert stage { id: 1, name: "ann" }
               insert stage { id: 2, name: "bo" }
               from s in stage select s.name"#,
        )
        .expect("the text runs");
    assert_eq!(Some(&texts(["ann", "bo"])), out.last());
}

#[test]
fn a_temp_relation_is_gone_when_its_text_ends() {
    let mut f = fixture();
    let before = f.db.clone();
    f.text(
        r#"temp relation stage : Small key id
           insert stage { id: 1, name: "ann" }"#,
    )
    .expect("the text runs");
    assert_eq!(before, f.db);
    let out = f
        .text(
            r"temp relation stage : Small key id
               from s in stage select s.name",
        )
        .expect("a later text declares it again");
    assert_eq!(Some(&set([])), out.last());
}

#[test]
fn a_temp_relation_without_a_key_clause_is_keyed_by_its_whole_row() {
    let mut f = fixture();
    let out = f
        .text(
            r"temp relation pairs : Pair
               insert pairs { a: 1, b: 1 }
               insert pairs { a: 1, b: 2 }
               (count(p for p in pairs))",
        )
        .expect("rows differing in any field are distinct");
    assert_eq!(Some(&int(2)), out.last());
}

#[test]
fn a_temp_relation_key_is_checked() {
    let mut f = fixture();
    match f.text(
        r#"temp relation stage : Small key id
           insert stage { id: 1, name: "ann" }
           insert stage { id: 1, name: "bo" }"#,
    ) {
        Err((3, Abort::Reject(Rejection::KeyViolation { relation, .. }))) => {
            assert_eq!("stage", relation);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failing_item_ends_its_text_and_keeps_nothing() {
    let mut f = fixture();
    let before = f.db.clone();
    match f.text(
        r#"insert users { name: "cy" }
           insert users { id: users.key(1), name: "dup" }
           insert users { name: "dee" }"#,
    ) {
        Err((2, Abort::Reject(Rejection::KeyViolation { .. }))) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(before, f.db);
}

#[test]
fn a_staged_load_lands_in_a_temp_relation_then_moves_with_rejects() {
    let mut f = fixture();
    f.files
        .put("s.csv", "line,id,name\n1,1,ann\n2,2,\n3,3,cy\n");
    f.text(
        r#"temp relation stage : Staged key line
           temp relation stage_rejects : RejectedRow key (load, line)
           insert stage from (from r in read_csv("s.csv", Staged) select r)
           delete s in stage where s.line == 3
           insert outs from (
             from s in stage
             select Small { id: s.id, name: if s.name == "" then reject "no name" else s.name })
           rejects into stage_rejects
           insert out_rejects from (from j in stage_rejects select j)"#,
    )
    .expect("the staged load runs");
    assert_eq!(texts(["ann"]), names(&f, "outs"));
    assert_eq!(
        texts(["stage"]),
        f.value("from j in out_rejects select j.source")
    );
}
