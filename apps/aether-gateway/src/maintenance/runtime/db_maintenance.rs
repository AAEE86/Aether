use tracing::warn;

use crate::gateway::gateway_data::GatewayDataState;

use super::{system_config_bool, DB_MAINTENANCE_TABLES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DbMaintenanceRunSummary {
    pub(super) attempted: usize,
    pub(super) succeeded: usize,
}

pub(super) async fn perform_db_maintenance_once(
    data: &GatewayDataState,
) -> Result<DbMaintenanceRunSummary, aether_data::DataLayerError> {
    let Some(pool) = data.postgres_pool() else {
        return Ok(DbMaintenanceRunSummary {
            attempted: 0,
            succeeded: 0,
        });
    };

    run_db_maintenance_with(data, |table_name| {
        let pool = pool.clone();
        async move {
            let statement = format!("VACUUM ANALYZE {table_name}");
            sqlx::raw_sql(&statement).execute(&pool).await?;
            Ok(())
        }
    })
    .await
}

pub(super) async fn run_db_maintenance_with<F, Fut>(
    data: &GatewayDataState,
    mut vacuum_table: F,
) -> Result<DbMaintenanceRunSummary, aether_data::DataLayerError>
where
    F: FnMut(&'static str) -> Fut,
    Fut: std::future::Future<Output = Result<(), aether_data::DataLayerError>>,
{
    if !system_config_bool(data, "enable_db_maintenance", true).await? {
        return Ok(DbMaintenanceRunSummary {
            attempted: 0,
            succeeded: 0,
        });
    }

    let mut summary = DbMaintenanceRunSummary {
        attempted: 0,
        succeeded: 0,
    };
    for table_name in DB_MAINTENANCE_TABLES {
        summary.attempted += 1;
        match vacuum_table(table_name).await {
            Ok(()) => summary.succeeded += 1,
            Err(err) => {
                warn!(table = table_name, error = %err, "gateway db maintenance table failed");
            }
        }
    }
    Ok(summary)
}
