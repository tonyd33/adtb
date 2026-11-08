use super::{
    AllocateFate, Draw, Faults, ForceFate, Landing, ReadFate, WriteFate, decide_allocate,
    decide_force, decide_read, decide_unforced, decide_write,
};

const CASES: u64 = if cfg!(miri) { 16 } else { 512 };

#[derive(Clone, Copy, Debug, PartialEq)]
enum Asked {
    Chance(f64),
    Below(u64),
    Coin,
}

/// A draw source answering from a fixed script and recording what it was asked.
struct Script {
    answers: Vec<u64>,
    asked: Vec<Asked>,
}

impl Script {
    fn new(answers: &[u64]) -> Script {
        Script {
            answers: answers.to_vec(),
            asked: Vec::new(),
        }
    }

    fn next(&mut self) -> u64 {
        let at = self.asked.len() - 1;
        *self
            .answers
            .get(at)
            .unwrap_or_else(|| panic!("the script holds {} answers", self.answers.len()))
    }
}

impl Draw for Script {
    fn chance(&mut self, p: f64) -> bool {
        self.asked.push(Asked::Chance(p));
        self.next() & 1 == 1
    }

    fn below(&mut self, n: u64) -> u64 {
        self.asked.push(Asked::Below(n));
        self.next() % n
    }

    fn coin(&mut self) -> bool {
        self.asked.push(Asked::Coin);
        self.next() & 1 == 1
    }
}

/// The draws a decision is expected to make, answered from the same script.
struct Expect<'a> {
    answers: &'a [u64],
    asked: Vec<Asked>,
}

impl Expect<'_> {
    fn answer(&mut self, asked: Asked) -> u64 {
        let raw = self.answers[self.asked.len()];
        self.asked.push(asked);
        raw
    }

    /// A fault of chance zero draws nothing.
    fn strikes(&mut self, p: f64) -> bool {
        p > 0.0 && self.answer(Asked::Chance(p)) & 1 == 1
    }

    fn below(&mut self, n: u64) -> u64 {
        self.answer(Asked::Below(n)) % n
    }

    fn coin(&mut self) -> bool {
        self.answer(Asked::Coin) & 1 == 1
    }
}

struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn p(&mut self) -> f64 {
        [0.0, 0.5, 1.0][self.below(3) as usize]
    }

    fn faults(&mut self) -> Faults {
        let sector = [512, 4096][self.below(2) as usize];
        Faults {
            seed: self.next(),
            torn_p: self.p(),
            corrupt_p: self.p(),
            misdirect_p: self.p(),
            lost_write_p: self.p(),
            latent_p: self.p(),
            fail_p: self.p(),
            lose_unsynced: self.below(2) == 1,
            no_space: self.below(2) == 1,
            sector,
            atomic_unit: sector * (1 + self.below(2)),
        }
    }

    fn answers(&mut self) -> Vec<u64> {
        (0..64).map(|_| self.next()).collect()
    }
}

/// A write: with `no_space`, past the end is `Full` with no draw; then fail, and a coin per
/// sector; then lost; then misdirect, and a uniform pick among the other sector-aligned offsets
/// where it fits within the file, or just past its end when there is none.
fn expected_write(
    e: &mut Expect<'_>,
    f: &Faults,
    offset: u64,
    len: u64,
    file_len: u64,
) -> WriteFate {
    if f.no_space && offset + len > file_len {
        return WriteFate::Full;
    }
    if e.strikes(f.fail_p) {
        let sectors = len.div_ceil(f.sector);
        let landed = (0..sectors)
            .filter(|_| e.coin())
            .map(|i| offset + i * f.sector..(offset + (i + 1) * f.sector).min(offset + len))
            .collect();
        return WriteFate::Fail { landed };
    }
    if e.strikes(f.lost_write_p) {
        return WriteFate::Lost;
    }
    if !e.strikes(f.misdirect_p) {
        return WriteFate::At(offset);
    }
    let end = file_len.max(offset + len);
    let others: Vec<u64> = (0..)
        .map(|i| i * f.sector)
        .take_while(|at| at + len <= end)
        .filter(|&at| at != offset)
        .collect();
    if others.is_empty() {
        WriteFate::At(offset + len)
    } else {
        WriteFate::At(others[e.below(others.len() as u64) as usize])
    }
}

/// A read finding bytes: latent, and a uniform pick of their sectors; then, unless a sector is
/// unreadable, corrupt, a uniform byte and a uniform bit.
fn expected_read(e: &mut Expect<'_>, f: &Faults, offset: u64, n: u64, bad: bool) -> ReadFate {
    if n == 0 {
        return ReadFate::Clean;
    }
    if e.strikes(f.latent_p) {
        let sectors: Vec<u64> = (offset / f.sector..=(offset + n - 1) / f.sector).collect();
        return ReadFate::Latent {
            sector: sectors[e.below(sectors.len() as u64) as usize],
        };
    }
    if bad {
        return ReadFate::Unreadable;
    }
    if !e.strikes(f.corrupt_p) {
        return ReadFate::Clean;
    }
    let byte = offset + e.below(n);
    let bit = 1 << e.below(8);
    ReadFate::Corrupt { byte, bit }
}

fn write_kind(fate: &WriteFate) -> &'static str {
    match fate {
        WriteFate::Full => "Full",
        WriteFate::Fail { .. } => "Fail",
        WriteFate::Lost => "Lost",
        WriteFate::At(_) => "At",
    }
}

fn read_kind(fate: ReadFate) -> &'static str {
    match fate {
        ReadFate::Latent { .. } => "Latent",
        ReadFate::Unreadable => "Unreadable",
        ReadFate::Corrupt { .. } => "Corrupt",
        ReadFate::Clean => "Clean",
    }
}

