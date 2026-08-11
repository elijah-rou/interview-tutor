use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

static REGISTRATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
static ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static ACTIVE_SIGNAL: AtomicI32 = AtomicI32::new(0);
static ACTIVE_CANCELLED: AtomicBool = AtomicBool::new(false);

extern "C" fn scoped_signal_handler(signal: libc::c_int) {
    if ACTIVE_GENERATION.load(Ordering::Acquire) == 0 {
        return;
    }
    let _ = ACTIVE_SIGNAL.compare_exchange(0, signal, Ordering::AcqRel, Ordering::Acquire);
    ACTIVE_CANCELLED.store(true, Ordering::Release);
}

#[derive(Clone, Debug)]
pub struct SignalState {
    generation: u64,
}

impl SignalState {
    pub fn received(&self) -> Option<i32> {
        if ACTIVE_GENERATION.load(Ordering::Acquire) != self.generation {
            return None;
        }
        match ACTIVE_SIGNAL.load(Ordering::Acquire) {
            0 => None,
            signal => Some(signal),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        ACTIVE_GENERATION.load(Ordering::Acquire) == self.generation
            && ACTIVE_CANCELLED.load(Ordering::Acquire)
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.received().map(signal_exit_code)
    }
}

fn signal_exit_code(signal: i32) -> i32 {
    match signal {
        libc::SIGINT => 130,
        libc::SIGTERM => 143,
        _ => unreachable!("only SIGINT and SIGTERM are scoped"),
    }
}

struct PriorDisposition {
    signal: i32,
    action: libc::sigaction,
}

pub struct ScopedSignalHandlers {
    state: SignalState,
    prior: Vec<PriorDisposition>,
    lock: Option<std::sync::MutexGuard<'static, ()>>,
}

impl ScopedSignalHandlers {
    pub fn register() -> Result<Self, String> {
        Self::register_signals(&[libc::SIGINT, libc::SIGTERM])
    }

    fn register_signals(signals: &[i32]) -> Result<Self, String> {
        let lock = REGISTRATION_LOCK
            .lock()
            .map_err(|_| "execution signal registration lock is poisoned".to_string())?;
        let generation = NEXT_GENERATION.fetch_add(1, Ordering::AcqRel);
        if generation == 0 || generation == u64::MAX {
            return Err("execution signal registration generation exhausted".to_string());
        }
        ACTIVE_SIGNAL.store(0, Ordering::Release);
        ACTIVE_CANCELLED.store(false, Ordering::Release);
        ACTIVE_GENERATION.store(generation, Ordering::Release);
        let mut handlers = Self {
            state: SignalState { generation },
            prior: Vec::with_capacity(2),
            lock: Some(lock),
        };
        for &signal in signals {
            handlers.install(signal)?;
        }
        assert_eq!(handlers.prior.len(), signals.len());
        Ok(handlers)
    }

    fn install(&mut self, signal: i32) -> Result<(), String> {
        // SAFETY: both actions are initialized, the signal numbers are fixed valid values in
        // production, and the installed handler touches only lock-free atomics.
        unsafe {
            let mut prior = std::mem::zeroed();
            if libc::sigaction(signal, std::ptr::null(), &mut prior) != 0 {
                return Err(format!(
                    "cannot inspect signal {signal} disposition: {}",
                    io::Error::last_os_error()
                ));
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = scoped_signal_handler as *const () as usize;
            action.sa_flags = libc::SA_RESTART;
            if libc::sigemptyset(&mut action.sa_mask) != 0 {
                return Err(format!(
                    "cannot initialize signal {signal} handler mask: {}",
                    io::Error::last_os_error()
                ));
            }
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(format!(
                    "cannot install signal {signal} handler: {}",
                    io::Error::last_os_error()
                ));
            }
            self.prior.push(PriorDisposition {
                signal,
                action: prior,
            });
        }
        Ok(())
    }

    pub fn state(&self) -> SignalState {
        self.state.clone()
    }

