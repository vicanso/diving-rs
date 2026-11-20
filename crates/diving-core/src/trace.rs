use nanoid::nanoid;

pub fn clone_value_from_task_local<T>(value: &T) -> T
where
    T: Clone,
{
    value.clone()
}

tokio::task_local! {
    pub static TRACE_ID: String;
}

pub fn generate_trace_id() -> String {
    nanoid!(6)
}

#[macro_export]
macro_rules! tl_info {
    ($($arg:tt)*) => ({
        // 无 scope 时（如库调用方 / 测试直接调用）降级为空 traceId，
        // 而不是 panic（release 下 panic=abort 会带走整个进程）。
        let trace_id = $crate::trace::TRACE_ID
            .try_with($crate::trace::clone_value_from_task_local)
            .unwrap_or_default();
        $crate::__tracing::info!(
            traceId = trace_id,
            $($arg)*
        )
    });
}

#[macro_export]
macro_rules! tl_error {
    ($($arg:tt)*) => ({
        let trace_id = $crate::trace::TRACE_ID
            .try_with($crate::trace::clone_value_from_task_local)
            .unwrap_or_default();
        $crate::__tracing::error!(
            traceId = trace_id,
            $($arg)*
        )
    });
}

#[macro_export]
macro_rules! tl_warn {
    ($($arg:tt)*) => ({
        let trace_id = $crate::trace::TRACE_ID
            .try_with($crate::trace::clone_value_from_task_local)
            .unwrap_or_default();
        $crate::__tracing::warn!(
            traceId = trace_id,
            $($arg)*
        )
    });
}
