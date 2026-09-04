//! Crash-loop protection for Desktop-owned managed-agent harnesses.
//!
//! The ACP circuit breaker supervises adapter children inside a live harness.
//! This guard sits one process boundary higher and protects automatic Desktop
//! starts when the harness itself exits rapidly.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

use super::ManagedAgentRuntimeKey;

const RAPID_EXIT_WINDOW: Duration = Duration::from_secs(5);
const RAPID_EXIT_LIMIT: u32 = 5;

#[derive(Debug, Default)]
struct CrashWatch {
    automatic_started_at: Option<Instant>,
    consecutive_rapid_exits: u32,
}

#[derive(Debug, Default)]
struct OuterHarnessGuard {
    watches: Mutex<HashMap<ManagedAgentRuntimeKey, CrashWatch>>,
}

impl OuterHarnessGuard {
    fn watches(&self) -> MutexGuard<'_, HashMap<ManagedAgentRuntimeKey, CrashWatch>> {
        self.watches
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn automatic_start<T>(
        &self,
        key: &ManagedAgentRuntimeKey,
        start: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.ensure_automatic_start_allowed(key)?;
        let started = start()?;
        self.watches()
            .entry(key.clone())
            .or_default()
            .automatic_started_at = Some(Instant::now());
        Ok(started)
    }

    fn ensure_automatic_start_allowed(&self, key: &ManagedAgentRuntimeKey) -> Result<(), String> {
        if self.is_blocked(key) {
            Err(blocked_error(key))
        } else {
            Ok(())
        }
    }

    fn note_exit_at(&self, key: &ManagedAgentRuntimeKey, now: Instant) -> bool {
        let mut watches = self.watches();
        let Some(watch) = watches.get_mut(key) else {
            return false;
        };
        let Some(started_at) = watch.automatic_started_at.take() else {
            return watch.consecutive_rapid_exits >= RAPID_EXIT_LIMIT;
        };
        if now.saturating_duration_since(started_at) <= RAPID_EXIT_WINDOW {
            watch.consecutive_rapid_exits = watch.consecutive_rapid_exits.saturating_add(1);
        } else {
            watches.remove(key);
            return false;
        }
        watch.consecutive_rapid_exits >= RAPID_EXIT_LIMIT
    }

    fn is_blocked(&self, key: &ManagedAgentRuntimeKey) -> bool {
        self.watches()
            .get(key)
            .map(|watch| watch.consecutive_rapid_exits >= RAPID_EXIT_LIMIT)
            .unwrap_or(false)
    }

    fn clear(&self, key: &ManagedAgentRuntimeKey) {
        self.watches().remove(key);
    }
}

fn guard() -> &'static OuterHarnessGuard {
    static GUARD: OnceLock<OuterHarnessGuard> = OnceLock::new();
    GUARD.get_or_init(OuterHarnessGuard::default)
}

pub(crate) fn automatic_start<T>(
    key: &ManagedAgentRuntimeKey,
    start: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    guard().automatic_start(key, start)
}

/// Record a process exit observed through normal lifecycle polling.
///
/// Intentional stop and coordinated shutdown remove tracked runtimes before
/// polling, so they never reach this function and never count as crashes.
pub(crate) fn note_exit(key: &ManagedAgentRuntimeKey) -> bool {
    guard().note_exit_at(key, Instant::now())
}

pub(crate) fn clear(key: &ManagedAgentRuntimeKey) {
    guard().clear(key);
}

pub(crate) fn blocked_error(key: &ManagedAgentRuntimeKey) -> String {
    format!(
        "automatic start blocked after {RAPID_EXIT_LIMIT} harness exits within {RAPID_EXIT_WINDOW:?} for {} on {}; use Start to retry",
        key.pubkey, key.relay_url
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key(pubkey_byte: &str, relay: &str) -> ManagedAgentRuntimeKey {
        ManagedAgentRuntimeKey::new(pubkey_byte.repeat(64), relay).unwrap()
    }

    fn rapid_exit(guard: &OuterHarnessGuard, key: &ManagedAgentRuntimeKey, starts: &AtomicUsize) {
        guard
            .automatic_start(key, || {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        let started_at = guard
            .watches()
            .get(key)
            .unwrap()
            .automatic_started_at
            .unwrap();
        guard.note_exit_at(key, started_at + RAPID_EXIT_WINDOW);
    }

    #[test]
    fn repeated_rapid_automatic_restarts_trip_guard_at_call_site() {
        let guard = OuterHarnessGuard::default();
        let key = key("a", "wss://relay.example");
        let starts = AtomicUsize::new(0);

        for _ in 0..RAPID_EXIT_LIMIT {
            rapid_exit(&guard, &key, &starts);
        }

        let error = guard
            .automatic_start(&key, || {
                starts.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err();
        assert_eq!(starts.load(Ordering::SeqCst), RAPID_EXIT_LIMIT as usize);
        assert!(error.contains("automatic start blocked"), "{error}");
    }

    #[test]
    fn stable_run_clears_rapid_exit_sequence() {
        let guard = OuterHarnessGuard::default();
        let key = key("b", "wss://relay.example");
        let starts = AtomicUsize::new(0);
        for _ in 0..(RAPID_EXIT_LIMIT - 1) {
            rapid_exit(&guard, &key, &starts);
        }

        guard.automatic_start(&key, || Ok(())).unwrap();
        let started_at = guard
            .watches()
            .get(&key)
            .unwrap()
            .automatic_started_at
            .unwrap();
        assert!(!guard.note_exit_at(
            &key,
            started_at + RAPID_EXIT_WINDOW + Duration::from_millis(1)
        ));
        assert!(!guard.is_blocked(&key));
    }

    #[test]
    fn manual_start_clears_only_the_selected_runtime_pair() {
        let guard = OuterHarnessGuard::default();
        let first = key("c", "wss://one.example");
        let second = key("c", "wss://two.example");
        let starts = AtomicUsize::new(0);
        for _ in 0..RAPID_EXIT_LIMIT {
            rapid_exit(&guard, &first, &starts);
            rapid_exit(&guard, &second, &starts);
        }
        assert!(guard.is_blocked(&first));
        assert!(guard.is_blocked(&second));

        guard.clear(&first);

        assert!(!guard.is_blocked(&first));
        assert!(guard.is_blocked(&second));
    }

    #[test]
    fn exit_without_automatic_start_does_not_count() {
        let guard = OuterHarnessGuard::default();
        let key = key("d", "wss://relay.example");
        for _ in 0..(RAPID_EXIT_LIMIT * 2) {
            assert!(!guard.note_exit_at(&key, Instant::now()));
        }
        assert!(!guard.is_blocked(&key));
    }

    #[test]
    fn failed_spawn_is_not_counted_as_a_started_harness() {
        let guard = OuterHarnessGuard::default();
        let key = key("e", "wss://relay.example");
        assert!(guard
            .automatic_start::<()>(&key, || Err("spawn failed".into()))
            .is_err());
        assert!(!guard.note_exit_at(&key, Instant::now()));
        assert!(!guard.is_blocked(&key));
    }
}
