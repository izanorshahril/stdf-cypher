//! Isolation Forest (Liu, Ting & Zhou 2008) for unsupervised outlier scoring.

use super::features::FeatureMatrix;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

enum INode {
    Split {
        feat: usize,
        thr: f64,
        left: usize,
        right: usize,
    },
    Leaf(usize),
}

struct ITree {
    nodes: Vec<INode>,
}

pub struct IsolationForest {
    trees: Vec<ITree>,
    psi: usize,
}

/// Average path length of an unsuccessful BST search over n points.
fn c(n: usize) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    if n == 2 {
        return 1.0;
    }
    let n = n as f64;
    2.0 * ((n - 1.0).ln() + 0.5772156649) - 2.0 * (n - 1.0) / n
}

fn grow(
    fm: &FeatureMatrix,
    idx: &mut [usize],
    depth: usize,
    max_depth: usize,
    nodes: &mut Vec<INode>,
    rng: &mut impl Rng,
) -> usize {
    let me = nodes.len();
    nodes.push(INode::Leaf(idx.len()));
    if idx.len() <= 1 || depth >= max_depth || fm.p == 0 {
        return me;
    }
    // try a few features to find one with spread
    for _ in 0..8 {
        let f = rng.gen_range(0..fm.p);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for &i in idx.iter() {
            let v = fm.get(i, f);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        if hi <= lo {
            continue;
        }
        let thr = rng.gen_range(lo..hi);
        let mut k = 0;
        for j in 0..idx.len() {
            if fm.get(idx[j], f) < thr {
                idx.swap(j, k);
                k += 1;
            }
        }
        let (l, r) = idx.split_at_mut(k);
        let li = grow(fm, l, depth + 1, max_depth, nodes, rng);
        let ri = grow(fm, r, depth + 1, max_depth, nodes, rng);
        nodes[me] = INode::Split {
            feat: f,
            thr,
            left: li,
            right: ri,
        };
        return me;
    }
    me
}

impl IsolationForest {
    pub fn fit(fm: &FeatureMatrix, n_trees: usize, psi: usize, seed: u64) -> Self {
        let psi = psi.min(fm.n).max(2);
        let max_depth = (psi as f64).log2().ceil() as usize;
        let trees = (0..n_trees)
            .into_par_iter()
            .map(|t| {
                let mut rng =
                    rand::rngs::StdRng::seed_from_u64(seed.wrapping_add(t as u64 * 104729));
                let mut s: Vec<usize> = (0..psi).map(|_| rng.gen_range(0..fm.n)).collect();
                let mut nodes = vec![];
                grow(fm, &mut s, 0, max_depth, &mut nodes, &mut rng);
                ITree { nodes }
            })
            .collect();
        Self { trees, psi }
    }

    fn path(&self, t: &ITree, x: &[f64]) -> f64 {
        let mut n = 0;
        let mut d = 0.0;
        loop {
            match &t.nodes[n] {
                INode::Leaf(size) => return d + c(*size),
                INode::Split {
                    feat,
                    thr,
                    left,
                    right,
                } => {
                    d += 1.0;
                    n = if x[*feat] < *thr { *left } else { *right };
                }
            }
        }
    }

    /// Anomaly score in (0,1]; > 0.6 is typically anomalous.
    pub fn score(&self, x: &[f64]) -> f64 {
        let e: f64 =
            self.trees.iter().map(|t| self.path(t, x)).sum::<f64>() / self.trees.len() as f64;
        2f64.powf(-e / c(self.psi))
    }

    pub fn score_all(&self, fm: &FeatureMatrix) -> Vec<f64> {
        (0..fm.n)
            .into_par_iter()
            .map(|i| self.score(fm.row(i)))
            .collect()
    }
}
