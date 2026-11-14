//! Deterministic data parallelism on std threads (no dependencies, no unsafe).
//!
//! `par_map(n, threads, f)` evaluates `f(i)` for i in 0..n on a pool of scoped
//! threads and returns the results IN INDEX ORDER, so output never depends on
//! scheduling. On targets without threads (wasm32) it degrades to a plain loop.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub fn available_threads() -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

pub fn par_map<T, F>(n: usize, threads: usize, f: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Sync,
{
    let threads = threads.max(1).min(n.max(1));
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        return (0..n).map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::with_capacity(n));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut local: Vec<(usize, T)> = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    local.push((i, f(i)));
                    if local.len() >= 8 {
                        out.lock().unwrap().append(&mut local);
                    }
                }
                out.lock().unwrap().append(&mut local);
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, t)| t).collect()
}

/// Run `f` over disjoint mutable row-chunks of `data` in parallel.
/// `chunk` is the chunk length in elements; `f(chunk_index, slice)`.
pub fn par_chunks_mut<T, F>(data: &mut [T], chunk: usize, threads: usize, f: F)
where
    T: Send,
    F: Fn(usize, &mut [T]) + Sync,
{
    let chunk = chunk.max(1);
    let threads = threads.max(1);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        for (i, c) in data.chunks_mut(chunk).enumerate() {
            f(i, c);
        }
        return;
    }
    let queue: Mutex<std::iter::Enumerate<std::slice::ChunksMut<'_, T>>> = Mutex::new(data.chunks_mut(chunk).enumerate());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let item = queue.lock().unwrap().next();
                match item {
                    Some((i, c)) => f(i, c),
                    None => break,
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_results() {
        let v = par_map(1000, 4, |i| i * i);
        assert_eq!(v.len(), 1000);
        assert!(v.iter().enumerate().all(|(i, x)| *x == i * i));
    }

    #[test]
    fn chunks() {
        let mut data = vec![0u32; 1000];
        par_chunks_mut(&mut data, 64, 4, |i, c| c.iter_mut().for_each(|x| *x = i as u32));
        assert!(data.iter().enumerate().all(|(k, x)| *x == (k / 64) as u32));
    }
}
