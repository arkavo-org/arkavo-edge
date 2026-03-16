use sha2::{Digest, Sha256};

/// A Merkle tree built from SHA-256 leaf hashes.
///
/// Supports inclusion proofs for individual leaves, enabling verification
/// that a specific OpBundle hash was included in an anchored batch.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    leaves: Vec<[u8; 32]>,
    /// All nodes in level-order. nodes[0] is root.
    nodes: Vec<[u8; 32]>,
}

/// Side indicator for a sibling in a Merkle proof path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// Inclusion proof for a single leaf in a Merkle tree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MerkleProof {
    pub leaf: [u8; 32],
    pub siblings: Vec<([u8; 32], Side)>,
    pub root: [u8; 32],
}

impl MerkleTree {
    /// Build a Merkle tree from leaf hashes.
    ///
    /// Pads to the next power of two by duplicating the last leaf.
    ///
    /// # Panics
    ///
    /// Panics if `hashes` is empty.
    pub fn from_hashes(hashes: Vec<[u8; 32]>) -> Self {
        assert!(
            !hashes.is_empty(),
            "Cannot build Merkle tree from empty leaves"
        );

        let padded_len = hashes.len().next_power_of_two();
        let mut leaves = hashes;
        let last = *leaves.last().unwrap();
        leaves.resize(padded_len, last);

        let total_nodes = 2 * padded_len - 1;
        let mut nodes = vec![[0u8; 32]; total_nodes];

        // Fill leaf level (bottom of tree, stored at end of nodes array)
        let leaf_start = padded_len - 1;
        for (i, leaf) in leaves.iter().enumerate() {
            nodes[leaf_start + i] = *leaf;
        }

        // Build internal nodes bottom-up
        for i in (0..leaf_start).rev() {
            let left = nodes[2 * i + 1];
            let right = nodes[2 * i + 2];
            nodes[i] = hash_pair(&left, &right);
        }

        Self { leaves, nodes }
    }

    /// The Merkle root hash.
    pub fn root(&self) -> [u8; 32] {
        self.nodes[0]
    }

    /// Number of original leaves (before padding).
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Generate an inclusion proof for the leaf at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= leaf_count()`.
    pub fn proof(&self, index: usize) -> MerkleProof {
        assert!(index < self.leaves.len(), "Leaf index out of bounds");

        let leaf_start = self.leaves.len() - 1;
        let mut node_idx = leaf_start + index;
        let mut siblings = Vec::new();

        while node_idx > 0 {
            let parent = (node_idx - 1) / 2;
            let left_child = 2 * parent + 1;
            let right_child = 2 * parent + 2;

            if node_idx == left_child {
                siblings.push((self.nodes[right_child], Side::Right));
            } else {
                siblings.push((self.nodes[left_child], Side::Left));
            }
            node_idx = parent;
        }

        MerkleProof {
            leaf: self.leaves[index],
            siblings,
            root: self.nodes[0],
        }
    }
}

impl MerkleProof {
    /// Verify that this proof is valid — the leaf hashes up to the claimed root.
    pub fn verify(&self) -> bool {
        let mut current = self.leaf;
        for (sibling, side) in &self.siblings {
            current = match side {
                Side::Left => hash_pair(sibling, &current),
                Side::Right => hash_pair(&current, sibling),
            };
        }
        current == self.root
    }
}

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(left);
    hasher.update(right);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }

    #[test]
    fn single_leaf_tree() {
        let h = leaf_hash(b"only leaf");
        let tree = MerkleTree::from_hashes(vec![h]);
        // Single leaf: padded to 2 (next_power_of_two for tree structure)
        // root = hash(h, h)
        let proof = tree.proof(0);
        assert!(proof.verify());
    }

    #[test]
    fn two_leaf_tree() {
        let a = leaf_hash(b"a");
        let b = leaf_hash(b"b");
        let tree = MerkleTree::from_hashes(vec![a, b]);
        assert_eq!(tree.root(), hash_pair(&a, &b));
    }

    #[test]
    fn four_leaf_tree() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves.clone());
        let left = hash_pair(&leaves[0], &leaves[1]);
        let right = hash_pair(&leaves[2], &leaves[3]);
        assert_eq!(tree.root(), hash_pair(&left, &right));
    }

    #[test]
    fn three_leaf_pads_to_four() {
        let leaves: Vec<_> = (0..3).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves.clone());
        // Fourth leaf should be duplicate of third
        let left = hash_pair(&leaves[0], &leaves[1]);
        let right = hash_pair(&leaves[2], &leaves[2]);
        assert_eq!(tree.root(), hash_pair(&left, &right));
    }

    #[test]
    fn proof_verifies_all_leaves() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves.clone());
        for i in 0..4 {
            let proof = tree.proof(i);
            assert!(proof.verify(), "Proof for leaf {i} should verify");
            assert_eq!(proof.leaf, leaves[i]);
            assert_eq!(proof.root, tree.root());
        }
    }

    #[test]
    fn proof_fails_with_wrong_root() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let mut proof = tree.proof(0);
        proof.root = [0xFF; 32]; // tamper
        assert!(!proof.verify());
    }

    #[test]
    fn proof_fails_with_wrong_leaf() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let mut proof = tree.proof(0);
        proof.leaf = [0xFF; 32]; // tamper
        assert!(!proof.verify());
    }

    #[test]
    fn proof_fails_with_tampered_sibling() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let mut proof = tree.proof(0);
        proof.siblings[0].0 = [0xFF; 32]; // tamper first sibling
        assert!(!proof.verify());
    }

    #[test]
    fn large_tree() {
        let leaves: Vec<_> = (0..100u32).map(|i| leaf_hash(&i.to_le_bytes())).collect();
        let tree = MerkleTree::from_hashes(leaves.clone());
        // Verify a sample of proofs
        for i in [0, 1, 50, 99] {
            let proof = tree.proof(i);
            assert!(proof.verify(), "Proof for leaf {i} should verify");
        }
    }

    #[test]
    fn deterministic_root() {
        let leaves: Vec<_> = (0..8).map(|i| leaf_hash(&[i])).collect();
        let tree1 = MerkleTree::from_hashes(leaves.clone());
        let tree2 = MerkleTree::from_hashes(leaves);
        assert_eq!(tree1.root(), tree2.root());
    }

    #[test]
    fn different_leaves_different_root() {
        let a: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let b: Vec<_> = (4..8).map(|i| leaf_hash(&[i])).collect();
        let ta = MerkleTree::from_hashes(a);
        let tb = MerkleTree::from_hashes(b);
        assert_ne!(ta.root(), tb.root());
    }

    #[test]
    #[should_panic(expected = "Cannot build Merkle tree from empty leaves")]
    fn empty_tree_panics() {
        MerkleTree::from_hashes(vec![]);
    }
}
