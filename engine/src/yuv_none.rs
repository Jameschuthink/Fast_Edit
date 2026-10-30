//! The GPU BGRA -> yuv420p pass is Linux only (yuv.rs): without a GPU worker there is never one, and the encoder
//! input is converted on the CPU. Same names as yuv.rs, so render.rs needs no fence.

use std::convert::Infallible;

use anyhow::Result;
use skia_safe::Surface;
use skia_safe::gpu::DirectContext;

/// Never exists off Linux: there is no `Gpu` to make it from.
pub struct GpuYuv(Infallible);

impl GpuYuv {
    pub fn new(_g: &mut crate::gpu::Gpu, _w: i32, _h: i32) -> Option<Result<GpuYuv>> {
        None
    }

    pub fn submit(&mut self, _ctx: &mut DirectContext, _src: &mut Surface) -> Result<()> {
        match self.0 {}
    }

    pub fn collect(&mut self, _out: &mut Vec<u8>) -> Result<()> {
        match self.0 {}
    }
}
