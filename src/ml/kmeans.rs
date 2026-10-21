//! k-means++ clustering with silhouette-based k selection.

use rand::{Rng, SeedableRng};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct KMeans {
    pub k: usize,
    pub centroids: Vec<Vec<f64>>,
    pub labels: Vec<usize>,
    pub sizes: Vec<usize>,
    pub inertia: f64,
    pub silhouette: f64,
    /// (k, silhouette) evaluated during auto-selection
    pub k_scan: Vec<(usize, f64)>,
}

fn d2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

pub fn fit(data: &[Vec<f64>], k: usize, seed: u64, max_iter: usize) -> KMeans {
    let n = data.len();
    let k = k.min(n).max(1);
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    // k-means++ init
    let mut cents: Vec<Vec<f64>> = vec![data[rng.gen_range(0..n)].clone()];
    let mut dist: Vec<f64> = data.par_iter().map(|x| d2(x, &cents[0])).collect();
    while cents.len() < k {
        let total: f64 = dist.iter().sum();
        let next = if total > 0.0 {
            let mut t = rng.gen::<f64>() * total;
            let mut idx = n - 1;
            for (i, d) in dist.iter().enumerate() {
                t -= d;
                if t <= 0.0 {
                    idx = i;
                    break;
                }
            }
            idx
        } else {
            rng.gen_range(0..n)
        };
        cents.push(data[next].clone());
        let c = cents.last().unwrap().clone();
        dist.par_iter_mut()
            .zip(data.par_iter())
            .for_each(|(d, x)| *d = d.min(d2(x, &c)));
    }
    let dim = data[0].len();
    let mut labels = vec![0usize; n];
    for _ in 0..max_iter {
        let new: Vec<usize> = data
            .par_iter()
            .map(|x| {
                (0..k)
                    .min_by(|&a, &b| d2(x, &cents[a]).partial_cmp(&d2(x, &cents[b])).unwrap())
                    .unwrap()
            })
            .collect();
        let changed = new != labels;
        labels = new;
        let mut sums = vec![vec![0.0; dim]; k];
        let mut cnt = vec![0usize; k];
        for (x, &l) in data.iter().zip(&labels) {
            cnt[l] += 1;
            for (s, v) in sums[l].iter_mut().zip(x) {
                *s += v;
            }
        }
        for c in 0..k {
            if cnt[c] > 0 {
                cents[c] = sums[c].iter().map(|s| s / cnt[c] as f64).collect();
            }
        }
        if !changed {
            break;
        }
    }
    let inertia: f64 = data
        .iter()
        .zip(&labels)
        .map(|(x, &l)| d2(x, &cents[l]))
        .sum();
    let mut sizes = vec![0; k];
    labels.iter().for_each(|&l| sizes[l] += 1);
    let sil = silhouette(data, &labels, k, 2000, seed);
    KMeans {
        k,
        centroids: cents,
        labels,
        sizes,
        inertia,
        silhouette: sil,
        k_scan: vec![],
    }
}

/// Mean silhouette over a random sample of points.
pub fn silhouette(data: &[Vec<f64>], labels: &[usize], k: usize, sample: usize, seed: u64) -> f64 {
    let n = data.len();
    if k < 2 || n < 3 {
        return 0.0;
    }
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed ^ 0x5EED);
    let idx: Vec<usize> = if n <= sample {
        (0..n).collect()
    } else {
        (0..sample).map(|_| rng.gen_range(0..n)).collect()
    };
    let ref_idx = idx.clone();
    let vals: Vec<f64> = idx
        .par_iter()
        .map(|&i| {
            let mut sum = vec![0.0; k];
            let mut cnt = vec![0usize; k];
            for &j in &ref_idx {
                if i == j {
                    continue;
                }
                sum[labels[j]] += d2(&data[i], &data[j]).sqrt();
                cnt[labels[j]] += 1;
            }
            let li = labels[i];
            if cnt[li] == 0 {
                return 0.0;
            }
            let a = sum[li] / cnt[li] as f64;
            let b = (0..k)
                .filter(|&c| c != li && cnt[c] > 0)
                .map(|c| sum[c] / cnt[c] as f64)
                .fold(f64::INFINITY, f64::min);
            if !b.is_finite() {
                return 0.0;
            }
            (b - a) / a.max(b)
        })
        .collect();
    vals.iter().sum::<f64>() / vals.len() as f64
}

/// Try k in [2, k_max] and keep the best silhouette.
pub fn auto(data: &[Vec<f64>], k_max: usize, seed: u64) -> KMeans {
    let mut best: Option<KMeans> = None;
    let mut scan = vec![];
    for k in 2..=k_max.max(2) {
        let m = fit(data, k, seed, 100);
        scan.push((k, m.silhouette));
        if best
            .as_ref()
            .map(|b| m.silhouette > b.silhouette)
            .unwrap_or(true)
        {
            best = Some(m);
        }
    }
    let mut b = best.unwrap();
    b.k_scan = scan;
    b
}
