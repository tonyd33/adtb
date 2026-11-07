//! File scans through a role session: `file_roots` confines them, the in-process API is not.

use adtb_simkit::api;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::{Wake, Waker};

use adtb::runtime::{ThreadId, Threads};
use adtb::{
    Completion, Database, Error, Log, Observation, OpenOptions, Pins, Registration, RejectReason,
    RoleBinding, RoleSession, Task, TaskOptions, Value,
};
use adtb_sim::SimRuntime;

use api::{once, options, printed, results};

const SCHEMA: &str = "module shop
add module shop
add type Named = { name: Text }
add relation loaded : Named key name
";

const GRANTS: &str = "module shop
grant insert loaded, read loaded to ann with grant option
";

const LOAD: &str = r"import shop
txn load (p: Text) -> I64 =
  transact
    insert loaded from (from r in read_csv(p, Named) select r)
    return 1
";

const OUTSIDE: &str = "is outside file_roots";

fn tree(name: &str) -> PathBuf {
    let at = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("server-files")
        .join(name);
    if at.exists() {
        std::fs::remove_dir_all(&at).expect("the old directory is removed");
    }
    for dir in ["root/sub", "out"] {
        std::fs::create_dir_all(at.join(dir)).expect("the directory is made");
    }
    for file in ["root/in.csv", "root/sub/deep.csv", "out/secret.csv"] {
        let stem = Path::new(file)
            .file_stem()
            .expect("a stem")
            .to_string_lossy();
        std::fs::write(at.join(file), format!("name\n{stem}\n")).expect("the file is written");
    }
    at
}

struct Unpark {
    rt: SimRuntime,
    thread: ThreadId,
}

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.rt.unpark(self.thread);
    }
}

fn finish(rt: &SimRuntime, task: &Task<SimRuntime>) -> Result<Vec<String>, Error> {
    let waker = Waker::from(Arc::new(Unpark {
        rt: rt.clone(),
        thread: rt.current(),
    }));
    task.supply(usize::MAX);
    let mut rows = Vec::new();
    loop {
        match task.observe(&waker) {
            Observation::Pending => rt.park(),
            Observation::Layout(_) => {}
            Observation::Batch(b) => {
                let batch = b.rows().expect("a task of values");
                rows.extend(batch.iter().map(|v| format!("{v:?}")));
            }
            Observation::End => task.drained(),
            Observation::Done(Completion::Failed(e)) => return Err(e),
            Observation::Done(_) => return Ok(rows),
        }
    }
}

struct Server {
    rt: SimRuntime,
    db: Database<SimRuntime>,
    owner: RoleSession<SimRuntime>,
}

impl Server {
    fn new(rt: SimRuntime, base: &Path, roots: Vec<PathBuf>) -> Server {
        let open = |log: &[&str]| {
            let opts = OpenOptions {
                file_root: Some(base.to_path_buf()),
                file_roots: roots.clone(),
                ..options(2)
            };
            Database::open_in(
                rt.clone(),
                "files.adt",
                &Log::new(log.iter().copied()),
                &opts,
            )
            .expect("the database opens")
        };
        let db = open(&[SCHEMA]);
        for role in ["ann", "web"] {
            db.role_authority().create_role(role).expect("the role");
        }
        db.close().expect("close");
        let db = open(&[SCHEMA, GRANTS]);
        let authority = db.role_authority();
        let owner = authority.session("owner").expect("a session");
        authority
            .grant(&owner, "grant read_files to ann")
            .expect("it grants");
        Server { rt, db, owner }
    }

