use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use super::model::{PrSeriesPlan, validate_checklist_leaf_ids};
use super::storage;
use crate::infra::tq::TqClient;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SetArgs {
    /// tq task number or UUID
    pub task_id: String,

    /// JSON file containing the complete PR series plan
    #[arg(long)]
    pub file: PathBuf,
}

pub async fn run(args: &SetArgs) -> Result<()> {
    let contents = fs::read(&args.file)
        .with_context(|| format!("failed to read plan file: {}", args.file.display()))?;
    let plan: PrSeriesPlan = serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse plan JSON: {}", args.file.display()))?;
    plan.validate()?;

    let tq_client =
        TqClient::detect().context("tq CLI is required to validate task checklist leaves")?;
    let task = tq_client.task_details(&args.task_id).await?;
    let checklists = tq_client.list_checklists(&task.id).await?;
    validate_checklist_leaf_ids(&plan, &checklists)?;
    storage::save_plan(&task.id, &plan)?;

    println!("Saved PR series plan for tq task {}.", task.number);
    Ok(())
}
