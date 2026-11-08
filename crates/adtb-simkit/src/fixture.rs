//! Build a component world, a pager and the structures a test makes in it, and find it again
//! after a crash.

use std::collections::BTreeSet;

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Heap, LargeStore};
use adtb::key::{CtorId, KeySchema};
use adtb::mvcc::Transactions;
use adtb::storage::{OpenError, Options, PageError, Pager, Pid};
use adtb::verify::{Inventory, Sum};
use adtb_sim::SimRuntime;

use crate::open;
use crate::recovery::verify_after_crash;

#[derive(Clone, Debug)]
struct HeapAt {
    owner: u32,
    root: Pid,
    row: Vec<FieldType>,
    large: LargeStore,
}

#[derive(Clone, Debug)]
struct IndexAt {
    owner: u32,
    root: Pid,
    schema: KeySchema,
    ctors: BTreeSet<CtorId>,
}

/// Where each structure of a world lies, kept outside the simulated process so it survives a
/// crash.
#[derive(Clone, Debug)]
pub struct Layout {
    options: Options,
    commit_log: Option<Pid>,
    larges: Vec<LargeStore>,
    heaps: Vec<HeapAt>,
    indexes: Vec<IndexAt>,
    trees: Vec<BTree>,
    next_owner: u32,
}

/// Why a world did not reopen.
#[derive(Debug)]
pub enum Refused {
    Open(OpenError),
    Page(PageError),
}

/// A pager and the structures a test made in it, each made with the next owner id of one counter.
///
/// Heap `i` holds constructor `CtorId(i)` of the one sum every index of the world ranges over.
pub struct Storage {
    pager: Pager<SimRuntime>,
    txns: Option<Transactions<SimRuntime>>,
    larges: Vec<LargeStore>,
    heaps: Vec<Heap>,
    indexes: Vec<Index>,
    trees: Vec<BTree>,
    layout: Layout,
}

impl std::fmt::Debug for Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Storage")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl Storage {
    /// Create the file `options` name, with no structure in it.
    pub fn create(rt: &SimRuntime, options: &Options) -> Storage {
        let pager = Pager::create(rt.clone(), options).expect("the file is created");
        Storage {
            pager,
            txns: None,
            larges: Vec::new(),
            heaps: Vec::new(),
            indexes: Vec::new(),
            trees: Vec::new(),
            layout: Layout {
                options: options.clone(),
                commit_log: None,
                larges: Vec::new(),
                heaps: Vec::new(),
                indexes: Vec::new(),
                trees: Vec::new(),
                next_owner: 1,
            },
        }
    }

    /// Reopen the world `layout` describes, recovering its file from the log.
    pub fn reopen(rt: &SimRuntime, layout: &Layout) -> Result<Storage, Refused> {
        let pager = open::reopen(rt, &layout.options).map_err(Refused::Open)?;
        let txns = layout
            .commit_log
            .map(|root| Transactions::open(rt.clone(), &pager, root))
            .transpose()
            .map_err(Refused::Page)?;
        let heaps = layout
            .heaps
            .iter()
            .map(|h| {
                Heap::open(&pager, h.owner, h.root, h.row.clone(), h.large)
                    .unwrap_or_else(|e| panic!("heap {} at {:?} opens: {e:?}", h.owner, h.root))
            })
            .collect();
        let indexes = layout
            .indexes
            .iter()
            .map(|i| {
                Index::open(
                    BTree::open(i.root, i.owner),
                    i.schema.clone(),
                    i.ctors.clone(),
                )
            })
            .collect();
        Ok(Storage {
            pager,
            txns,
            larges: layout.larges.clone(),
            heaps,
            indexes,
            trees: layout.trees.clone(),
            layout: layout.clone(),
        })
    }

    /// Returns where every structure lies, for [`Storage::reopen`] after a crash.
    pub fn layout(&self) -> Layout {
        self.layout.clone()
    }

    fn owner(&mut self) -> u32 {
        let owner = self.layout.next_owner;
        self.layout.next_owner += 1;
        owner
    }

    /// Create the commit log.
    pub fn commit_log(&mut self, rt: &SimRuntime) -> &Transactions<SimRuntime> {
        assert!(self.txns.is_none(), "a world has one commit log");
        let txns = Transactions::create(rt.clone(), &self.pager).expect("the commit log");
        self.layout.commit_log = Some(txns.commit_log().root());
        self.txns.insert(txns)
    }

    /// Create a store of large values.
    pub fn large(&mut self) -> LargeStore {
        let owner = self.owner();
        let large = LargeStore::create(&self.pager, owner).expect("a large store");
        self.larges.push(large);
        self.layout.larges.push(large);
        large
    }

    /// Create a heap of rows `row`, its large values in `large`, its directory nodes `fanout` wide.
    pub fn heap(&mut self, row: Vec<FieldType>, large: LargeStore, fanout: u16) -> &Heap {
        let owner = self.owner();
        let heap = Heap::create_with_fanout(&self.pager, owner, row.clone(), large, fanout)
            .expect("a heap");
        self.layout.heaps.push(HeapAt {
            owner,
            root: heap.root(),
            row,
            large,
        });
        self.heaps.push(heap);
        self.heaps.last().expect("the heap just made")
    }

    /// Create an index of keys `schema` over the heaps of `ctors`.
    pub fn index(&mut self, schema: KeySchema, ctors: BTreeSet<CtorId>) -> &Index {
        let owner = self.owner();
        let index =
            Index::create(&self.pager, owner, schema.clone(), ctors.clone()).expect("an index");
        self.layout.indexes.push(IndexAt {
            owner,
            root: index.tree().root(),
            schema,
            ctors,
        });
        self.indexes.push(index);
        self.indexes.last().expect("the index just made")
    }

    /// Create a B-link tree outside any index.
    pub fn tree(&mut self) -> BTree {
        let owner = self.owner();
        let tree = BTree::create(&self.pager, owner).expect("a tree");
        self.trees.push(tree);
        self.layout.trees.push(tree);
        tree
    }

    pub fn pager(&self) -> &Pager<SimRuntime> {
        &self.pager
    }

    /// Returns the pager, the commit log and the heaps and indexes of the world, in creation order.
    pub fn split(
        self,
    ) -> (
        Pager<SimRuntime>,
        Option<Transactions<SimRuntime>>,
        Vec<Heap>,
        Vec<Index>,
    ) {
        (self.pager, self.txns, self.heaps, self.indexes)
    }

    /// Returns the structures of the world, borrowed.
    pub fn parts(&self) -> Parts<'_> {
        Parts::new(
            &self.pager,
            self.txns.as_ref(),
            &self.heaps,
            &self.indexes,
            &self.trees,
        )
    }

    /// Returns what [`Parts::verify`] finds wrong with the world.
    pub fn verify(&self) -> Vec<String> {
        self.parts().verify()
    }
}

