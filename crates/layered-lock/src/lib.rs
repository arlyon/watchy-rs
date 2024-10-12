#![cfg_attr(not(test), no_std)]

//! layered_lock
//!
//! A layered mutex to model recursive dependencies. Intermediate layers
//! can be used to model sleep states or enabling/disabling the modem.
//!
//! It is modeled as a series of locks, one for each leaf node, and a
//! set of dependencies between them. An intermediate lock is considered
//! open if all of its leaf nodes are open. Optionally, a layer can
//! choose to prevent its leaf nodes from being opened, to model a
//! graceful shutdown.

use core::{marker::PhantomData, num::NonZeroUsize};

pub trait AsyncMutex<T> {
    type Locked<'a>
    where
        T: 'a,
        Self: 'a;

    fn new(val: T) -> Self;
    async fn lock<'a>(&'a self) -> Self::Locked<'a>
    where
        T: 'a;
}

/// A layered lock.
///
/// All mutexes are stored in a heapless vector. The dependencies
/// are a list of integers that represent a non-zero offset into the
/// vector, which is the layer that the mutex points to.
///
/// A trivial case is a single layer, where the dependencies are empty.
/// A more complex case is a two layer lock. There is exactly one leaf
/// node, and the dependencies array is simply [0], pointing to the virtual
/// layer one along which is the root.
#[derive(Debug)]
pub struct LayeredLock<const MAX_LEAFS: usize, const MAX_CON: usize, M: AsyncMutex<bool>> {
    locks: heapless::Vec<M, MAX_LEAFS>,
    dependencies: heapless::Vec<usize, MAX_CON>,
}

impl<const L: usize, const C: usize, M: AsyncMutex<bool>> LayeredLock<L, C, M> {
    /// Create a new layered lock.
    pub fn new() -> (Self, Leaf<C, Open>) {
        let mut locks = heapless::Vec::new();
        locks.push(M::new(false)).unwrap_or_else(|_| panic!());
        let dependencies = heapless::Vec::from_slice(&[0]).unwrap(); // pointing to the end is a dep on the implicit 0th layer
        let path = heapless::Vec::from_slice(&[0]).unwrap();

        (
            Self {
                locks,
                dependencies,
            },
            Leaf {
                path,
                _data: PhantomData,
            },
        )
    }

    /// produce a sibling leaf with a path just one off
    pub fn push_sibling(&mut self, mut leaf: Leaf<C, Open>) -> (Leaf<C, Fused>, Leaf<C, Open>) {
        // push a new lock to the front
        self.locks
            .insert(0, M::new(false))
            .unwrap_or_else(|_| panic!());
        let (current, _) = find_node(&self.dependencies, &leaf.path).unwrap();
        let parent = current + self.dependencies[current] + 1;
        self.dependencies.insert(0, parent).unwrap();

        let mut orig = Leaf {
            path: leaf.path.clone(),
            _data: PhantomData,
        };

        let index = leaf.path.len() - 1;
        *leaf
            .path
            .get_mut(index)
            .expect("there is always 1 item in the path") += 1;

        (
            orig,
            Leaf {
                path: leaf.path,
                _data: PhantomData,
            },
        )
    }

    /// produce a child leaf with a path one longer
    pub fn push_child<S>(&mut self, mut leaf: Leaf<C, S>) -> (Layer<C>, Leaf<C, Open>) {
        // find the current item
        let (current, _) = find_node(&self.dependencies, &leaf.path).unwrap();
        // calculate offset from start of dep list to item
        self.dependencies.insert(0, current).unwrap();
        // push a new value at front pointing to item

        let mut path = leaf.path.clone();
        path.push(0).expect("does not exceed");
        (
            Layer { path: leaf.path },
            Leaf {
                path,
                _data: PhantomData,
            },
        )
    }
}

/// Find the node identified by path in the tree, using the list of pointers
///
/// # Arguments
/// - pointers: A list of relative offsets pointing from a node to its parent
/// - path: An array of sibling counts
///
/// # Algorithm
/// A given node is pointed to by n other nodes, which can be found by traversing
/// backwards from the target index and finding all other indices that point to it.
pub fn find_node(pointers: &[usize], path: &[usize]) -> Option<(usize, usize)> {
    path.iter().fold(Some((pointers.len(), 0)), |fold, nth| {
        let (curr_idx, total_skipped) = fold?;
        find_nth_sibling(*nth, &pointers[..curr_idx])
            .map(|(next_idx, skipped)| (next_idx, total_skipped + skipped))
    })
}

