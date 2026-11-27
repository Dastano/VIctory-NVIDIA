//! The end of a landing ([`Host::relay_land`]), the part after title memory
//! is complete: less of it before the title is told, and less of it on the
//! thread that lands flights at all.
//!
//! In the seconds under 29.5 fps of the boat drive (run r99) that thread was
//! 10.2 ms a frame landing (6.2 in those over 38): fetching 2.2, merging 1.5,
//! "read-backs and lists" 3.2 (of it the texture read-backs 1.0) and "the
//! rest" 2.5 (1.2 over 38). "The rest" is [`bury`]: the objects retired
//! `bury_lag` submissions ago, some 20,000 a frame of them resources slots showed,
//! released one by one; while it runs the next landing waits, and with it
//! the line's waiters (`relay_wait`). "Lists" is each flight's tidying: the
//! compare's lists given back (`card_done`), the list of the steps done on
//! the card and its host copies let go, its dynamic descriptor slots given
//! back. It stood between a flight's read-backs and the fence signals behind
//! it.
//!
//! `VICTORY_RELAY2_BURY_ASIDE` (on by default; `=0`: on this thread, as
//! before): what is due is taken out of the list under its lock here, as
//! before, and released on a thread of its own ([`drop_aside`]). Nothing is
//! released sooner than before, only later; nothing of it is title memory.
//!
//! `VICTORY_RELAY2_TELL_FIRST` (on by default; `=0`: flight by flight, as
//! before): the title is told everything of the landing (read-backs,
//! bottom-of-pipe writes, fence signals, in the line's order, as before)
//! before the flights are tidied, in their order ([`tell_then_tidy`]). The
//! tidying writes no title memory and none of it is read by the telling: what
//! the title sees, and when it sees it relative to the rest of what it sees,
//! is the same.

use super::*;

/// See the file.
pub(super) fn bury_aside() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_RELAY2_BURY_ASIDE").is_none_or(|v| v != "0"))
}

/// See the file.
pub(super) fn tell_first() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_RELAY2_TELL_FIRST").is_none_or(|v| v != "0"))
}

type Dropped = Box<dyn Send>;

/// `what` is dropped on a thread of its own, after what was given it
/// before; here and now if there is no such thread.
pub(super) fn drop_aside(what: impl Send + 'static) {
    static THREAD: OnceLock<Option<Mutex<std::sync::mpsc::Sender<Dropped>>>> = OnceLock::new();
    let thread = THREAD.get_or_init(|| {
        let (send, take) = std::sync::mpsc::channel::<Dropped>();
        let made = std::thread::Builder::new().name("victory-bury".into()).spawn(move || {
            for what in take {
                let _dropping = crate::timeline::span("beside: retired objects released");
                drop(what);
            }
        });
        match made {
            Ok(_) => Some(Mutex::new(send)),
            Err(e) => {
                tracing::warn!(error = %e, "no thread to release retired objects on: the thread that lands flights does (VICTORY_RELAY2_BURY_ASIDE)");
                None
            }
        }
    });
    let what: Dropped = Box::new(what);
    let Some(send) = thread else {
        drop(what);
        return;
    };
    // (A thread that is gone hands it back: dropped here.)
    if let Err(back) = send.lock().unwrap_or_else(|e| e.into_inner()).send(what) {
        drop(back.0);
    }
}

/// `items` in their order: each is told (`tell`), and what is left of it to
/// tidy is tidied (`tidy`) right after it, or with `first` after all of
/// them are told, in the same order.
pub(super) fn tell_then_tidy<T, U>(
    items: impl IntoIterator<Item = T>,
    first: bool,
    mut tell: impl FnMut(T) -> Option<U>,
    mut tidy: impl FnMut(U),
) {
    let mut held = Vec::new();
    for item in items {
        let Some(left) = tell(item) else {
            continue;
        };
        if first {
            held.push(left);
        } else {
            tidy(left);
        }
    }
    held.into_iter().for_each(&mut tidy);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a landing tells the title, as the title sees it: a flight's
    /// read-back writes bytes, a bottom-of-pipe write a dword, a signal
    /// lets the title look (it is given title memory as it is then).
    #[derive(Clone)]
    enum Item {
        Flight { at: usize, bytes: Vec<u8>, number: u64 },
        Write { at: usize, value: u32 },
        Signal(u64),
    }

    /// Title memory at each signal, title memory at the end, the flights
    /// tidied in their order.
    fn land(items: &[Item], first: bool) -> (Vec<(u64, Vec<u8>)>, Vec<u8>, Vec<u64>) {
        let title = std::cell::RefCell::new(vec![0u8; 256]);
        let mut seen = Vec::new();
        let mut tidied = Vec::new();
        tell_then_tidy(
            items.iter().cloned(),
            first,
            |item| match item {
                Item::Flight { at, bytes, number } => {
                    title.borrow_mut()[at..at + bytes.len()].copy_from_slice(&bytes);
                    Some(number)
                }
                Item::Write { at, value } => {
                    title.borrow_mut()[at..at + 4].copy_from_slice(&value.to_le_bytes());
                    None
                }
                Item::Signal(value) => {
                    seen.push((value, title.borrow().clone()));
                    None
                }
            },
            |number| {
                // Tidying touches no title memory; a check that it comes
                // after the flight was told.
                assert!(title.borrow().iter().any(|&b| b != 0));
                tidied.push(number);
            },
        );
        (seen, title.into_inner(), tidied)
    }

    /// The title sees the same bytes at every signal and at the end, with
    /// the flights tidied before the next is told or after all are; the
    /// flights are tidied in the same order either way.
    #[test]
    fn telling_first_lands_the_same_bytes() {
        let items = vec![
            Item::Flight { at: 0, bytes: vec![1; 64], number: 7 },
            Item::Write { at: 60, value: 0xdead_beef },
            Item::Signal(1),
            Item::Flight { at: 32, bytes: vec![2; 64], number: 8 },
            Item::Signal(2),
            Item::Write { at: 128, value: 5 },
            Item::Flight { at: 120, bytes: vec![3; 16], number: 9 },
            Item::Signal(3),
        ];
        let before = land(&items, false);
        let now = land(&items, true);
        assert_eq!(before.0, now.0);
        assert_eq!(before.1, now.1);
        assert_eq!(before.2, now.2);
        assert_eq!(now.2, vec![7, 8, 9]);
        assert_eq!(now.0.len(), 3);
        assert_eq!(&now.1[60..64], &[2, 2, 2, 2], "the second flight's read-back stands over the write before it");
    }

    /// What is given aside is dropped, on another thread, in the order it
    /// was given.
    #[test]
    fn dropped_aside_in_order() {
        struct Noted(u32, std::sync::Arc<Mutex<Vec<(u32, std::thread::ThreadId)>>>);
        impl Drop for Noted {
            fn drop(&mut self) {
                self.1.lock().unwrap().push((self.0, std::thread::current().id()));
            }
        }
        let notes = std::sync::Arc::new(Mutex::new(Vec::new()));
        for i in 0..4 {
            drop_aside(vec![Noted(i, notes.clone())]);
        }
        let began = std::time::Instant::now();
        while notes.lock().unwrap().len() < 4 {
            assert!(began.elapsed() < std::time::Duration::from_secs(10), "never dropped");
            std::thread::yield_now();
        }
        let notes = notes.lock().unwrap();
        assert_eq!(notes.iter().map(|n| n.0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(notes.iter().all(|n| n.1 != std::thread::current().id()));
    }
}