    /// Returns the rows of a scan of `path` as ad hoc text of a role session.
    fn scan(&self, path: &str) -> Result<Vec<String>, Error> {
        let text = format!(r#"from r in read_csv("{path}", Named) select r.name"#);
        let task = self.owner.submit_text(&text, &[], TaskOptions::default())?;
        finish(&self.rt, &task)
    }

    fn refused(&self, path: &str) {
        assert_outside(path, self.scan(path));
    }

    /// Returns ann's bundle's binding for `web`, its one transaction loading the file its
    /// argument names.
    fn load(&self) -> RoleBinding<SimRuntime> {
        let mut source = adtb_lang::evolve::Source::default();
        for text in [SCHEMA, GRANTS] {
            let file =
                adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
                    .ok()
                    .expect("the migration parses");
            source = source.migrate(&file, text).expect("the migration applies");
        }
        let schema = source.schema().expect("the source builds");
        let hash: String = adtb::catalog::version_hash(&schema, &source)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let text = format!("bundle load-1.0 pins 2 ({hash}) edition \"2026-10\"\n{LOAD}");
        let authority = self.db.role_authority();
        let ann = authority.session("ann").expect("a session");
        let reg = Registration::bundle(text).expect("the bundle parses");
        authority.register(&ann, &reg).expect("it registers");
        authority
            .grant(&ann, "grant call load-1.0 to web")
            .expect("it grants");
        let web = authority.session("web").expect("a session");
        authority
            .bind(&web, "load-1.0", &Pins::default())
            .expect("it binds")
    }

    fn call(&self, bound: &RoleBinding<SimRuntime>, path: &str) -> Result<Vec<String>, Error> {
        let task = bound.submit(1, vec![Value::text(path)], TaskOptions::default())?;
        finish(&self.rt, &task)
    }
}

fn assert_outside(path: &str, got: Result<Vec<String>, Error>) {
    match got {
        Err(Error::Rejected {
            reason: RejectReason::Denied { detail },
        }) => assert!(detail.contains(OUTSIDE), "{path}: {detail}"),
        other => panic!("{path} is not denied: {other:?}"),
    }
}

#[test]
fn an_empty_file_roots_refuses_every_read_with_read_files_held() {
    let base = tree("empty");
    once(1, |rt| {
        let s = Server::new(rt, &base, Vec::new());
        s.refused("root/in.csv");
        s.refused("root/nosuch.csv");
        s.refused("root/*.csv");
        s.refused("nosuch/*.csv");
    });
}

#[test]
fn a_read_inside_a_root_passes() {
    let base = tree("inside");
    once(2, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        assert_eq!(1, s.scan("root/in.csv").expect("relative").len());
        assert_eq!(1, s.scan("root/sub/deep.csv").expect("deeper").len());
        assert_eq!(1, s.scan("root/sub/../in.csv").expect("stays inside").len());
        assert_eq!(1, s.scan("root/*.csv").expect("a glob").len());
        assert_eq!(
            1,
            s.scan("root/*/deep.csv").expect("a directory glob").len()
        );
        let absolute = base.join("root/in.csv");
        let absolute = absolute.to_str().expect("a text path");
        assert_eq!(1, s.scan(absolute).expect("absolute").len());
    });
}

#[test]
fn a_root_of_absolute_path_confines_the_same() {
    let base = tree("absolute");
    once(3, |rt| {
        let s = Server::new(rt, &base, vec![base.join("root")]);
        assert_eq!(1, s.scan("root/in.csv").expect("inside").len());
        s.refused("out/secret.csv");
    });
}

#[test]
fn a_root_is_a_whole_path_component() {
    let base = tree("component");
    std::fs::create_dir_all(base.join("rootless")).expect("the directory is made");
    std::fs::write(base.join("rootless/a.csv"), "name\na\n").expect("the file is written");
    once(4, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        s.refused("rootless/a.csv");
    });
}

#[test]
fn dot_dot_out_of_a_root_is_refused_whether_or_not_the_file_exists() {
    let base = tree("dotdot");
    once(5, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        s.refused("root/../out/secret.csv");
        s.refused("root/../out/nosuch.csv");
        s.refused("out/secret.csv");
        s.refused("/etc/hostname");
    });
}

