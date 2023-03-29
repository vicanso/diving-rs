// task local log
#[macro_export]
macro_rules! tl_info {
    ($($arg:tt)*) => (
        let trace_id = TRACE_ID.with(clone_value_from_task_local);
        info!(
            traceId = trace_id,
            $($arg)*
        )
    );
}

#[macro_export]
macro_rules! tl_error {
    ($($arg:tt)*) => (
        let trace_id = TRACE_ID.with(clone_value_from_task_local);
        error!(
            traceId = trace_id,
            $($arg)*
        )
    );
}

#[macro_export]
macro_rules! tl_warn {
    ($($arg:tt)*) => (
        let trace_id = TRACE_ID.with(clone_value_from_task_local);
        warn!(
            traceId = trace_id,
            $($arg)*
        )
    );
}
