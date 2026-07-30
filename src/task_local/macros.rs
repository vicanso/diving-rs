// task local log
#[macro_export]
macro_rules! tl_info {
    ($($arg:tt)*) => (
        // 无 scope 时（如库调用方 / 测试直接调用）降级为空 traceId，
        // 而不是 panic（release 下 panic=abort 会带走整个进程）。
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        info!(
            traceId = trace_id,
            $($arg)*
        )
    );
}

#[macro_export]
macro_rules! tl_error {
    ($($arg:tt)*) => (
        // 无 scope 时（如库调用方 / 测试直接调用）降级为空 traceId，
        // 而不是 panic（release 下 panic=abort 会带走整个进程）。
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        error!(
            traceId = trace_id,
            $($arg)*
        )
    );
}

#[macro_export]
macro_rules! tl_warn {
    ($($arg:tt)*) => (
        // 无 scope 时（如库调用方 / 测试直接调用）降级为空 traceId，
        // 而不是 panic（release 下 panic=abort 会带走整个进程）。
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        warn!(
            traceId = trace_id,
            $($arg)*
        )
    );
}