/// Given a list of pointers, finds the nth node that points to the end, while
/// summing up all the leaf nodes inbetween
///
/// # Returns
/// A tuple consisting of the idx of the nth sibling and the number of leaf
/// nodes passed to get there.
fn find_nth_sibling(nth: usize, pointers: &[usize]) -> Option<(usize, usize)> {
    let mut matches = 0;
    let mut leaves = 0;
    for (id, val) in pointers.iter().enumerate().rev() {
        #[cfg(test)]
        println!("looking at {} {}", id, val);

        // if val is > 0 then the last node must have been a leaf
        if *val > 0 {
            leaves += 1;
        }

        if id + val == pointers.len() - 1 {
            if matches == nth {
                #[cfg(test)]
                println!("yielding {}", id);
                return Some((id, leaves));
            }
            matches += 1;
            #[cfg(test)]
            println!("found {}th sibling", matches);
        }
    }
    None
}

/// Find the lock identified by the path in the tree. Walks the indices
/// to find all the nth leaf node and takes that from the back of the list.
///
/// This is O(n) right now but with the optimisation that if we
/// discover a sibling we can stop traversing that node.
///
/// # Returns
/// - Some(n) if the nth leaf node is found where n is the index from the
///           back of the list
/// - None if the nth leaf node is not found
fn find_lock_idx(pointers: &[usize], path: &[usize]) -> Option<usize> {
    find_node(pointers, path).map(|t| t.1)
}

pub struct Layer<const C: usize> {
    path: heapless::Vec<usize, C>,
}

/// A leaf node in the layered lock, identified by walking from the end
/// of the vector taking the nth child at each step.
#[derive(Clone)]
pub struct Leaf<const C: usize, State> {
    path: heapless::Vec<usize, C>,
    _data: PhantomData<State>,
}

/// no more siblings can be produced for this leaf
pub struct Fused;
/// siblings can be produced for this leaf
pub struct Open;

#[cfg(test)]
mod test {
    use super::*;
    use test_case::test_case;

    impl<T> AsyncMutex<T> for smol::lock::Mutex<T> {
        type Locked<'a>
            = async_lock::MutexGuard<'a, T>
        where
            T: 'a;

        fn new(val: T) -> Self {
            smol::lock::Mutex::new(val)
        }

