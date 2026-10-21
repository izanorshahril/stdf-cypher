//! Random forest (classification and regression) on histogram-binned
//! features, with impurity-based feature importance. Used to find which
//! parametric tests explain a bin / pass-fail outcome or another test.

use super::features::FeatureMatrix;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ForestOptions {
    pub n_trees: usize,
    pub max_depth: usize,
    pub min_leaf: usize,
    pub mtry: Option<usize>,
    pub seed: u64,
    /// inverse-frequency class weights (recommended for yield data)
    pub balanced: bool,
    pub n_bins: usize,
    /// hold-out fraction for evaluation
    pub test_frac: f64,
}

impl Default for ForestOptions {
    fn default() -> Self {
        Self {
            n_trees: 100,
            max_depth: 12,
            min_leaf: 1,
            mtry: None,
            seed: 7,
            balanced: true,
            n_bins: 64,
            test_frac: 0.25,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Target {
    Class { y: Vec<usize>, names: Vec<String> },
    Reg(Vec<f64>),
}

#[derive(Debug, Clone)]
enum Node {
    Split {
        feat: usize,
        bin: u8,
        left: usize,
        right: usize,
    },
    Leaf(Vec<f64>),
}

#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
}

pub struct Forest {
    trees: Vec<Tree>,
    edges: Vec<Vec<f64>>,
    n_out: usize,
    pub importance: Vec<f64>,
    classification: bool,
    /// out-of-bag class probabilities (or prediction) for training rows
    pub oob: Vec<Option<Vec<f64>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForestReport {
    pub kind: String,
    pub target: String,
    pub n_train: usize,
    pub n_test: usize,
    pub n_features: usize,
    pub class_names: Vec<String>,
    pub class_counts: Vec<usize>,
    pub accuracy: Option<f64>,
    pub balanced_accuracy: Option<f64>,
    /// ROC AUC on the hold-out (binary) or macro one-vs-rest AUC (multi-class)
    pub auc: Option<f64>,
    /// decision threshold on P(class 1) for binary targets, tuned on out-of-bag data (Youden's J)
    pub threshold: Option<f64>,
    /// confusion[actual][predicted] on the hold-out set
    pub confusion: Vec<Vec<usize>>,
    pub r2: Option<f64>,
    pub rmse: Option<f64>,
    pub mae: Option<f64>,
    /// (feature, importance) sorted descending, normalized to sum 1
    pub importance: Vec<(String, f64)>,
    #[serde(skip)]
    pub test_pred: Vec<(f64, f64)>,
}

fn bin_features(fm: &FeatureMatrix, n_bins: usize) -> (Vec<Vec<f64>>, Vec<Vec<u8>>) {
    let n_bins = n_bins.clamp(2, 255);
    let cols: Vec<(Vec<f64>, Vec<u8>)> = (0..fm.p)
        .into_par_iter()
        .map(|j| {
            let mut v: Vec<f64> = (0..fm.n)
                .step_by((fm.n / 5000).max(1))
                .map(|i| fm.get(i, j))
                .collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
            let mut edges: Vec<f64> = (1..n_bins).map(|k| q(k as f64 / n_bins as f64)).collect();
            // resolve the tails, where failing parts live, and split exactly at test limits
            edges.extend(
                [0.001, 0.003, 0.006, 0.994, 0.997, 0.999]
                    .iter()
                    .map(|&p| q(p)),
            );
            edges.extend(fm.limit_edges.get(j).cloned().unwrap_or_default());
            // the sample may miss rare extremes: use the full column's min/max region too
            let (mn, mx) = (0..fm.n).fold((f64::INFINITY, f64::NEG_INFINITY), |a, i| {
                (a.0.min(fm.get(i, j)), a.1.max(fm.get(i, j)))
            });
            if mn.is_finite() {
                edges.push(mn);
                edges.push(mx);
            }
            edges.sort_by(|a, b| a.partial_cmp(b).unwrap());
            edges.dedup();
            edges.truncate(254);
            let b: Vec<u8> = (0..fm.n)
                .map(|i| edges.partition_point(|&e| e < fm.get(i, j)) as u8)
                .collect();
            (edges, b)
        })
        .collect();
    cols.into_iter().unzip()
}

struct Ctx<'a> {
    binned: &'a [Vec<u8>],
    n_bins: Vec<usize>,
    y_class: Option<&'a [usize]>,
    y_reg: Option<&'a [f64]>,
    w: &'a [f64],
    n_out: usize,
    opt: &'a ForestOptions,
}

fn impurity(stats: &[f64], tot_w: f64, classification: bool) -> f64 {
    if tot_w <= 0.0 {
        return 0.0;
    }
    if classification {
        1.0 - stats.iter().map(|c| (c / tot_w).powi(2)).sum::<f64>()
    } else {
        // stats = [sum, sumsq]
        let m = stats[0] / tot_w;
        (stats[1] / tot_w - m * m).max(0.0)
    }
}

fn node_stats(ctx: &Ctx, idx: &[usize]) -> (Vec<f64>, f64) {
    let mut s = vec![0.0; if ctx.y_class.is_some() { ctx.n_out } else { 2 }];
    let mut tw = 0.0;
    for &i in idx {
        let w = ctx.w[i];
        tw += w;
        if let Some(y) = ctx.y_class {
            s[y[i]] += w;
        } else {
            let v = ctx.y_reg.unwrap()[i];
            s[0] += w * v;
            s[1] += w * v * v;
        }
    }
    (s, tw)
}

fn leaf(ctx: &Ctx, stats: &[f64], tw: f64) -> Node {
    if ctx.y_class.is_some() {
        Node::Leaf(stats.iter().map(|c| c / tw.max(1e-12)).collect())
    } else {
        Node::Leaf(vec![stats[0] / tw.max(1e-12)])
    }
}

fn build(
    ctx: &Ctx,
    idx: &mut [usize],
    depth: usize,
    nodes: &mut Vec<Node>,
    imp: &mut [f64],
    rng: &mut impl Rng,
) -> usize {
    let cls = ctx.y_class.is_some();
    let (stats, tw) = node_stats(ctx, idx);
    let me = nodes.len();
    nodes.push(Node::Leaf(vec![]));
    let parent_imp = impurity(&stats, tw, cls);
    if depth >= ctx.opt.max_depth || idx.len() < 2 * ctx.opt.min_leaf || parent_imp <= 1e-12 {
        nodes[me] = leaf(ctx, &stats, tw);
        return me;
    }
    let p = ctx.binned.len();
    let mtry = ctx
        .opt
        .mtry
        .unwrap_or_else(|| {
            if cls {
                (p as f64).sqrt().ceil() as usize
            } else {
                (p / 3).max(1)
            }
        })
        .min(p)
        .max(1);
    let mut feats: Vec<usize> = (0..p).collect();
    feats.partial_shuffle(rng, mtry);
    let k = stats.len();
    let mut best: Option<(f64, usize, u8)> = None;
    for &f in &feats[..mtry] {
        let nb = ctx.n_bins[f];
        let mut h = vec![0.0; nb * k];
        let mut hw = vec![0.0; nb];
        let mut hc = vec![0usize; nb];
        let col = &ctx.binned[f];
        for &i in idx.iter() {
            let b = col[i] as usize;
            let w = ctx.w[i];
            hw[b] += w;
            hc[b] += 1;
            if let Some(y) = ctx.y_class {
                h[b * k + y[i]] += w;
            } else {
                let v = ctx.y_reg.unwrap()[i];
                h[b * k] += w * v;
                h[b * k + 1] += w * v * v;
            }
        }
        let mut left = vec![0.0; k];
        let (mut lw, mut lc) = (0.0, 0usize);
        for b in 0..nb.saturating_sub(1) {
            for c in 0..k {
                left[c] += h[b * k + c];
            }
            lw += hw[b];
            lc += hc[b];
            let rc = idx.len() - lc;
            if lc < ctx.opt.min_leaf || rc < ctx.opt.min_leaf {
                continue;
            }
            let right: Vec<f64> = stats.iter().zip(&left).map(|(s, l)| s - l).collect();
            let rw = tw - lw;
            let child = lw * impurity(&left, lw, cls) + rw * impurity(&right, rw, cls);
            let gain = tw * parent_imp - child;
            if gain > 1e-12 && best.map(|bb| gain > bb.0).unwrap_or(true) {
                best = Some((gain, f, b as u8));
            }
        }
    }
    let Some((gain, f, b)) = best else {
        nodes[me] = leaf(ctx, &stats, tw);
        return me;
    };
    imp[f] += gain;
    let col = &ctx.binned[f];
    let mut lo = 0;
    for i in 0..idx.len() {
        if col[idx[i]] <= b {
            idx.swap(i, lo);
            lo += 1;
        }
    }
    let (l_idx, r_idx) = idx.split_at_mut(lo);
    let l = build(ctx, l_idx, depth + 1, nodes, imp, rng);
    let r = build(ctx, r_idx, depth + 1, nodes, imp, rng);
    nodes[me] = Node::Split {
        feat: f,
        bin: b,
        left: l,
        right: r,
    };
    me
}

impl Tree {
    fn predict(&self, binned: &[Vec<u8>], i: usize) -> &[f64] {
        let mut n = 0;
        loop {
            match &self.nodes[n] {
                Node::Leaf(v) => return v,
                Node::Split {
                    feat,
                    bin,
                    left,
                    right,
                } => {
                    n = if binned[*feat][i] <= *bin {
                        *left
                    } else {
                        *right
                    };
                }
            }
        }
    }
}

impl Forest {
    pub fn fit(
        fm: &FeatureMatrix,
        target: &Target,
        train: &[usize],
        opt: &ForestOptions,
    ) -> Forest {
        let (edges, binned) = bin_features(fm, opt.n_bins);
        Self::fit_binned(edges, &binned, target, train, opt)
    }

    fn fit_binned(
        edges: Vec<Vec<f64>>,
        binned: &[Vec<u8>],
        target: &Target,
        train: &[usize],
        opt: &ForestOptions,
    ) -> Forest {
        let n = binned.first().map(|c| c.len()).unwrap_or(0);
        let (y_class, y_reg, n_out) = match target {
            Target::Class { y, names } => (Some(y.as_slice()), None, names.len()),
            Target::Reg(y) => (None, Some(y.as_slice()), 1),
        };
        let mut w = vec![1.0; n];
        if let (Some(y), true) = (y_class, opt.balanced) {
            let mut cnt = vec![0usize; n_out];
            train.iter().for_each(|&i| cnt[y[i]] += 1);
            let present = cnt.iter().filter(|&&c| c > 0).count().max(1) as f64;
            for i in 0..n {
                let c = cnt[y[i]];
                w[i] = if c > 0 {
                    train.len() as f64 / (present * c as f64)
                } else {
                    0.0
                };
            }
        }
        let n_bins: Vec<usize> = edges.iter().map(|e| e.len() + 1).collect();
        let ctx = Ctx {
            binned,
            n_bins,
            y_class,
            y_reg,
            w: &w,
            n_out,
            opt,
        };
        let results: Vec<(Tree, Vec<f64>, Vec<(usize, Vec<f64>)>)> = (0..opt.n_trees)
            .into_par_iter()
            .map(|t| {
                let mut rng =
                    rand::rngs::StdRng::seed_from_u64(opt.seed.wrapping_add(t as u64 * 7919));
                let mut sample: Vec<usize> = (0..train.len())
                    .map(|_| train[rng.gen_range(0..train.len())])
                    .collect();
                let mut nodes = vec![];
                let mut imp = vec![0.0; binned.len()];
                let mut in_bag = vec![false; n];
                sample.iter().for_each(|&i| in_bag[i] = true);
                build(&ctx, &mut sample, 0, &mut nodes, &mut imp, &mut rng);
                let tree = Tree { nodes };
                let oob: Vec<(usize, Vec<f64>)> = train
                    .iter()
                    .filter(|&&i| !in_bag[i])
                    .map(|&i| (i, tree.predict(binned, i).to_vec()))
                    .collect();
                (tree, imp, oob)
            })
            .collect();
        let mut importance = vec![0.0; binned.len()];
        let mut trees = vec![];
        let mut oob_sum: Vec<(Vec<f64>, usize)> = vec![(vec![0.0; n_out.max(1)], 0); n];
        for (t, imp, oob) in results {
            importance.iter_mut().zip(imp).for_each(|(a, b)| *a += b);
            trees.push(t);
            for (i, p) in oob {
                oob_sum[i].0.iter_mut().zip(p).for_each(|(a, b)| *a += b);
                oob_sum[i].1 += 1;
            }
        }
        let oob: Vec<Option<Vec<f64>>> = oob_sum
            .into_iter()
            .map(|(v, c)| (c > 0).then(|| v.into_iter().map(|x| x / c as f64).collect()))
            .collect();
        let s: f64 = importance.iter().sum();
        if s > 0.0 {
            importance.iter_mut().for_each(|v| *v /= s);
        }
        Forest {
            trees,
            edges,
            n_out,
            importance,
            classification: y_class.is_some(),
            oob,
        }
    }

    fn predict_binned(&self, binned: &[Vec<u8>], i: usize) -> Vec<f64> {
        let mut acc = vec![0.0; self.n_out.max(1)];
        for t in &self.trees {
            for (a, v) in acc.iter_mut().zip(t.predict(binned, i)) {
                *a += v;
            }
        }
        acc.iter_mut().for_each(|a| *a /= self.trees.len() as f64);
        acc
    }

    /// Predict for row i of a feature matrix (binned with this forest's edges).
    pub fn predict_row(&self, row: &[f64]) -> Vec<f64> {
        let binned: Vec<Vec<u8>> = row
            .iter()
            .enumerate()
            .map(|(j, &x)| vec![self.edges[j].partition_point(|&e| e < x) as u8])
            .collect();
        self.predict_binned(&binned, 0)
    }

    pub fn is_classifier(&self) -> bool {
        self.classification
    }
}

/// Train on a random split and evaluate on the hold-out part.
pub fn train_and_evaluate(
    fm: &FeatureMatrix,
    target: &Target,
    target_name: &str,
    opt: &ForestOptions,
) -> ForestReport {
    let n = fm.n;
    let mut rng = rand::rngs::StdRng::seed_from_u64(opt.seed);
    // stratified hold-out for classification so rare bins are represented
    let strata: Vec<Vec<usize>> = match target {
        Target::Class { y, names } => {
            let mut s = vec![vec![]; names.len()];
            (0..n).for_each(|i| s[y[i]].push(i));
            s
        }
        Target::Reg(_) => vec![(0..n).collect()],
    };
    let (mut test, mut train) = (vec![], vec![]);
    for mut s in strata {
        s.shuffle(&mut rng);
        let k = if s.len() >= 2 {
            (((s.len() as f64) * opt.test_frac).round() as usize).clamp(1, s.len() - 1)
        } else {
            0
        };
        test.extend_from_slice(&s[..k]);
        train.extend_from_slice(&s[k..]);
    }
    let (test, train) = (test.as_slice(), train.as_slice());
    let (edges, binned) = bin_features(fm, opt.n_bins);
    let forest = Forest::fit_binned(edges, &binned, target, train, opt);
    let mut importance: Vec<(String, f64)> = fm
        .names
        .iter()
        .cloned()
        .zip(forest.importance.iter().copied())
        .collect();
    importance.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let mut rep = ForestReport {
        kind: if forest.classification {
            "classification".into()
        } else {
            "regression".into()
        },
        target: target_name.into(),
        n_train: train.len(),
        n_test: test.len(),
        n_features: fm.p,
        class_names: vec![],
        class_counts: vec![],
        accuracy: None,
        balanced_accuracy: None,
        auc: None,
        threshold: None,
        confusion: vec![],
        r2: None,
        rmse: None,
        mae: None,
        importance,
        test_pred: vec![],
    };
    match target {
        Target::Class { y, names } => {
            let k = names.len();
            let mut conf = vec![vec![0usize; k]; k];
            let probs: Vec<Vec<f64>> = test
                .iter()
                .map(|&i| forest.predict_binned(&binned, i))
                .collect();
            let aucs: Vec<f64> = (0..k)
                .filter_map(|c| {
                    let sc: Vec<(f64, bool)> = test
                        .iter()
                        .zip(&probs)
                        .map(|(&i, p)| (p[c], y[i] == c))
                        .collect();
                    auc(&sc)
                })
                .collect();
            rep.auc = if k == 2 {
                aucs.last().copied()
            } else if aucs.is_empty() {
                None
            } else {
                Some(aucs.iter().sum::<f64>() / aucs.len() as f64)
            };
            if k == 2 {
                let oob: Vec<(f64, bool)> = train
                    .iter()
                    .filter_map(|&i| forest.oob[i].as_ref().map(|p| (p[1], y[i] == 1)))
                    .collect();
                rep.threshold = Some(youden_threshold(&oob));
            }
            for (ti, &i) in test.iter().enumerate() {
                let p = &probs[ti];
                let pred = match rep.threshold {
                    Some(th) => (p[1] >= th) as usize,
                    None => (0..k)
                        .max_by(|&a, &b| p[a].partial_cmp(&p[b]).unwrap())
                        .unwrap_or(0),
                };
                conf[y[i]][pred] += 1;
            }
            let correct: usize = (0..k).map(|c| conf[c][c]).sum();
            rep.accuracy = Some(correct as f64 / test.len().max(1) as f64);
            let recalls: Vec<f64> = (0..k)
                .filter_map(|c| {
                    let tot: usize = conf[c].iter().sum();
                    (tot > 0).then(|| conf[c][c] as f64 / tot as f64)
                })
                .collect();
            rep.balanced_accuracy = Some(recalls.iter().sum::<f64>() / recalls.len().max(1) as f64);
            rep.confusion = conf;
            rep.class_names = names.clone();
            let mut cc = vec![0; k];
            y.iter().for_each(|&c| cc[c] += 1);
            rep.class_counts = cc;
        }
        Target::Reg(y) => {
            let preds: Vec<(f64, f64)> = test
                .iter()
                .map(|&i| (y[i], forest.predict_binned(&binned, i)[0]))
                .collect();
            let m = preds.iter().map(|p| p.0).sum::<f64>() / preds.len().max(1) as f64;
            let ss_tot: f64 = preds.iter().map(|p| (p.0 - m).powi(2)).sum();
            let ss_res: f64 = preds.iter().map(|p| (p.0 - p.1).powi(2)).sum();
            rep.r2 = Some(if ss_tot > 0.0 {
                1.0 - ss_res / ss_tot
            } else {
                0.0
            });
            rep.rmse = Some((ss_res / preds.len().max(1) as f64).sqrt());
            rep.mae = Some(
                preds.iter().map(|p| (p.0 - p.1).abs()).sum::<f64>() / preds.len().max(1) as f64,
            );
            rep.test_pred = preds;
        }
    }
    rep
}

/// Threshold maximizing sensitivity + specificity − 1 (falls back to 0.5).
pub fn youden_threshold(scores: &[(f64, bool)]) -> f64 {
    let pos = scores.iter().filter(|s| s.1).count() as f64;
    let neg = scores.len() as f64 - pos;
    if pos == 0.0 || neg == 0.0 {
        return 0.5;
    }
    let mut v = scores.to_vec();
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    let (mut tp, mut fp, mut best, mut th) = (0.0, 0.0, f64::NEG_INFINITY, 0.5);
    let mut i = 0;
    while i < v.len() {
        let cur = v[i].0;
        while i < v.len() && v[i].0 == cur {
            if v[i].1 {
                tp += 1.0
            } else {
                fp += 1.0
            }
            i += 1;
        }
        let j = tp / pos - fp / neg;
        if j > best {
            best = j;
            th = cur;
        }
    }
    th.max(1e-9)
}

/// Area under the ROC curve (Mann–Whitney formulation, ties averaged).
pub fn auc(scores: &[(f64, bool)]) -> Option<f64> {
    let pos = scores.iter().filter(|s| s.1).count();
    let neg = scores.len() - pos;
    if pos == 0 || neg == 0 {
        return None;
    }
    let mut v: Vec<(f64, bool)> = scores.to_vec();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut rank_sum = 0.0;
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1].0 == v[i].0 {
            j += 1;
        }
        let r = (i + j) as f64 / 2.0 + 1.0;
        rank_sum += r * v[i..=j].iter().filter(|x| x.1).count() as f64;
        i = j + 1;
    }
    Some((rank_sum - (pos * (pos + 1)) as f64 / 2.0) / (pos * neg) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn auc_basic() {
        assert_eq!(
            auc(&[(0.1, false), (0.4, false), (0.35, true), (0.8, true)]),
            Some(0.75)
        );
    }
}
