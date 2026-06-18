use clap::Parser;
use std::error::Error;
use vrm_adapter_wgpu::{WgpuVrmViewerOptions, run_vrma_viewer};

fn main() -> Result<(), Box<dyn Error>> {
    run_vrma_viewer(WgpuVrmViewerOptions::parse())
}
