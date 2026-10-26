//! The runtime every exported async method runs its body on.
//!
//! UniFFI polls an exported future on whatever thread the foreign caller
//! awaits it from. On iOS that is a Swift cooperative-pool thread with a
//! 512 KiB stack, and an unoptimised build of a send preview — nonce, fee and
//! balance reads nested inside one another — needs more than that on its
//! first poll: building a transaction crashed with a stack-guard fault before
//! the first request left the device.
//!
//! So an exported method does not run its body where it is polled. It hands
//! the body to [`run`], which spawns it on this runtime's workers, whose
//! stacks are sized here, and the foreign thread only ever polls the small
//! join future. How much stack core needs is then core's to decide, in one
//! place, rather than whatever the calling platform happens to give it.
use std::future::Future;
use std::sync::LazyLock;

/// Generous on purpose: an unoptimised build nests far deeper than a release
/// build, and untouched stack pages cost address space, not memory.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .thread_name("spectra-core")
        .thread_stack_size(WORKER_STACK_BYTES)
        .enable_all()
        .build()
        .expect("core cannot run without its worker runtime")
});

/// Run `body` on a core worker and wait for its result.
///
/// Dropping the returned future aborts the body at its next await, so a
/// caller that cancels still cancels the work, exactly as when the body was
/// polled in place. A panic in the body resumes in the caller.
pub(crate) async fn run<T, F>(body: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut task = AbortOnDrop(RUNTIME.spawn(body));
    match (&mut task.0).await {
        Ok(value) => value,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        // Only `AbortOnDrop` aborts the task, and it cannot while this
        // future is still being polled.
        Err(error) => unreachable!("core worker task ended without finishing: {error}"),
    }
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_on_a_core_worker_with_its_stack() {
        let name = run(async { std::thread::current().name().map(str::to_owned) }).await;
        assert_eq!(name.as_deref(), Some("spectra-core"));
    }

    #[tokio::test]
    async fn dropping_the_caller_aborts_the_body() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let pending = run(async move {
            let _keep = tx;
            std::future::pending::<()>().await;
        });
        drop(tokio::time::timeout(std::time::Duration::from_millis(10), pending).await);
        // The body owned the sender; aborting it drops the sender.
        assert!(rx.await.is_err());
    }

    #[tokio::test]
    #[should_panic(expected = "boom")]
    async fn a_panic_in_the_body_reaches_the_caller() {
        run(async { panic!("boom") }).await
    }
}
