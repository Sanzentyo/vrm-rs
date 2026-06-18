use clap::Parser;
use std::error::Error;
use vrm_adapter_ash::{AshVrmFramePlanOptions, frame_plan_from_options};

fn main() -> Result<(), Box<dyn Error>> {
    let options = AshVrmFramePlanOptions::parse();
    let plan = frame_plan_from_options(&options)?;
    println!(
        "ash frame plan: {} primitives, {} materials, {} texture uploads, {} MToon pipeline plans",
        plan.primitives.len(),
        plan.materials.len(),
        plan.texture_uploads.len(),
        plan.mtoon_pipelines.len()
    );
    Ok(())
}