    pub fn restore(mut self) -> Result<(), String> {
        let result = self.restore_inner();
        self.lock.take();
        result
    }

    fn restore_inner(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        for prior in self.prior.drain(..).rev() {
            // SAFETY: each disposition was initialized by a successful sigaction query for the
            // same valid signal and remains owned until this restoration.
            if unsafe { libc::sigaction(prior.signal, &prior.action, std::ptr::null_mut()) } != 0 {
                errors.push(format!(
                    "cannot restore signal {} disposition: {}",
                    prior.signal,
                    io::Error::last_os_error()
                ));
            }
        }
        ACTIVE_GENERATION.store(0, Ordering::Release);
        ACTIVE_CANCELLED.store(false, Ordering::Release);
        ACTIVE_SIGNAL.store(0, Ordering::Release);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for ScopedSignalHandlers {
    fn drop(&mut self) {
        let _ = self.restore_inner();
    }
}

pub struct BlockedExecutionSignals {
    prior_mask: libc::sigset_t,
    active: bool,
}

impl BlockedExecutionSignals {
    pub fn block() -> Result<Self, String> {
        // SAFETY: all signal sets are initialized before use and pthread_sigmask changes only the
        // calling thread's mask.
        unsafe {
            let mut signals = std::mem::zeroed();
            if libc::sigemptyset(&mut signals) != 0 {
                return Err(format!(
                    "cannot initialize execution signal set: {}",
                    io::Error::last_os_error()
                ));
            }
            for signal in [libc::SIGINT, libc::SIGTERM] {
                if libc::sigaddset(&mut signals, signal) != 0 {
                    return Err(format!(
                        "cannot add execution signal to set: {}",
                        io::Error::last_os_error()
                    ));
                }
            }
            let mut prior_mask = std::mem::zeroed();
            let error = libc::pthread_sigmask(libc::SIG_BLOCK, &signals, &mut prior_mask);
            if error != 0 {
                return Err(format!(
                    "cannot block execution signals: {}",
                    io::Error::from_raw_os_error(error)
                ));
            }
            Ok(Self {
                prior_mask,
                active: true,
            })
        }
    }

    pub fn pending_exit_code(&self) -> Result<Option<i32>, String> {
        // SAFETY: pending is initialized by sigpending before sigismember reads it.
        unsafe {
            let mut pending = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return Err(format!(
                    "cannot inspect pending execution signals: {}",
                    io::Error::last_os_error()
                ));
            }
            for (signal, exit_code) in [(libc::SIGINT, 130), (libc::SIGTERM, 143)] {
                match libc::sigismember(&pending, signal) {
                    1 => return Ok(Some(exit_code)),
                    0 => {}
                    _ => {
                        return Err(format!(
                            "cannot inspect pending execution signal: {}",
                            io::Error::last_os_error()
                        ));
                    }
                }
            }
            Ok(None)
        }
    }

    pub fn consume_pending(&self) -> Result<(), String> {
        // SAFETY: pending and wait_set are initialized before use. sigwait is called only for a
        // signal proven pending and blocked in this thread.
        unsafe {
            let mut pending = std::mem::zeroed();
            if libc::sigpending(&mut pending) != 0 {
                return Err(format!(
                    "cannot inspect pending execution signals for cleanup: {}",
                    io::Error::last_os_error()
                ));
            }
            for signal in [libc::SIGINT, libc::SIGTERM] {
                match libc::sigismember(&pending, signal) {
                    0 => continue,
                    1 => {}
                    _ => {
                        return Err(format!(
                            "cannot inspect pending execution signal for cleanup: {}",
                            io::Error::last_os_error()
                        ));
                    }
                }
                let mut wait_set = std::mem::zeroed();
                if libc::sigemptyset(&mut wait_set) != 0
                    || libc::sigaddset(&mut wait_set, signal) != 0
                {
                    return Err(format!(
                        "cannot initialize pending execution signal cleanup: {}",
                        io::Error::last_os_error()
                    ));
                }
                let mut received = 0;
                let error = libc::sigwait(&wait_set, &mut received);
                if error != 0 {
                    return Err(format!(
                        "cannot consume pending execution signal: {}",
                        io::Error::from_raw_os_error(error)
                    ));
                }
                if received != signal {
                    return Err(format!(
                        "consumed unexpected execution signal {received}; expected {signal}"
                    ));
                }
            }
            Ok(())
        }
    }

    pub fn restore(mut self) -> Result<(), String> {
        // SAFETY: prior_mask was initialized by the successful pthread_sigmask call in block.
        let error = unsafe {
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.prior_mask, std::ptr::null_mut())
        };
        if error != 0 {
            return Err(format!(
                "cannot restore execution signal mask: {}",
                io::Error::from_raw_os_error(error)
            ));
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for BlockedExecutionSignals {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: prior_mask was initialized by the successful pthread_sigmask call in block.
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, &self.prior_mask, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(debug_assertions)]
pub mod test_support {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    extern "C" fn custom_sigint(_signal: libc::c_int) {}
    extern "C" fn custom_sigterm(_signal: libc::c_int) {}

    pub struct DispositionProbe {
        path: PathBuf,
        expected: Vec<PriorDisposition>,
        original: Vec<PriorDisposition>,
        mask: libc::sigset_t,
    }

    impl DispositionProbe {
        pub fn from_environment() -> Result<Option<Self>, String> {
            let Some(path) = std::env::var_os("INTERVIEW_TUTOR_TEST_SIGNAL_DISPOSITION_FILE")
            else {
                return Ok(None);
            };
            Self::install(PathBuf::from(path)).map(Some)
        }

        #[cfg(test)]
        pub fn for_test(path: PathBuf) -> Result<Self, String> {
            Self::install(path)
        }

        fn install(path: PathBuf) -> Result<Self, String> {
            let mut probe = Self {
                path,
                expected: Vec::with_capacity(2),
                original: Vec::with_capacity(2),
                // SAFETY: the mask is initialized below before it is read.
                mask: unsafe { std::mem::zeroed() },
            };
            // SAFETY: actions and masks are initialized before use. The custom handlers return
            // without touching process state and are installed only for this debug test hook.
            unsafe {
                let error =
                    libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut probe.mask);
                if error != 0 {
                    return Err(format!(
                        "cannot capture test signal mask: {}",
                        io::Error::from_raw_os_error(error)
                    ));
                }
                for (signal, handler) in [
                    (libc::SIGINT, custom_sigint as *const () as usize),
                    (libc::SIGTERM, custom_sigterm as *const () as usize),
                ] {
                    let mut original = std::mem::zeroed();
                    if libc::sigaction(signal, std::ptr::null(), &mut original) != 0 {
                        return Err(format!(
                            "cannot capture test signal disposition: {}",
                            io::Error::last_os_error()
                        ));
                    }
                    probe.original.push(PriorDisposition {
                        signal,
                        action: original,
                    });
                    let mut custom: libc::sigaction = std::mem::zeroed();
                    custom.sa_sigaction = handler;
                    custom.sa_flags = libc::SA_RESTART;
                    libc::sigemptyset(&mut custom.sa_mask);
                    libc::sigaddset(&mut custom.sa_mask, libc::SIGUSR1);
                    if libc::sigaction(signal, &custom, std::ptr::null_mut()) != 0 {
                        return Err(format!(
                            "cannot install test signal disposition: {}",
                            io::Error::last_os_error()
                        ));
                    }
                    let mut expected = std::mem::zeroed();
                    if libc::sigaction(signal, std::ptr::null(), &mut expected) != 0 {
                        return Err(format!(
                            "cannot query test signal disposition: {}",
                            io::Error::last_os_error()
                        ));
                    }
                    probe.expected.push(PriorDisposition {
                        signal,
                        action: expected,
                    });
                }
            }
            Ok(probe)
        }

        pub fn verify(mut self) -> Result<(), String> {
            let result = self.verify_inner();
            self.restore_original();
            result?;
            fs::write(&self.path, "dispositions=restored mask=restored\n")
                .map_err(|error| format!("cannot write signal disposition probe: {error}"))
        }

        fn verify_inner(&self) -> Result<(), String> {
            for expected in &self.expected {
                // SAFETY: actual is initialized by sigaction before comparison.
                unsafe {
                    let mut actual = std::mem::zeroed();
                    if libc::sigaction(expected.signal, std::ptr::null(), &mut actual) != 0 {
                        return Err(format!(
                            "cannot query restored signal disposition: {}",
                            io::Error::last_os_error()
                        ));
                    }
                    if !same_disposition(&actual, &expected.action) {
                        return Err(format!(
                            "signal {} disposition was not restored exactly",
                            expected.signal
                        ));
                    }
                }
            }
            // SAFETY: current is initialized before same_mask reads it.
            unsafe {
                let mut current = std::mem::zeroed();
                let error =
                    libc::pthread_sigmask(libc::SIG_SETMASK, std::ptr::null(), &mut current);
                if error != 0 {
                    return Err(format!(
                        "cannot query restored signal mask: {}",
                        io::Error::from_raw_os_error(error)
                    ));
                }
                if !same_mask(&current, &self.mask) {
                    return Err("execution signal mask was not restored exactly".to_string());
                }
            }
            Ok(())
        }

        fn restore_original(&mut self) {
            for original in self.original.drain(..).rev() {
                // SAFETY: the original action came from sigaction for this signal.
                unsafe {
                    libc::sigaction(original.signal, &original.action, std::ptr::null_mut());
                }
            }
        }
    }

    impl Drop for DispositionProbe {
        fn drop(&mut self) {
            self.restore_original();
        }
    }

    unsafe fn same_disposition(left: &libc::sigaction, right: &libc::sigaction) -> bool {
        left.sa_sigaction == right.sa_sigaction
            && left.sa_flags == right.sa_flags
            && unsafe { same_mask(&left.sa_mask, &right.sa_mask) }
    }

    unsafe fn same_mask(left: &libc::sigset_t, right: &libc::sigset_t) -> bool {
        (1..=64).all(|signal| unsafe {
            libc::sigismember(left, signal) == libc::sigismember(right, signal)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SIGNAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn scoped_run_restores_exact_custom_dispositions_without_delivering_a_signal() {
        let _lock = TEST_SIGNAL_LOCK.lock().unwrap();
        let probe_path =
            std::env::temp_dir().join(format!("interview-signal-probe-{}", std::process::id()));
        // The debug probe installs non-default handlers and captures their full queried actions.
        let probe = test_support::DispositionProbe::for_test(probe_path.clone()).unwrap();
        {
            let handlers = ScopedSignalHandlers::register().unwrap();
            assert_eq!(handlers.state().received(), None);
            handlers.restore().unwrap();
        }
        probe.verify().unwrap();
        assert_eq!(
            std::fs::read_to_string(&probe_path).unwrap(),
            "dispositions=restored mask=restored\n"
        );
        std::fs::remove_file(&probe_path).unwrap();
    }

    #[test]
    fn partial_registration_failure_restores_already_replaced_dispositions() {
        let _lock = TEST_SIGNAL_LOCK.lock().unwrap();
        let probe_path = std::env::temp_dir().join(format!(
            "interview-signal-partial-probe-{}",
            std::process::id()
        ));
        let probe = test_support::DispositionProbe::for_test(probe_path.clone()).unwrap();
        let error = ScopedSignalHandlers::register_signals(&[libc::SIGINT, -1])
            .err()
            .expect("invalid second signal must fail registration");
        assert!(error.contains("cannot inspect signal -1 disposition"));
        probe.verify().unwrap();
        std::fs::remove_file(probe_path).unwrap();
    }
}