#[test]
fn a_symlink_inside_a_root_pointing_out_is_refused() {
    let base = tree("symlink");
    let target = base.join("out/secret.csv");
    std::os::unix::fs::symlink(&target, base.join("root/link.csv")).expect("the link");
    std::os::unix::fs::symlink(base.join("out"), base.join("root/dir")).expect("the link");
    once(6, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        s.refused("root/link.csv");
        s.refused("root/dir/secret.csv");
        s.refused("root/dir/*.csv");
        s.refused("root/*/nosuch.csv");
    });
}

#[test]
fn a_symlink_inside_a_root_pointing_inside_passes() {
    let base = tree("symlink-in");
    std::os::unix::fs::symlink(base.join("root/in.csv"), base.join("root/alias.csv"))
        .expect("the link");
    once(7, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        assert_eq!(1, s.scan("root/alias.csv").expect("inside").len());
    });
}

#[test]
fn a_glob_reaching_out_is_refused() {
    let base = tree("glob");
    std::os::unix::fs::symlink(base.join("out/secret.csv"), base.join("root/z.csv"))
        .expect("the link");
    once(8, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        s.refused("root/*.csv");
        s.refused("root/../out/*.csv");
        s.refused("*/secret.csv");
        s.refused("root/../out/nosuch*.csv");
    });
}

#[test]
fn a_symlinked_directory_a_pattern_matches_is_no_file_of_the_read() {
    let base = tree("glob-dir");
    std::os::unix::fs::symlink(base.join("out"), base.join("root/away")).expect("the link");
    once(9, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        assert_eq!(1, s.scan("root/*").expect("the directory is skipped").len());
    });
}

#[test]
fn a_parameter_path_is_checked_when_the_call_runs() {
    let base = tree("parameter");
    once(10, |rt| {
        let s = Server::new(rt, &base, vec!["root".into()]);
        let bound = s.load();
        s.call(&bound, "root/in.csv").expect("inside");
        for path in ["out/secret.csv", "root/../out/secret.csv"] {
            assert_outside(path, s.call(&bound, path));
        }
        assert_eq!(
            vec![r#""in""#.to_owned()],
            printed(results(&s.db, "from l in loaded select l.name").remove(0)),
            "a refused call loads nothing"
        );
    });
}

#[test]
fn an_empty_file_roots_refuses_a_call_of_an_owner_holding_read_files() {
    let base = tree("empty-call");
    once(13, |rt| {
        let s = Server::new(rt, &base, Vec::new());
        let bound = s.load();
        assert_outside("root/in.csv", s.call(&bound, "root/in.csv"));
    });
}

#[test]
fn the_in_process_api_is_not_confined() {
    let base = tree("in-process");
    once(11, |rt| {
        let s = Server::new(rt, &base, Vec::new());
        let got = printed(
            results(
                &s.db,
                r#"from r in read_csv("out/secret.csv", Named) select r.name"#,
            )
            .remove(0),
        );
        assert_eq!(vec![r#""secret""#.to_owned()], got);
    });
}

#[test]
fn file_roots_shows_in_process_options() {
    let base = tree("shown");
    let shown = "from o in sys::process_options where o.name == \"file_roots\" select { value: o.value, derived: o.derived }";
    once(12, |rt| {
        let db = Database::open_in(rt.clone(), "a.adt", &Log::new([SCHEMA]), &options(2))
            .expect("the database opens");
        let got = printed(results(&db, shown).remove(0));
        assert_eq!(vec![r#"{ derived: True, value: "" }"#.to_owned()], got);
        let opts = OpenOptions {
            file_roots: vec![base.join("root"), "other".into()],
            ..options(2)
        };
        let db =
            Database::open_in(rt, "b.adt", &Log::new([SCHEMA]), &opts).expect("the database opens");
        let got = printed(results(&db, shown).remove(0));
        let want = format!(
            r#"{{ derived: False, value: "{}, other" }}"#,
            base.join("root").display()
        );
        assert_eq!(vec![want], got);
    });
}
