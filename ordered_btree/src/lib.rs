use std::rc::{Rc, Weak};

use arrayvec::ArrayVec;
use qcell::{
    generativity::{Guard, Id},
    LCell, LCellOwner,
};

// BP = m + 1
pub struct RootWithOwner<'owner, V, const BP: usize> {
    root_chunk: LCell<'owner, Rc<LCell<'owner, Chunk<'owner, V, BP>>>>,
    owner: LCellOwner<'owner>,
}

pub type OrderedBList<V, const BP: usize> = RootWithOwner<'static, V, BP>;

struct Chunk<'owner, V, const BP: usize> {
    edges: Edges<'owner, V, BP>,
    parent: Option<Weak<LCell<'owner, Chunk<'owner, V, BP>>>>,
}
struct LeafNode<'owner, V, const BP: usize> {
    value: V,
    parent: Weak<LCell<'owner, Chunk<'owner, V, BP>>>,
}

enum Edges<'owner, V, const BP: usize> {
    Chunks {
        chunks: ArrayVec<Rc<LCell<'owner, Chunk<'owner, V, BP>>>, BP>,
        counts: ArrayVec<usize, BP>,
    },
    Leaves {
        leaves: ArrayVec<Rc<LCell<'owner, LeafNode<'owner, V, BP>>>, BP>,
    },
}

impl<'owner, V, const BP: usize> Edges<'owner, V, BP> {
    fn set_all_parents(
        &self,
        parent: Weak<LCell<'owner, Chunk<'owner, V, BP>>>,
        owner: &mut LCellOwner<'owner>,
    ) {
        match self {
            Edges::Chunks { chunks, .. } => {
                for chunk in chunks.iter() {
                    chunk.rw(owner).parent = Some(parent.clone());
                }
            }
            Edges::Leaves { leaves } => {
                for leaf in leaves.iter() {
                    leaf.rw(owner).parent = parent.clone();
                }
            }
        }
    }
    fn get_count(&self) -> usize {
        match self {
            Edges::Chunks { counts, .. } => counts.iter().sum(),
            Edges::Leaves { leaves } => leaves.len(),
        }
    }
    fn get_length(&self) -> usize {
        match self {
            Edges::Chunks { counts, .. } => counts.len(),
            Edges::Leaves { leaves } => leaves.len(),
        }
    }
}

const VIOL_CONNECTION: &str = "invariant violation: parent-child connection";
const VIOL_LEAF_DEPTH: &str = "invariant violation: uniform leaf depth";
pub struct InsertedIdWithOwner<'owner, V, const BP: usize> {
    node: Rc<LCell<'owner, LeafNode<'owner, V, BP>>>,
}

pub type InsertedId<V, const BP: usize> = InsertedIdWithOwner<'static, V, BP>;

fn search_counts(counts: &[usize], index: usize) -> Option<(usize, usize)> {
    let mut acc = 0;
    for (i, count) in counts.iter().enumerate() {
        acc += count;
        if acc > index {
            return Some((i, (index + count) - acc));
        }
    }
    return None;
}
fn idx_among_siblings<T>(siblings: &[Rc<T>], chunk: &Rc<T>) -> usize {
    siblings
        .iter()
        .position(|sib| Rc::ptr_eq(chunk, sib))
        .expect(VIOL_CONNECTION)
}

impl<'owner, V, const BP: usize> RootWithOwner<'owner, V, BP> {
    const HALF: usize = BP / 2; // ceil(m / 2)

