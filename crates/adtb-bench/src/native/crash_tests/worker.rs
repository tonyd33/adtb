use super::*;

pub(super) fn entry(role: &str) -> Result<()> {
    let root = PathBuf::from(std::env::var("ADTB_CRASH_ROOT")?);
    let peer = match std::env::var("ADTB_CRASH_PEER")?.as_str() {
        "adtb" => Peer::Adtb,
        "sqlite" => Peer::Sqlite,
        _ => return Err("unknown peer".into()),
    };
    let index = std::env::var("ADTB_CRASH_CASE")?.parse::<usize>()?;
    let (keys, cases) = corpus()?;
    let case = cases.get(index).ok_or("unknown crash case")?;
    let deadline = now() + Duration::from_secs(10);
    let mut stream = UnixStream::connect(root.join("ipc"))?;
    send(
        &mut stream,
        [HELLO, adtb_heap_cap::current_cap() as u64, 0, 0, 0],
        deadline,
    )?;
    match role {
        "writer" => writer(peer, &root, case, &keys, &mut stream, deadline),
        "verifier" => verify(peer, &root, case, &keys, &mut stream, deadline),
        _ => Err("unknown child role".into()),
    }
}

fn writer(
    peer: Peer,
    root: &Path,
    case: &Case,
    keys: &[Id],
    stream: &mut UnixStream,
    deadline: Instant,
) -> Result<()> {
    let mut adapter = Adapter::open(peer, root, 1, true).map_err(|e| format!("{e:?}"))?;
    let fixture = model::fixture(4, false);
    adapter.seed(&fixture).map_err(|e| format!("{e:?}"))?;
    assert_eq!(adapter.snapshot().map_err(|e| format!("{e:?}"))?, fixture);
    let mut prepared = adapter.prepare().map_err(|e| format!("{e:?}"))?;
    for (i, key) in keys.iter().take(case.prefix).enumerate() {
        let (Output::Number(value), _) = prepared
            .call(Action::Bump(*key), Duration::from_secs(5))
            .map_err(|e| format!("{e:?}"))?
        else {
            return Err("Bump must return Number".into());
        };
        send(
            stream,
            [
                ACK,
                i as u64,
                key.0.cast_unsigned(),
                value.cast_unsigned(),
                0,
            ],
            deadline,
        )?;
        assert_eq!(frame(stream, deadline)?, PERMIT);
    }
    if case.cut == Cut::Withheld {
        let (Output::Number(_), _) = prepared
            .call(Action::Bump(keys[case.prefix]), Duration::from_secs(5))
            .map_err(|e| format!("{e:?}"))?
        else {
            return Err("pending Bump must return Number".into());
        };
    }
    send(stream, gate(case, keys), deadline)?;
    assert_eq!(
        frame(stream, deadline)?,
        PERMIT,
        "writer waits at reached kill gate"
    );
    Err("parent unexpectedly permitted the kill gate".into())
}

fn verify(
    peer: Peer,
    root: &Path,
    case: &Case,
    keys: &[Id],
    stream: &mut UnixStream,
    deadline: Instant,
) -> Result<()> {
    let bytes = std::fs::read(root.join("acknowledged"))?;
    let pending = usize::from(case.cut != Cut::Acknowledged);
    assert_eq!(
        bytes.len(),
        (case.prefix + pending) * 40,
        "retained journal is complete"
    );
    let mut observations = Vec::new();
    for (i, chunk) in bytes.as_chunks::<40>().0.iter().enumerate() {
        let words = decode(chunk);
        if i < case.prefix {
            assert_eq!(words, ack(i, keys), "external retained ACK identity");
            observations.push(counter::Observation {
                key: keys[i],
                outcome: counter::Outcome::Acknowledged(i64::try_from(words[3])?),
            });
        } else {
            assert_eq!(
                words,
                gate(case, keys),
                "only identified terminal call may be uncertain"
            );
            observations.push(counter::Observation {
                key: keys[i],
                outcome: counter::Outcome::Uncertain,
            });
        }
    }
    let adapter = Adapter::open(peer, root, 1, false).map_err(|e| format!("{e:?}"))?;
    let actual = adapter.snapshot().map_err(|e| format!("{e:?}"))?;
    let mut expected = model::fixture(4, false);
    for (row, value) in expected.counters.iter_mut().zip(case.expected) {
        row.value = value;
    }
    assert_eq!(
        actual, expected,
        "complete reopened state matches external case including support tables"
    );
    let values = actual
        .counters
        .iter()
        .map(|row| (Id(row.id), row.value))
        .collect();
    assert_eq!(
        counter::check(&[observations], &values),
        Ok(()),
        "retained acknowledged history is legal"
    );
    adapter.close().map_err(|e| format!("{e:?}"))?;
    send(stream, [VALIDATED, case.prefix as u64, 4, 0, 0], deadline)
}
