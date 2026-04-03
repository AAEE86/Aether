#![allow(
    dead_code,
    unused_assignments,
    unused_imports,
    unused_mut,
    unused_variables,
    clippy::bool_assert_comparison,
    clippy::collapsible_if,
    clippy::empty_line_after_outer_attr,
    clippy::field_reassign_with_default,
    clippy::if_same_then_else,
    clippy::large_enum_variant,
    clippy::manual_div_ceil,
    clippy::manual_find,
    clippy::match_like_matches_macro,
    clippy::needless_as_bytes,
    clippy::needless_lifetimes,
    clippy::nonminimal_bool,
    clippy::question_mark,
    clippy::redundant_closure,
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::useless_concat
)]

mod gateway;

pub use gateway::exports::{
    build_execution_runtime_router, build_execution_runtime_router_with_request_concurrency_limit,
    build_execution_runtime_router_with_request_gates, build_router, build_router_with_state,
    build_tunnel_runtime_router_with_state, serve_execution_runtime_tcp,
    serve_execution_runtime_unix, serve_tcp, strip_cf_headers_middleware, tunnel_protocol,
    AppState, FrontdoorCorsConfig, FrontdoorUserRpmConfig, GatewayDataConfig, TunnelConnConfig,
    TunnelControlPlaneClient, TunnelRuntimeState, UsageRuntimeConfig, VideoTaskTruthSourceMode,
};