        async fn lock<'a>(&'a self) -> Self::Locked<'a>
        where
            T: 'a,
        {
            smol::lock::Mutex::<T>::lock(self).await
        }
    }

    #[test_case(0, &[0] => Some((0, 0)) ; "trivial")]
    #[test_case(1, &[1, 0] => Some((0, 1)) ; "first sibling")]
    #[test_case(0, &[1, 0] => Some((1, 0)) ; "second sibling")]
    #[test_case(2, &[4, 0, 0, 1, 0] => Some((0, 2)) ; "skip over branches")]
    #[test_case(2, &[4, 8, 0, 1, 0] => Some((0, 3)) ; "handle subsequences")] // TODO: INVALID TREE
    #[test_case(5, &[2, 1, 3, 0, 4] => None ; "nth sibling out of range")]
    fn find_nth_sibling(nth: usize, siblings: &[usize]) -> Option<(usize, usize)> {
        super::find_nth_sibling(nth, siblings)
    }

    #[test_case(&[0], &[0] => Some((0, 0)) ; "trivial")]
    #[test_case(&[0, 0, 0], &[0, 0, 0] => Some((0, 0)) ; "trivial chain")]
    #[test_case(&[0, 1, 0, 0], &[0, 1, 0] => Some((0, 1)) ; "follow branch")]
    fn find_node(pointers: &[usize], path: &[usize]) -> Option<(usize, usize)> {
        super::find_node(pointers, path)
    }

    #[test]
    fn example() {
        let (mut lock, leaf) = LayeredLock::<2, 4, smol::lock::Mutex<bool>>::new();
        let (_, leaf) = lock.push_sibling(leaf);
        let (layer, leaf) = lock.push_child(leaf);
        let (layer, leaf) = lock.push_child(leaf);
        println!("{:?}", lock);
        assert_eq!(Some(1), find_lock_idx(&lock.dependencies, &leaf.path));
    }

    #[test]
    fn example2() {
        let (mut lock, leaf) = LayeredLock::<2, 4, smol::lock::Mutex<bool>>::new();
        let (_, leaf) = lock.push_sibling(leaf);
        println!("{:?}", lock);
        assert_eq!(Some(1), find_lock_idx(&lock.dependencies, &leaf.path));
    }
    #[test]
    fn example3() {
        let (mut lock, leaf) = LayeredLock::<2, 4, smol::lock::Mutex<bool>>::new();
        let (_, leaf) = lock.push_child(leaf);
        let (fused, leaf) = lock.push_sibling(leaf);
        let (_, leaf) = lock.push_child(leaf);
        println!("{:?}", lock);
        assert_eq!(Some(1), find_lock_idx(&lock.dependencies, &leaf.path));
        assert_eq!(Some(0), find_lock_idx(&lock.dependencies, &fused.path));
    }

    #[test]
    fn base_case() {
        let (mut lock, leaf) = LayeredLock::<1, 1, smol::lock::Mutex<bool>>::new();
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn child() {
        let (mut lock, leaf) = LayeredLock::<1, 2, smol::lock::Mutex<bool>>::new();
        let child = lock.push_child(leaf);
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn push_sibling_case() {
        let (mut lock, leaf) = LayeredLock::<2, 2, smol::lock::Mutex<bool>>::new();
        let (orig_leaf, new_leaf) = lock.push_sibling(leaf);

        // Make sure the original and new leaf are different
        assert_ne!(orig_leaf.path, new_leaf.path);
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn push_child_case() {
        let (mut lock, leaf) = LayeredLock::<1, 2, smol::lock::Mutex<bool>>::new();
        let (layer, child) = lock.push_child(leaf);

        // Ensure the child's path is a longer version of the layer
        assert_eq!(layer.path.len() + 1, child.path.len());
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn layered_lock_multiple_leaves() {
        let (mut lock, leaf1) = LayeredLock::<3, 3, smol::lock::Mutex<bool>>::new();
        let (_, leaf2) = lock.push_sibling(leaf1);
        let (_, leaf3) = lock.push_sibling(leaf2);

        // Ensure there are three unique leaves with distinct paths
        assert_eq!(lock.locks.len(), 3);

        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn push_child_after_sibling() {
        let (mut lock, leaf1) = LayeredLock::<2, 3, smol::lock::Mutex<bool>>::new();
        let (_, leaf2) = lock.push_sibling(leaf1);
        println!("{:?}", lock);
        let (layer, child) = lock.push_child(leaf2);
        println!("{:?}", lock);

        // Ensure the child path is one longer than the layer
        assert_eq!(layer.path.len() + 1, child.path.len());
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn push_multiple_children() {
        let (mut lock, leaf) = LayeredLock::<1, 3, smol::lock::Mutex<bool>>::new();
        let (layer1, child1) = lock.push_child(leaf);
        let (layer2, child2) = lock.push_child(child1);

        // Ensure each child path is one longer than the previous layer
        // assert_eq!(layer1.path.len() + 1, child1.path.len());
        assert_eq!(layer2.path.len() + 1, child2.path.len());
        insta::assert_debug_snapshot!(lock);
    }

    #[test]
    fn push_sibling_after_child() {
        let (mut lock, leaf) = LayeredLock::<2, 3, smol::lock::Mutex<bool>>::new();
        let (layer, child) = lock.push_child(leaf);
        println!("{:?}", lock);
        let (orig_leaf, sibling) = lock.push_sibling(child);

        // Ensure the sibling has the same path length as the original leaf
        assert_eq!(orig_leaf.path.len(), sibling.path.len());
        // Ensure the paths are distinct
        assert_ne!(orig_leaf.path, sibling.path);
        insta::assert_debug_snapshot!(lock);
    }
}
