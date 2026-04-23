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
    /// One or more user step bodies failed. Within a concurrent block
    /// (workers, `Parallel`) we collect every failure rather than stopping
    /// at the first, so a run summary can report a complete picture.
    Step(Vec<anyhow::Error>),
    Framework(anyhow::Error),
}
