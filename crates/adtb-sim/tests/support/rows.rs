use std::collections::BTreeMap;

use adtb::heap::{FieldType, Heap};
use adtb::key::{Scalar, Tid, Value};

pub const EMAIL: usize = 0;
pub const APPLE: usize = 2;
pub const HEAP_IDS: [u32; 3] = [10, 11, 12];
pub const LARGE_ID: u32 = 9;

/// Each key's heap and row.
pub type Rows = BTreeMap<i64, (usize, Vec<Value>)>;
/// Each key's heap, the TID of its newest version and its row.
pub type Live = BTreeMap<i64, (usize, Tid, Vec<Value>)>;

pub fn row_type(heap: usize) -> Vec<FieldType> {
    let scalars: &[Scalar] = match heap {
        APPLE => &[Scalar::I64, Scalar::I32],
        _ => &[Scalar::I64, Scalar::Text, Scalar::Bytes],
    };
    scalars.iter().map(|s| FieldType::Scalar(*s)).collect()
}

pub fn key_of(row: &[Value]) -> i64 {
    match row[0] {
        Value::I64(k) => k,
        ref v => panic!("key field holds {v:?}"),
    }
}

/// Returns heaps `a` and `b` of `heaps`, which must differ.
pub fn pair(heaps: &mut [Heap], a: usize, b: usize) -> (&mut Heap, &mut Heap) {
    assert_ne!(a, b);
    if a < b {
        let (l, r) = heaps.split_at_mut(b);
        (&mut l[a], &mut r[0])
    } else {
        let (l, r) = heaps.split_at_mut(a);
        (&mut r[0], &mut l[b])
    }
}

#[derive(Clone, Debug)]
pub enum Op {
    Insert {
        key: i64,
        heap: usize,
        row: Vec<Value>,
    },
    Update {
        key: i64,
        row: Vec<Value>,
    },
    Delete {
        key: i64,
    },
    Retag {
        key: i64,
        to: usize,
        row: Vec<Value>,
    },
}

/// Apply `op` to the model `state`.
pub fn replay(state: &mut Rows, op: &Op) {
    match op {
        Op::Insert { key, heap, row } => {
            state.insert(*key, (*heap, row.clone()));
        }
        Op::Update { key, row } => state.get_mut(key).unwrap().1 = row.clone(),
        Op::Delete { key } => {
            state.remove(key);
        }
        Op::Retag { key, to, row } => {
            state.insert(*key, (*to, row.clone()));
        }
    }
}
