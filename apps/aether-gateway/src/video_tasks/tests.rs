use std::collections::BTreeMap;

use aether_contracts::{ExecutionPlan, ExecutionTimeouts, ProxySnapshot, RequestBody};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::{
    GatewayControlAuthContext, GeminiVideoTaskSeed, LocalVideoTaskContentAction,
    LocalVideoTaskPersistence, LocalVideoTaskSeed, LocalVideoTaskSnapshot, LocalVideoTaskStatus,
    LocalVideoTaskTransport, OpenAiVideoTaskSeed, VideoTaskService, VideoTaskSyncReportMode,
    VideoTaskTruthSourceMode,
};

mod fixtures;
mod plans;
mod projection;
mod sync;
