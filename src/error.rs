/// Internal classification of execution errors.
///
/// - [`ExecError::Step`] means a user-provided step body returned `Err`.
///   These surface through the public [`RunSummary`](crate::RunSummary) as
///   `success == false` + `error == Some(...)`.
/// - [`ExecError::Framework`] means the harness itself failed: a hook
///   returned `Err`, a step had no function attached, or a policy/placement
///   invariant was violated (e.g. workers > 1 on a mutable step,
///   mutable step inside `Parallel`). These propagate out of
///   `Scenario::run` as `Err` because they indicate bugs in the test code
///   or in Undine itself rather than observed test outcomes.
#[derive(Debug)]
pub(crate) enum ExecError {
    /// User step bodies failed. Concurrent workers and `Parallel` children
    /// all continue running on failure so every error is observed; each
    /// error is logged at the moment it's detected, and this variant
    /// carries only the count so the run summary stays lightweight.
    Step(u64),
    Framework(anyhow::Error),
}
