use super::*;
use std::error::Error;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Instant;

mod process;
mod worker;

type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[expect(
    clippy::disallowed_methods,
    reason = "remove when host process deadlines have a clock independent of the engine runtime"
)]
fn now() -> Instant {
    Instant::now()
}

const ENTRY: &str = "native::driver::crash_tests::child_entry";
const ROLE: &str = "ADTB_CRASH_ROLE";
const HELLO: u64 = 0;
const ACK: u64 = 1;
const VALIDATED: u64 = 5;
const PERMIT: [u64; 5] = [6, 0, 0, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    Acknowledged = 2,
    BeforeCall = 3,
    Withheld = 4,
}

#[derive(Clone, Debug)]
struct Case {
    name: String,
    cut: Cut,
    prefix: usize,
    expected: [i64; 4],
}

fn corpus() -> Result<(Vec<Id>, Vec<Case>)> {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/crash.txt"))?;
    let mut lines = text.lines();
    let keys: Vec<Id> = lines
        .next()
        .ok_or("missing keys")?
        .split_whitespace()
        .map(|v| v.parse::<i64>().map(Id))
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(
        keys,
        [Id(1), Id(2), Id(3), Id(4), Id(1), Id(2), Id(3), Id(4)]
    );
    let mut cases = Vec::new();
    for line in lines {
        let fields: Vec<_> = line.split_whitespace().collect();
        let [name, mode, prefix, a, b, c, d] = fields.as_slice() else {
            return Err("crash case has seven fields".into());
        };
        let cut = match *mode {
            "ack" => Cut::Acknowledged,
            "before" => Cut::BeforeCall,
            "withheld" => Cut::Withheld,
            _ => return Err("unknown cut".into()),
        };
        cases.push(Case {
            name: (*name).into(),
            cut,
            prefix: prefix.parse()?,
            expected: [a.parse()?, b.parse()?, c.parse()?, d.parse()?],
        });
    }
    assert_eq!(cases.len(), 4, "all four cut contracts run");
    assert_eq!(
        cases.iter().map(|c| (c.cut, c.prefix)).collect::<Vec<_>>(),
        [
            (Cut::Acknowledged, 3),
            (Cut::Acknowledged, 8),
            (Cut::BeforeCall, 3),
            (Cut::Withheld, 3)
        ]
    );
    Ok((keys, cases))
}

fn frame(stream: &mut UnixStream, deadline: Instant) -> Result<[u64; 5]> {
    let mut bytes = [0; 40];
    let mut at = 0;
    while at < bytes.len() {
        stream.set_read_timeout(Some(
            deadline
                .checked_duration_since(now())
                .ok_or("read deadline")?,
        ))?;
        let read = stream.read(&mut bytes[at..])?;
        if read == 0 {
            return Err("incomplete frame".into());
        }
        at += read;
    }
    Ok(decode(&bytes))
}

fn decode(bytes: &[u8; 40]) -> [u64; 5] {
    std::array::from_fn(|i| {
        u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().expect("eight bytes"))
    })
}

fn send(stream: &mut UnixStream, words: [u64; 5], deadline: Instant) -> Result<()> {
    let bytes = encode(words);
    let mut at = 0;
    while at < bytes.len() {
        stream.set_write_timeout(Some(
            deadline
                .checked_duration_since(now())
                .ok_or("write deadline")?,
        ))?;
        let written = stream.write(&bytes[at..])?;
        if written == 0 {
            return Err("incomplete write".into());
        }
        at += written;
    }
    Ok(())
}

fn encode(words: [u64; 5]) -> [u8; 40] {
    let mut bytes = [0; 40];
    for (chunk, word) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn ack(index: usize, keys: &[Id]) -> [u64; 5] {
    [
        ACK,
        index as u64,
        keys[index].0.cast_unsigned(),
        (index / 4 + 1) as u64,
        0,
    ]
}

fn gate(case: &Case, keys: &[Id]) -> [u64; 5] {
    [
        case.cut as u64,
        case.prefix as u64,
        keys.get(case.prefix).map_or(0, |id| id.0.cast_unsigned()),
        0,
        0,
    ]
}

fn hello(stream: &mut UnixStream, deadline: Instant) -> Result<()> {
    assert_eq!(
        frame(stream, deadline)?,
        [HELLO, adtb_heap_cap::current_cap() as u64, 0, 0, 0],
        "one child receives the remaining one-slot cap"
    );
    Ok(())
}

fn run_case(peer: Peer, index: usize, case: &Case, keys: &[Id], root: &Path) -> Result<()> {
    std::fs::create_dir(root)?;
    let socket = root.join("ipc");
    let listener = UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let deadline = now() + Duration::from_secs(10);
    let mut writer = process::Child::start(peer, index, "writer", root, deadline)?;
    let mut stream = writer.connect(&listener)?;
    hello(&mut stream, deadline)?;
    let mut journal = File::create_new(root.join("acknowledged"))?;
    for i in 0..case.prefix {
        let observed = frame(&mut stream, deadline)?;
        assert_eq!(
            observed,
            ack(i, keys),
            "ACK {i} has external key/value identity"
        );
        journal.write_all(&encode(observed))?;
        journal.sync_all()?;
        send(&mut stream, PERMIT, deadline)?;
    }
    let reached = frame(&mut stream, deadline)?;
    assert_eq!(
        reached,
        gate(case, keys),
        "the declared kill cut was reached"
    );
    if case.cut != Cut::Acknowledged {
        journal.write_all(&encode(reached))?;
        journal.sync_all()?;
    }
    drop(journal);
    writer.terminate()?;
    drop(stream);
    let mut verifier = process::Child::start(peer, index, "verifier", root, deadline)?;
    let mut stream = verifier.connect(&listener)?;
    hello(&mut stream, deadline)?;
    assert_eq!(
        frame(&mut stream, deadline)?,
        [VALIDATED, case.prefix as u64, 4, 0, 0],
        "full reopened state and retained history both validated"
    );
    verifier.finish()?;
    Ok(())
}

fn run(peer: Peer) -> Result<()> {
    assert_eq!(
        std::env::var("NEXTEST_TEST_GROUP")?,
        "slots-2",
        "the parent reserves its own slot and its child's slot"
    );
    let (keys, cases) = corpus()?;
    let scratch =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sqlite-crash-controls/cases");
    std::fs::create_dir_all(&scratch)?;
    let scratch = scratch.canonicalize()?;
    let mut reached = 0;
    for (index, case) in cases.iter().enumerate() {
        let root = scratch.join(format!("{peer:?}-{}-{}", std::process::id(), case.name));
        run_case(peer, index, case, &keys, &root)?;
        reached += 1;
    }
    assert_eq!(reached, 4, "every cut ran for {peer:?}");
    Ok(())
}

#[test]
fn adtb_acknowledged_prefix() {
    run(Peer::Adtb).expect("ADTB acknowledged prefix survives owned child termination");
}

#[test]
fn sqlite_acknowledged_prefix() {
    run(Peer::Sqlite).expect("SQLite acknowledged prefix survives owned child termination");
}

#[test]
fn child_entry() {
    if let Ok(role) = std::env::var(ROLE) {
        worker::entry(&role).expect("crash-control child completes its declared role");
    }
}
