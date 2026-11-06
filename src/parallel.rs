//! Bounded parallel map over git probes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

/// At most this many git processes run at once, as in the 1.x CLI.
pub const PARALLEL_CAP: usize = 16;

/// Apply `f` to every item on up to [`PARALLEL_CAP`] threads and return the
/// results in input order. 0 or 1 items run inline.
///
/// Each probe blocks on a child git process, so plain threads give real
/// concurrency without an async runtime.
pub fn map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    if items.len() <= 1 {
        return items.iter().map(&f).collect();
    }
    let workers = items.len().min(PARALLEL_CAP);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() {
                    break;
                }
                let result = f(&items[i]);
                *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every slot is filled once the scope joins")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn preserves_input_order() {
        let items: Vec<u64> = (0..50).collect();
        let out = map(&items, |&n| {
            // Later items finish first.
            std::thread::sleep(std::time::Duration::from_millis(50 - n));
            n * 2
        });
        assert_eq!(out, items.iter().map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn empty_and_single() {
        let empty: Vec<u8> = vec![];
        assert!(map(&empty, |x| *x).is_empty());
        assert_eq!(map(&[7u8], |x| *x + 1), vec![8]);
    }

    #[test]
    fn never_exceeds_cap() {
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items: Vec<u32> = (0..64).collect();
        map(&items, |_| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(5));
            running.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(peak.load(Ordering::SeqCst) <= PARALLEL_CAP);
    }
}