/// The structures of a world, for a test that keeps them in its own fields.
#[derive(Clone, Copy)]
pub struct Parts<'a> {
    pager: &'a Pager<SimRuntime>,
    txns: Option<&'a Transactions<SimRuntime>>,
    heaps: &'a [Heap],
    indexes: &'a [Index],
    trees: &'a [BTree],
}

impl std::fmt::Debug for Parts<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parts")
            .field("heaps", &self.heaps.len())
            .field("indexes", &self.indexes.len())
            .field("trees", &self.trees)
            .finish_non_exhaustive()
    }
}

impl<'a> Parts<'a> {
    pub fn new(
        pager: &'a Pager<SimRuntime>,
        txns: Option<&'a Transactions<SimRuntime>>,
        heaps: &'a [Heap],
        indexes: &'a [Index],
        trees: &'a [BTree],
    ) -> Parts<'a> {
        Parts {
            pager,
            txns,
            heaps,
            indexes,
            trees,
        }
    }

    /// Returns what `verify` finds wrong with the world, as
    /// [`verify_after_crash`](crate::recovery::verify_after_crash) does, over every structure in
    /// it: heap `i` as constructor `i` of one sum that every index ranges over.
    pub fn verify(&self) -> Vec<String> {
        let clog = self.txns.map(Transactions::commit_log);
        let inventory = Inventory {
            commit_log: clog.as_deref(),
            sums: vec![Sum {
                heaps: (0..).map(CtorId).zip(self.heaps).collect(),
                indexes: self.indexes.iter().collect(),
            }],
            trees: self.trees.to_vec(),
            pages: Vec::new(),
        };
        verify_after_crash(self.pager, &inventory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{finished, options};
    use crate::rows::row_type;
    use adtb::key::{Dir, KeyType, Scalar};
    use adtb_sim::{Config, Sim};

    /// Returns the owner ids of the large stores, heaps, indexes and trees of `s`, and the roots
    /// of its heaps.
    fn owners(s: &Storage) -> [Vec<u32>; 5] {
        [
            s.larges.iter().map(LargeStore::id).collect(),
            s.heaps.iter().map(|h| h.id().0).collect(),
            s.indexes.iter().map(|i| i.tree().owner()).collect(),
            s.trees.iter().map(BTree::owner).collect(),
            s.heaps.iter().map(|h| h.root().0 as u32).collect(),
        ]
    }

    #[test]
    fn reopen_finds_every_structure_create_made_under_its_owner() {
        let mut sim = Sim::new(3, Config::default());
        let mut made = None;
        finished(&mut sim, |rt| {
            let mut s = Storage::create(&rt, &options(16));
            s.commit_log(&rt);
            let large = s.large();
            s.heap(row_type(0), large, 4);
            s.heap(row_type(2), large, 4);
            let schema = KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)]);
            s.index(schema, BTreeSet::from([CtorId(0), CtorId(1)]));
            s.tree();
            s.pager.flush_log();
            made = Some((s.layout(), owners(&s)));
        });
        let (layout, made) = made.expect("the world is made");
        assert_eq!([vec![1], vec![2, 3], vec![4], vec![5]], made[..4]);
        finished(&mut sim, |rt| {
            let s = Storage::reopen(&rt, &layout).expect("the world reopens");
            assert!(s.txns.is_some(), "the commit log reopens");
            assert_eq!(made, owners(&s));
            assert_eq!(Vec::<String>::new(), s.verify());
        });
    }
}