    pub fn get_id(&self, index: usize) -> InsertedIdWithOwner<'owner, V, BP> {
        self.search(&*self.root_chunk.ro(&self.owner), index)
    }
    pub fn insert_before_id(&mut self, value: V, id: &InsertedIdWithOwner<'owner, V, BP>) {
        self.insert_with_offset(value, 0, id)
    }
    pub fn insert_after_id(&mut self, value: V, id: &InsertedIdWithOwner<'owner, V, BP>) {
        self.insert_with_offset(value, 1, id)
    }
    fn insert_with_offset(
        &mut self,
        value: V,
        offset: usize,
        id: &InsertedIdWithOwner<'owner, V, BP>,
    ) {
        let chunk_rc = id
            .node
            .ro(&self.owner)
            .parent
            .upgrade()
            .expect(VIOL_CONNECTION)
            .to_owned();
        let chunk = chunk_rc.rw(&mut self.owner);
        match &mut chunk.edges {
            Edges::Leaves { leaves } => {
                let pos = idx_among_siblings(leaves, &id.node);
                let new_leaf = LeafNode {
                    parent: Rc::downgrade(&chunk_rc),
                    value,
                };
                let new_leaf = Rc::new(LCell::new(new_leaf));
                leaves.insert(pos + offset, new_leaf);
                let leaves_len = leaves.len();
                self.modify_count(&chunk_rc, 1);
                if leaves_len == BP {
                    self.rebalance_overflow(&chunk_rc)
                }
            }
            Edges::Chunks { .. } => {
                unreachable!("{}", VIOL_LEAF_DEPTH)
            }
        }
    }
    pub fn remove(&mut self, id: &InsertedIdWithOwner<'owner, V, BP>) {
        let chunk_rc = id
            .node
            .ro(&self.owner)
            .parent
            .upgrade()
            .expect(VIOL_CONNECTION)
            .to_owned();
        let chunk = chunk_rc.rw(&mut self.owner);
        match &mut chunk.edges {
            Edges::Leaves { leaves } => {
                let pos = idx_among_siblings(leaves, &id.node);
                leaves.remove(pos);
                let leaves_len = leaves.len();
                self.modify_count(&chunk_rc, -1);
                if leaves_len < Self::HALF {
                    self.rebalance_underflow(&chunk_rc);
                }
            }
            Edges::Chunks { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
        }
    }
    pub fn value<'b>(&'b self, id: &'b InsertedIdWithOwner<'owner, V, BP>) -> &'b V {
        &id.node.ro(&self.owner).value
    }
    pub fn value_mut<'b>(&'b mut self, id: &'b InsertedIdWithOwner<'owner, V, BP>) -> &'b mut V {
        &mut id.node.rw(&mut self.owner).value
    }
    pub fn get_index(&self, id: &InsertedIdWithOwner<'owner, V, BP>) -> usize {
        let chunk_rc = id
            .node
            .ro(&self.owner)
            .parent
            .upgrade()
            .expect(VIOL_CONNECTION)
            .to_owned();
        let mut pos = match &chunk_rc.ro(&self.owner).edges {
            Edges::Leaves { leaves } => idx_among_siblings(leaves, &id.node),
            Edges::Chunks { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
        };
        let mut parent_chunk = chunk_rc;
        loop {
            let Some(p) = parent_chunk.ro(&self.owner).parent.as_ref() else {break};
            parent_chunk = p.upgrade().expect(VIOL_CONNECTION);
            match &parent_chunk.ro(&self.owner).edges {
                Edges::Chunks { chunks, counts } => {
                    pos += counts[0..idx_among_siblings(chunks, &parent_chunk)]
                        .iter()
                        .sum::<usize>();
                }
                Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
            }
        }
        pos
    }
    pub fn new_with_owner(owner: LCellOwner<'owner>) -> Self {
        Self {
            root_chunk: LCell::new(Rc::new(LCell::new(Chunk {
                edges: Edges::Leaves {
                    leaves: ArrayVec::new(),
                },
                parent: None,
            }))),
            owner,
        }
    }
}
impl<V, const BP: usize> OrderedBList<V, BP> {
    pub fn new() -> Self {
        Self::new_with_owner(LCellOwner::new(unsafe { Guard::new(Id::new()) }))
    }
}
impl<'owner, V, const BP: usize> RootWithOwner<'owner, V, BP> {
    fn search(
        &self,
        chunk: &Rc<LCell<'owner, Chunk<'owner, V, BP>>>,
        index: usize,
    ) -> InsertedIdWithOwner<'owner, V, BP> {
        let me = &*chunk.ro(&self.owner);
        match &me.edges {
            Edges::Chunks {
                chunks: edges,
                counts,
            } => {
                let (pos, inner_index) = search_counts(&*counts, index).expect("out of bound");
                self.search(&edges[pos], inner_index)
            }
            Edges::Leaves { leaves } => InsertedIdWithOwner {
                node: leaves[index].to_owned(),
            },
        }
    }

    fn modify_count(&mut self, chunk: &Rc<LCell<'owner, Chunk<'owner, V, BP>>>, delta: isize) {
        if let Some(parent) = chunk
            .ro(&self.owner)
            .parent
            .as_ref()
            .map(|p| p.upgrade().expect(VIOL_CONNECTION))
        {
            let parent_chunk = parent.rw(&mut self.owner);
            match &mut parent_chunk.edges {
                Edges::Chunks { chunks, counts } => {
                    let pos = idx_among_siblings(chunks, chunk);
                    let count = counts
                        .get_mut(pos)
                        .expect("invariant violation: counts-edges parallelism");
                    *count = ((*count as isize) + delta) as usize;
                }
                Edges::Leaves { .. } => unreachable!("{}", VIOL_CONNECTION),
            }
            self.modify_count(&parent, delta);
        }
    }

    fn rebalance_overflow(&mut self, chunk: &Rc<LCell<'owner, Chunk<'owner, V, BP>>>) {
        let me = chunk.rw(&mut self.owner);
        let new_edges = match &mut me.edges {
            Edges::Chunks { chunks, counts } => Edges::Chunks {
                chunks: chunks.drain(Self::HALF..).collect(),
                counts: counts.drain(Self::HALF..).collect(),
            },
            Edges::Leaves { leaves } => {
                let leaves: ArrayVec<Rc<LCell<'owner, LeafNode<V, BP>>>, BP> =
                    leaves.drain(Self::HALF..).collect();
                Edges::Leaves { leaves }
            }
        };
        let new_count = new_edges.get_count();

        let new_chunk = Chunk {
            edges: new_edges,
            parent: me.parent.clone(),
        };
        let new_chunk = Rc::new_cyclic(|w| {
            new_chunk
                .edges
                .set_all_parents(w.to_owned(), &mut self.owner);
            LCell::new(new_chunk)
        });

        if let Some(parent_rc) = chunk
            .ro(&self.owner)
            .parent
            .as_ref()
            .map(|p| p.upgrade().expect(VIOL_CONNECTION))
        {
            let parent_chunk = parent_rc.rw(&mut self.owner);
            match &mut parent_chunk.edges {
                Edges::Chunks {
                    chunks: sibling_chunks,
                    counts: sibling_counts,
                } => {
                    let pos = idx_among_siblings(sibling_chunks, chunk);
                    sibling_chunks.insert(pos, new_chunk);
                    sibling_counts[pos] -= new_count;
                    sibling_counts.insert(pos, new_count);
                    if sibling_chunks.len() == BP {
                        self.rebalance_overflow(&parent_rc);
                    }
                }
                Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
            }
        } else {
            let new_root = Chunk {
                edges: Edges::Chunks {
                    chunks: [chunk.clone(), new_chunk].into_iter().collect(),
                    counts: [chunk.ro(&self.owner).edges.get_count(), new_count]
                        .into_iter()
                        .collect(),
                },
                parent: None,
            };
            let new_root = Rc::new_cyclic(|w| {
                new_root
                    .edges
                    .set_all_parents(w.to_owned(), &mut self.owner);
                LCell::new(new_root)
            });
            *self.root_chunk.rw(&mut self.owner) = new_root;
        }
    }
    fn rebalance_underflow(&mut self, chunk: &Rc<LCell<'owner, Chunk<'owner, V, BP>>>) {
        if let Some(parent_rc) = chunk
            .ro(&self.owner)
            .parent
            .as_ref()
            .map(|p| p.upgrade().expect(VIOL_CONNECTION))
        {
            let (pos, next_chunk, prev_chunk) = match &parent_rc.ro(&self.owner).edges {
                Edges::Chunks {
                    chunks: sibling_chunks,
                    ..
                } => {
                    let pos = idx_among_siblings(sibling_chunks, chunk);
                    let next_chunk = sibling_chunks.get(pos + 1).cloned();
                    let prev_chunk = (if pos > 0 {
                        sibling_chunks.get(pos - 1)
                    } else {
                        None
                    })
                    .cloned();

                    (pos, next_chunk, prev_chunk)
                }
                Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
            };
            let next_chunk = next_chunk.map(|c| (c.ro(&self.owner).edges.get_length(), c));
            let prev_chunk = prev_chunk.map(|c| (c.ro(&self.owner).edges.get_length(), c));
            let parent_chunk = parent_rc.rw(&mut self.owner);
            match &mut parent_chunk.edges {
                Edges::Chunks {
                    counts: sibling_counts,
                    ..
                } => {
                    match (next_chunk, prev_chunk) {
                        (Some((next_chunk_len, next_chunk)), _) if next_chunk_len > Self::HALF => {
                            // transfer from next chunk
                            sibling_counts[pos] += 1;
                            sibling_counts[pos + 1] -= 1;
                            let chunk_len = chunk.ro(&self.owner).edges.get_length();
                            self.transfer_item(next_chunk, chunk.to_owned(), 0, chunk_len)
                        }
                        (_, Some((prev_chunk_len, prev_chunk))) if prev_chunk_len > Self::HALF => {
                            // transfer from previous chunk
                            sibling_counts[pos] += 1;
                            sibling_counts[pos - 1] -= 1;
                            self.transfer_item(prev_chunk, chunk.to_owned(), prev_chunk_len - 1, 0)
                        }
                        (Some(_), _) => {
                            // merge with next chunk
                            self.merge_chunks(parent_rc, pos);
                        }
                        (_, Some(_)) => {
                            // merge with previous chunk
                            self.merge_chunks(parent_rc, pos - 1);
                        }
                        (None, None) => {
                            // only node, should become root
                            chunk.rw(&mut self.owner).parent = None;
                            *self.root_chunk.rw(&mut self.owner) = chunk.to_owned();
                        }
                    }
                }
                Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
            }
        }
    }
    fn merge_chunks(&mut self, parent: Rc<LCell<'owner, Chunk<'owner, V, BP>>>, l_idx: usize) {
        let (r_chunk, r_count, underflowed) = match &mut parent.rw(&mut self.owner).edges {
            Edges::Chunks { chunks, counts } => (
                chunks.remove(l_idx + 1),
                counts.remove(l_idx + 1),
                chunks.len() < Self::HALF,
            ),
            Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
        };
        let r_taken = Rc::try_unwrap(r_chunk)
            .map_err(|_e| ())
            .expect(VIOL_CONNECTION)
            .into_inner();
        let l_chunk = match &mut parent.rw(&mut self.owner).edges {
            Edges::Chunks { chunks, counts } => {
                counts[l_idx] += r_count;
                chunks[l_idx].to_owned()
            }
            Edges::Leaves { .. } => unreachable!("{}", VIOL_LEAF_DEPTH),
        };
        r_taken
            .edges
            .set_all_parents(Rc::downgrade(&l_chunk), &mut self.owner);
        match (&mut l_chunk.rw(&mut self.owner).edges, r_taken.edges) {
            (
                Edges::Chunks { chunks, counts },
                Edges::Chunks {
                    chunks: r_chunks,
                    counts: r_counts,
                },
            ) => {
                chunks.extend(r_chunks.into_iter());
                counts.extend(r_counts.into_iter());
            }
            (Edges::Leaves { leaves }, Edges::Leaves { leaves: r_leaves }) => {
                leaves.extend(r_leaves.into_iter())
            }
            _ => unreachable!("{}", VIOL_LEAF_DEPTH),
        }
        if underflowed {
            self.rebalance_underflow(&parent);
        }
    }
    fn transfer_item(
        &mut self,
        src_chunk: Rc<LCell<'owner, Chunk<'owner, V, BP>>>,
        dst_chunk: Rc<LCell<'owner, Chunk<'owner, V, BP>>>,
        src_chunk_idx: usize,
        dst_chunk_idx: usize,
    ) {
        match &mut src_chunk.rw(&mut self.owner).edges {
            Edges::Chunks { chunks, counts } => {
                let removed_chunk = chunks.remove(src_chunk_idx);
                let removed_count = counts.remove(src_chunk_idx);
                match &mut dst_chunk.rw(&mut self.owner).edges {
                    Edges::Chunks { chunks, counts } => {
                        chunks.insert(dst_chunk_idx, removed_chunk);
                        counts.insert(dst_chunk_idx, removed_count);
                    }
                    _ => unreachable!("{}", VIOL_LEAF_DEPTH),
                }
            }
            Edges::Leaves { leaves } => {
                let removed_leaf = leaves.remove(src_chunk_idx);
                match &mut dst_chunk.rw(&mut self.owner).edges {
                    Edges::Leaves { leaves } => {
                        leaves.insert(dst_chunk_idx, removed_leaf);
                    }
                    _ => unreachable!("{}", VIOL_LEAF_DEPTH),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Both<T, const BP: usize> {
        vec: Vec<T>,
        obl: OrderedBList<T, BP>,
    }

    impl<T: Clone, const BP: usize> Both<T, BP> {
        fn new() -> Self {
            Self {
                vec: Vec::new(),
                obl: OrderedBList::new(),
            }
        }
        fn insert(&mut self, index: usize, element: T) {}
    }

    #[test]
    fn it_works() {}
}
