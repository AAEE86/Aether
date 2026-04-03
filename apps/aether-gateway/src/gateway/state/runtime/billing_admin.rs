use super::{
    admin_billing_collector_from_row, admin_billing_rule_from_row, AdminBillingCollectorRecord,
    AdminBillingCollectorWriteInput, AdminBillingPresetApplyResult, AdminBillingRuleRecord,
    AdminBillingRuleWriteInput, AppState, GatewayError, LocalMutationOutcome,
};

impl AppState {
    pub(crate) async fn admin_billing_enabled_default_value_exists(
        &self,
        api_format: &str,
        task_type: &str,
        dimension_name: &str,
        existing_id: Option<&str>,
    ) -> Result<bool, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            let exists = store
                .lock()
                .expect("admin billing collector store should lock")
                .values()
                .any(|collector| {
                    collector.api_format == api_format
                        && collector.task_type == task_type
                        && collector.dimension_name == dimension_name
                        && collector.is_enabled
                        && collector.default_value.is_some()
                        && existing_id.is_none_or(|value| collector.id != value)
                });
            return Ok(exists);
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(false);
        };
        let exists = sqlx::query_scalar::<_, bool>(
            r#"
SELECT EXISTS(
  SELECT 1
  FROM dimension_collectors
  WHERE api_format = $1
    AND task_type = $2
    AND dimension_name = $3
    AND is_enabled = TRUE
    AND default_value IS NOT NULL
    AND ($4::TEXT IS NULL OR id <> $4)
)
            "#,
        )
        .bind(api_format)
        .bind(task_type)
        .bind(dimension_name)
        .bind(existing_id)
        .fetch_one(&pool)
        .await
        .map_err(|err| GatewayError::Internal(err.to_string()))?;
        Ok(exists)
    }

    pub(crate) async fn create_admin_billing_rule(
        &self,
        input: &AdminBillingRuleWriteInput,
    ) -> Result<LocalMutationOutcome<AdminBillingRuleRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_rule_store.as_ref() {
            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let record = AdminBillingRuleRecord {
                id: uuid::Uuid::new_v4().to_string(),
                name: input.name.clone(),
                task_type: input.task_type.clone(),
                global_model_id: input.global_model_id.clone(),
                model_id: input.model_id.clone(),
                expression: input.expression.clone(),
                variables: input.variables.clone(),
                dimension_mappings: input.dimension_mappings.clone(),
                is_enabled: input.is_enabled,
                created_at_unix_secs: now_unix_secs,
                updated_at_unix_secs: now_unix_secs,
            };
            store
                .lock()
                .expect("admin billing rule store should lock")
                .insert(record.id.clone(), record.clone());
            return Ok(LocalMutationOutcome::Applied(record));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(LocalMutationOutcome::Unavailable);
        };
        let rule_id = uuid::Uuid::new_v4().to_string();
        let row = match sqlx::query(
            r#"
INSERT INTO billing_rules (
  id,
  name,
  task_type,
  global_model_id,
  model_id,
  expression,
  variables,
  dimension_mappings,
  is_enabled,
  created_at,
  updated_at
)
VALUES (
  $1,
  $2,
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  NOW(),
  NOW()
)
RETURNING
  id,
  name,
  task_type,
  global_model_id,
  model_id,
  expression,
  variables,
  dimension_mappings,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
            "#,
        )
        .bind(&rule_id)
        .bind(&input.name)
        .bind(&input.task_type)
        .bind(input.global_model_id.as_deref())
        .bind(input.model_id.as_deref())
        .bind(&input.expression)
        .bind(&input.variables)
        .bind(&input.dimension_mappings)
        .bind(input.is_enabled)
        .fetch_one(&pool)
        .await
        {
            Ok(row) => row,
            Err(sqlx::Error::Database(err)) => {
                return Ok(LocalMutationOutcome::Invalid(format!(
                    "Integrity error: {err}"
                )))
            }
            Err(err) => return Err(GatewayError::Internal(err.to_string())),
        };
        Ok(LocalMutationOutcome::Applied(admin_billing_rule_from_row(
            &row,
        )?))
    }

    pub(crate) async fn list_admin_billing_rules(
        &self,
        task_type: Option<&str>,
        is_enabled: Option<bool>,
        page: u32,
        page_size: u32,
    ) -> Result<Option<(Vec<AdminBillingRuleRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (task_type, is_enabled, page, page_size);

        #[cfg(test)]
        if let Some(store) = self.admin_billing_rule_store.as_ref() {
            let mut items = store
                .lock()
                .expect("admin billing rule store should lock")
                .values()
                .filter(|record| {
                    task_type.is_none_or(|expected| record.task_type == expected)
                        && is_enabled.is_none_or(|expected| record.is_enabled == expected)
                })
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .updated_at_unix_secs
                    .cmp(&left.updated_at_unix_secs)
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let offset = (page.saturating_sub(1) as usize) * (page_size as usize);
            let items = items
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn read_admin_billing_rule(
        &self,
        rule_id: &str,
    ) -> Result<Option<AdminBillingRuleRecord>, GatewayError> {
        #[cfg(not(test))]
        let _ = rule_id;

        #[cfg(test)]
        if let Some(store) = self.admin_billing_rule_store.as_ref() {
            return Ok(store
                .lock()
                .expect("admin billing rule store should lock")
                .get(rule_id)
                .cloned());
        }

        Ok(None)
    }

    pub(crate) async fn update_admin_billing_rule(
        &self,
        rule_id: &str,
        input: &AdminBillingRuleWriteInput,
    ) -> Result<LocalMutationOutcome<AdminBillingRuleRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_rule_store.as_ref() {
            let mut guard = store.lock().expect("admin billing rule store should lock");
            let Some(record) = guard.get_mut(rule_id) else {
                return Ok(LocalMutationOutcome::NotFound);
            };
            record.name = input.name.clone();
            record.task_type = input.task_type.clone();
            record.global_model_id = input.global_model_id.clone();
            record.model_id = input.model_id.clone();
            record.expression = input.expression.clone();
            record.variables = input.variables.clone();
            record.dimension_mappings = input.dimension_mappings.clone();
            record.is_enabled = input.is_enabled;
            record.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            return Ok(LocalMutationOutcome::Applied(record.clone()));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(LocalMutationOutcome::Unavailable);
        };
        let row = match sqlx::query(
            r#"
UPDATE billing_rules
SET
  name = $2,
  task_type = $3,
  global_model_id = $4,
  model_id = $5,
  expression = $6,
  variables = $7,
  dimension_mappings = $8,
  is_enabled = $9,
  updated_at = NOW()
WHERE id = $1
RETURNING
  id,
  name,
  task_type,
  global_model_id,
  model_id,
  expression,
  variables,
  dimension_mappings,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
            "#,
        )
        .bind(rule_id)
        .bind(&input.name)
        .bind(&input.task_type)
        .bind(input.global_model_id.as_deref())
        .bind(input.model_id.as_deref())
        .bind(&input.expression)
        .bind(&input.variables)
        .bind(&input.dimension_mappings)
        .bind(input.is_enabled)
        .fetch_optional(&pool)
        .await
        {
            Ok(row) => row,
            Err(sqlx::Error::Database(err)) => {
                return Ok(LocalMutationOutcome::Invalid(format!(
                    "Integrity error: {err}"
                )))
            }
            Err(err) => return Err(GatewayError::Internal(err.to_string())),
        };
        match row {
            Some(row) => Ok(LocalMutationOutcome::Applied(admin_billing_rule_from_row(
                &row,
            )?)),
            None => Ok(LocalMutationOutcome::NotFound),
        }
    }

    pub(crate) async fn create_admin_billing_collector(
        &self,
        input: &AdminBillingCollectorWriteInput,
    ) -> Result<LocalMutationOutcome<AdminBillingCollectorRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let record = AdminBillingCollectorRecord {
                id: uuid::Uuid::new_v4().to_string(),
                api_format: input.api_format.clone(),
                task_type: input.task_type.clone(),
                dimension_name: input.dimension_name.clone(),
                source_type: input.source_type.clone(),
                source_path: input.source_path.clone(),
                value_type: input.value_type.clone(),
                transform_expression: input.transform_expression.clone(),
                default_value: input.default_value.clone(),
                priority: input.priority,
                is_enabled: input.is_enabled,
                created_at_unix_secs: now_unix_secs,
                updated_at_unix_secs: now_unix_secs,
            };
            store
                .lock()
                .expect("admin billing collector store should lock")
                .insert(record.id.clone(), record.clone());
            return Ok(LocalMutationOutcome::Applied(record));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(LocalMutationOutcome::Unavailable);
        };
        let collector_id = uuid::Uuid::new_v4().to_string();
        let row = match sqlx::query(
            r#"
INSERT INTO dimension_collectors (
  id,
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  created_at,
  updated_at
)
VALUES (
  $1,
  $2,
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  $10,
  $11,
  NOW(),
  NOW()
)
RETURNING
  id,
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
            "#,
        )
        .bind(&collector_id)
        .bind(&input.api_format)
        .bind(&input.task_type)
        .bind(&input.dimension_name)
        .bind(&input.source_type)
        .bind(input.source_path.as_deref())
        .bind(&input.value_type)
        .bind(input.transform_expression.as_deref())
        .bind(input.default_value.as_deref())
        .bind(input.priority)
        .bind(input.is_enabled)
        .fetch_one(&pool)
        .await
        {
            Ok(row) => row,
            Err(sqlx::Error::Database(err)) => {
                return Ok(LocalMutationOutcome::Invalid(format!(
                    "Integrity error: {err}"
                )))
            }
            Err(err) => return Err(GatewayError::Internal(err.to_string())),
        };
        Ok(LocalMutationOutcome::Applied(
            admin_billing_collector_from_row(&row)?,
        ))
    }

    pub(crate) async fn list_admin_billing_collectors(
        &self,
        api_format: Option<&str>,
        task_type: Option<&str>,
        dimension_name: Option<&str>,
        is_enabled: Option<bool>,
        page: u32,
        page_size: u32,
    ) -> Result<Option<(Vec<AdminBillingCollectorRecord>, u64)>, GatewayError> {
        #[cfg(not(test))]
        let _ = (
            api_format,
            task_type,
            dimension_name,
            is_enabled,
            page,
            page_size,
        );

        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            let mut items = store
                .lock()
                .expect("admin billing collector store should lock")
                .values()
                .filter(|record| {
                    api_format.is_none_or(|expected| record.api_format == expected)
                        && task_type.is_none_or(|expected| record.task_type == expected)
                        && dimension_name.is_none_or(|expected| record.dimension_name == expected)
                        && is_enabled.is_none_or(|expected| record.is_enabled == expected)
                })
                .cloned()
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                right
                    .updated_at_unix_secs
                    .cmp(&left.updated_at_unix_secs)
                    .then_with(|| right.priority.cmp(&left.priority))
                    .then_with(|| right.id.cmp(&left.id))
            });
            let total = items.len() as u64;
            let offset = (page.saturating_sub(1) as usize) * (page_size as usize);
            let items = items
                .into_iter()
                .skip(offset)
                .take(page_size as usize)
                .collect::<Vec<_>>();
            return Ok(Some((items, total)));
        }

        Ok(None)
    }

    pub(crate) async fn read_admin_billing_collector(
        &self,
        collector_id: &str,
    ) -> Result<Option<AdminBillingCollectorRecord>, GatewayError> {
        #[cfg(not(test))]
        let _ = collector_id;

        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            return Ok(store
                .lock()
                .expect("admin billing collector store should lock")
                .get(collector_id)
                .cloned());
        }

        Ok(None)
    }

    pub(crate) async fn update_admin_billing_collector(
        &self,
        collector_id: &str,
        input: &AdminBillingCollectorWriteInput,
    ) -> Result<LocalMutationOutcome<AdminBillingCollectorRecord>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            let mut guard = store
                .lock()
                .expect("admin billing collector store should lock");
            let Some(record) = guard.get_mut(collector_id) else {
                return Ok(LocalMutationOutcome::NotFound);
            };
            record.api_format = input.api_format.clone();
            record.task_type = input.task_type.clone();
            record.dimension_name = input.dimension_name.clone();
            record.source_type = input.source_type.clone();
            record.source_path = input.source_path.clone();
            record.value_type = input.value_type.clone();
            record.transform_expression = input.transform_expression.clone();
            record.default_value = input.default_value.clone();
            record.priority = input.priority;
            record.is_enabled = input.is_enabled;
            record.updated_at_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            return Ok(LocalMutationOutcome::Applied(record.clone()));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(LocalMutationOutcome::Unavailable);
        };
        let row = match sqlx::query(
            r#"
UPDATE dimension_collectors
SET
  api_format = $2,
  task_type = $3,
  dimension_name = $4,
  source_type = $5,
  source_path = $6,
  value_type = $7,
  transform_expression = $8,
  default_value = $9,
  priority = $10,
  is_enabled = $11,
  updated_at = NOW()
WHERE id = $1
RETURNING
  id,
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  CAST(EXTRACT(EPOCH FROM created_at) AS BIGINT) AS created_at_unix_secs,
  CAST(EXTRACT(EPOCH FROM updated_at) AS BIGINT) AS updated_at_unix_secs
            "#,
        )
        .bind(collector_id)
        .bind(&input.api_format)
        .bind(&input.task_type)
        .bind(&input.dimension_name)
        .bind(&input.source_type)
        .bind(input.source_path.as_deref())
        .bind(&input.value_type)
        .bind(input.transform_expression.as_deref())
        .bind(input.default_value.as_deref())
        .bind(input.priority)
        .bind(input.is_enabled)
        .fetch_optional(&pool)
        .await
        {
            Ok(row) => row,
            Err(sqlx::Error::Database(err)) => {
                return Ok(LocalMutationOutcome::Invalid(format!(
                    "Integrity error: {err}"
                )))
            }
            Err(err) => return Err(GatewayError::Internal(err.to_string())),
        };
        match row {
            Some(row) => Ok(LocalMutationOutcome::Applied(
                admin_billing_collector_from_row(&row)?,
            )),
            None => Ok(LocalMutationOutcome::NotFound),
        }
    }

    pub(crate) async fn apply_admin_billing_preset(
        &self,
        preset: &str,
        mode: &str,
        collectors: &[AdminBillingCollectorWriteInput],
    ) -> Result<LocalMutationOutcome<AdminBillingPresetApplyResult>, GatewayError> {
        #[cfg(test)]
        if let Some(store) = self.admin_billing_collector_store.as_ref() {
            let mut created = 0_u64;
            let mut updated = 0_u64;
            let mut skipped = 0_u64;
            let now_unix_secs = chrono::Utc::now().timestamp().max(0) as u64;
            let mut guard = store
                .lock()
                .expect("admin billing collector store should lock");
            for collector in collectors {
                let existing_id = guard
                    .values()
                    .find(|record| {
                        record.api_format == collector.api_format
                            && record.task_type == collector.task_type
                            && record.dimension_name == collector.dimension_name
                            && record.priority == collector.priority
                            && record.is_enabled
                    })
                    .map(|record| record.id.clone());

                match existing_id {
                    Some(existing_id) if mode == "overwrite" => {
                        if let Some(record) = guard.get_mut(&existing_id) {
                            record.source_type = collector.source_type.clone();
                            record.source_path = collector.source_path.clone();
                            record.value_type = collector.value_type.clone();
                            record.transform_expression = collector.transform_expression.clone();
                            record.default_value = collector.default_value.clone();
                            record.is_enabled = collector.is_enabled;
                            record.updated_at_unix_secs = now_unix_secs;
                            updated += 1;
                        } else {
                            skipped += 1;
                        }
                    }
                    Some(_) => {
                        skipped += 1;
                    }
                    None => {
                        let record = AdminBillingCollectorRecord {
                            id: uuid::Uuid::new_v4().to_string(),
                            api_format: collector.api_format.clone(),
                            task_type: collector.task_type.clone(),
                            dimension_name: collector.dimension_name.clone(),
                            source_type: collector.source_type.clone(),
                            source_path: collector.source_path.clone(),
                            value_type: collector.value_type.clone(),
                            transform_expression: collector.transform_expression.clone(),
                            default_value: collector.default_value.clone(),
                            priority: collector.priority,
                            is_enabled: collector.is_enabled,
                            created_at_unix_secs: now_unix_secs,
                            updated_at_unix_secs: now_unix_secs,
                        };
                        guard.insert(record.id.clone(), record);
                        created += 1;
                    }
                }
            }
            return Ok(LocalMutationOutcome::Applied(
                AdminBillingPresetApplyResult {
                    preset: preset.to_string(),
                    mode: mode.to_string(),
                    created,
                    updated,
                    skipped,
                    errors: Vec::new(),
                },
            ));
        }

        let Some(pool) = self.postgres_pool() else {
            return Ok(LocalMutationOutcome::Unavailable);
        };

        let mut created = 0_u64;
        let mut updated = 0_u64;
        let mut skipped = 0_u64;
        let mut errors = Vec::new();

        for collector in collectors {
            let existing_id = match sqlx::query_scalar::<_, String>(
                r#"
SELECT id
FROM dimension_collectors
WHERE api_format = $1
  AND task_type = $2
  AND dimension_name = $3
  AND priority = $4
  AND is_enabled = TRUE
LIMIT 1
                "#,
            )
            .bind(&collector.api_format)
            .bind(&collector.task_type)
            .bind(&collector.dimension_name)
            .bind(collector.priority)
            .fetch_optional(&pool)
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    errors.push(format!(
                        "Failed to query collector: api_format={} task_type={} dim={}: {}",
                        collector.api_format, collector.task_type, collector.dimension_name, err
                    ));
                    continue;
                }
            };

            if let Some(existing_id) = existing_id {
                if mode == "overwrite" {
                    match sqlx::query(
                        r#"
UPDATE dimension_collectors
SET
  source_type = $2,
  source_path = $3,
  value_type = $4,
  transform_expression = $5,
  default_value = $6,
  is_enabled = $7,
  updated_at = NOW()
WHERE id = $1
                        "#,
                    )
                    .bind(&existing_id)
                    .bind(&collector.source_type)
                    .bind(collector.source_path.as_deref())
                    .bind(&collector.value_type)
                    .bind(collector.transform_expression.as_deref())
                    .bind(collector.default_value.as_deref())
                    .bind(collector.is_enabled)
                    .execute(&pool)
                    .await
                    {
                        Ok(_) => updated += 1,
                        Err(err) => errors.push(format!(
                            "Failed to update collector {}: {}",
                            existing_id, err
                        )),
                    }
                } else {
                    skipped += 1;
                }
                continue;
            }

            match sqlx::query(
                r#"
INSERT INTO dimension_collectors (
  id,
  api_format,
  task_type,
  dimension_name,
  source_type,
  source_path,
  value_type,
  transform_expression,
  default_value,
  priority,
  is_enabled,
  created_at,
  updated_at
)
VALUES (
  $1,
  $2,
  $3,
  $4,
  $5,
  $6,
  $7,
  $8,
  $9,
  $10,
  $11,
  NOW(),
  NOW()
)
                "#,
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&collector.api_format)
            .bind(&collector.task_type)
            .bind(&collector.dimension_name)
            .bind(&collector.source_type)
            .bind(collector.source_path.as_deref())
            .bind(&collector.value_type)
            .bind(collector.transform_expression.as_deref())
            .bind(collector.default_value.as_deref())
            .bind(collector.priority)
            .bind(collector.is_enabled)
            .execute(&pool)
            .await
            {
                Ok(_) => created += 1,
                Err(err) => errors.push(format!(
                    "Failed to create collector: api_format={} task_type={} dim={}: {}",
                    collector.api_format, collector.task_type, collector.dimension_name, err
                )),
            }
        }

        Ok(LocalMutationOutcome::Applied(
            AdminBillingPresetApplyResult {
                preset: preset.to_string(),
                mode: mode.to_string(),
                created,
                updated,
                skipped,
                errors,
            },
        ))
    }
}
