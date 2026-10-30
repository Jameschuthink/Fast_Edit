//! Painting on the GPU is Linux only (Vulkan, gpu.rs): on another OS a GPU worker cannot be made, so every worker
//! paints on the CPU. Same names as gpu.rs, so render.rs needs no fence.

use std::convert::Infallible;

use anyhow::{Result, anyhow};
use skia_safe::gpu::DirectContext;

/// Never exists off Linux: `new` always fails, and the `Infallible` field makes one impossible to build.
pub struct Gpu {
    pub context: DirectContext,
    _never: Infallible,
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        Err(anyhow!("GPU painting is Linux only (Vulkan)"))
    }
}