fn allocate_kind(fate: AllocateFate) -> &'static str {
    match fate {
        AllocateFate::Fail => "Fail",
        AllocateFate::Full => "Full",
        AllocateFate::Run => "Run",
    }
}

fn landing_kind(landing: &Landing) -> &'static str {
    match landing {
        Landing::Whole => "Whole",
        Landing::Lost => "Lost",
        Landing::Torn(_) => "Torn",
    }
}

#[test]
fn every_write_meets_its_fate_from_draws_in_the_stated_order() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..CASES {
        let mut g = Gen(seed);
        let faults = g.faults();
        let offset = g.below(8) * faults.sector;
        let len = (1 + g.below(3)) * faults.sector;
        let file_len = g.below(10) * faults.sector;
        let answers = g.answers();
        let mut e = Expect {
            answers: &answers,
            asked: Vec::new(),
        };
        let expected = expected_write(&mut e, &faults, offset, len, file_len);
        let mut script = Script::new(&answers);
        let fate = decide_write(&mut script, &faults, offset, len, file_len);
        assert_eq!(
            expected, fate,
            "seed {seed}: {faults:?} @{offset}+{len} of {file_len}"
        );
        assert_eq!(e.asked, script.asked, "seed {seed}: the draws");
        seen.insert(write_kind(&fate));
    }
    assert_eq!(
        ["At", "Fail", "Full", "Lost"].to_vec(),
        seen.into_iter().collect::<Vec<_>>(),
        "every fate is reached"
    );
}

#[test]
fn every_read_meets_its_fate_from_draws_in_the_stated_order() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..CASES {
        let mut g = Gen(seed);
        let faults = g.faults();
        let offset = g.below(8) * faults.sector;
        let n = g.below(3 * faults.sector);
        let bad = g.below(4) == 0;
        let answers = g.answers();
        let mut e = Expect {
            answers: &answers,
            asked: Vec::new(),
        };
        let expected = expected_read(&mut e, &faults, offset, n, bad);
        let mut script = Script::new(&answers);
        let fate = decide_read(&mut script, &faults, offset, n, bad);
        assert_eq!(
            expected, fate,
            "seed {seed}: {faults:?} @{offset} finds {n}"
        );
        assert_eq!(e.asked, script.asked, "seed {seed}: the draws");
        seen.insert(read_kind(fate));
    }
    assert_eq!(
        ["Clean", "Corrupt", "Latent", "Unreadable"].to_vec(),
        seen.into_iter().collect::<Vec<_>>(),
        "every fate is reached"
    );
}

/// An allocate: fail; then under `no_space` one that grows the file is `Full`. A force: fail.
#[test]
fn every_allocate_and_force_meets_its_fate_from_draws_in_the_stated_order() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..CASES {
        let mut g = Gen(seed);
        let faults = g.faults();
        let len = g.below(4) * faults.sector;
        let file_len = g.below(4) * faults.sector;
        let answers = g.answers();
        let mut e = Expect {
            answers: &answers,
            asked: Vec::new(),
        };
        let expected = if e.strikes(faults.fail_p) {
            AllocateFate::Fail
        } else if faults.no_space && len > file_len {
            AllocateFate::Full
        } else {
            AllocateFate::Run
        };
        let mut script = Script::new(&answers);
        let fate = decide_allocate(&mut script, &faults, len, file_len);
        assert_eq!(
            expected, fate,
            "seed {seed}: {faults:?} to {len} of {file_len}"
        );
        assert_eq!(e.asked, script.asked, "seed {seed}: the allocate's draws");
        seen.insert(allocate_kind(fate));

        let mut e = Expect {
            answers: &answers,
            asked: Vec::new(),
        };
        let expected = if e.strikes(faults.fail_p) {
            ForceFate::Fail
        } else {
            ForceFate::Run
        };
        let mut script = Script::new(&answers);
        assert_eq!(
            expected,
            decide_force(&mut script, &faults),
            "seed {seed}: {faults:?}"
        );
        assert_eq!(e.asked, script.asked, "seed {seed}: the force's draws");
    }
    assert_eq!(
        ["Fail", "Full", "Run"].to_vec(),
        seen.into_iter().collect::<Vec<_>>(),
        "every fate is reached"
    );
}

/// A crash: a completed write without `lose_unsynced` lands whole with no draw; otherwise one
/// larger than the atomic unit draws torn, and a coin per atomic unit when it strikes; else one
/// coin for whether it lands whole.
#[test]
fn every_unforced_write_meets_its_crash_from_draws_in_the_stated_order() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..CASES {
        let mut g = Gen(seed);
        let faults = g.faults();
        let len = (1 + g.below(4)) * faults.sector;
        let completed = g.below(2) == 1;
        let answers = g.answers();
        let mut e = Expect {
            answers: &answers,
            asked: Vec::new(),
        };
        let expected = if completed && !faults.lose_unsynced {
            Landing::Whole
        } else if len > faults.atomic_unit && e.strikes(faults.torn_p) {
            let units = len.div_ceil(faults.atomic_unit);
            Landing::Torn((0..units).map(|_| e.coin()).collect())
        } else if e.coin() {
            Landing::Whole
        } else {
            Landing::Lost
        };
        let mut script = Script::new(&answers);
        let landing = decide_unforced(&mut script, &faults, len, completed);
        assert_eq!(
            expected, landing,
            "seed {seed}: {faults:?} {len} bytes, completed {completed}"
        );
        assert_eq!(e.asked, script.asked, "seed {seed}: the draws");
        seen.insert(landing_kind(&landing));
    }
    assert_eq!(
        ["Lost", "Torn", "Whole"].to_vec(),
        seen.into_iter().collect::<Vec<_>>(),
        "every landing is reached"
    );
}
