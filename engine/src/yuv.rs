//! BGRA -> yuv420p on the GPU (the idea of fframes' frame_export/planes.rs, docs/FFRAMES_ARCHITECTURE.md §12 #1).
//! One SkSL pass writes the three planes into one RGBA8 surface w/4 texels wide and h*3/2 rows high: each texel holds
//! 4 bytes of a plane, so one copy with w bytes per row returns the planes back to back, exactly the layout
//! `bgra_to_yuv420` makes. The readback is 1.5 B/px instead of 4, and the CPU does no conversion.
//! Same numbers as `bgra_to_yuv420` (BT.709 limited, chroma = the 2x2 average, here by bilinear sampling at the
//! corner between the four pixels), within 1 step of rounding. Needs w % 8 == 0 and h % 4 == 0; else the caller
//! uses the CPU path. `AIOS_GPU_YUV=0` turns it off, `AIOS_GPU_YUV=check` compares both paths on every frame.
//!
//! The readback does not block (fframes' backends/vulkan_readback.rs, §12 #2): `submit` hands the frame to the GPU
//! with our own copy into host-cached memory behind a fence, and the worker records its next frame while the GPU
//! paints and copies this one; `collect` then waits on the fence (by then usually done) and takes the bytes. Two slots
//! (packed surface + staging buffer + fence) alternate. If the Vulkan part cannot be set up, `submit` reads back at
//! once with Skia's read_pixels (`AIOS_GPU_YUV=sync` forces that).
use anyhow::{anyhow, Result};
use ash::vk::{self, Handle};
use skia_safe::runtime_effect::ChildPtr;
use skia_safe::surface::BackendHandleAccess;
use skia_safe::{gpu, AlphaType, BlendMode, ColorType, Data, FilterMode, ImageInfo, MipmapMode, Paint, Rect, RuntimeEffect, SamplingOptions, Surface, TileMode};
use std::collections::VecDeque;

const SKSL: &str = "
uniform shader frame;
uniform float mode;   // 0 = Y, 1 = U, 2 = V
uniform float row0;   // first row of the plane in the packed surface
uniform float w;      // frame width = bytes per packed row
uniform float cw;     // chroma width
uniform float3 k;     // weights on premultiplied rgb in 0..1
uniform float off;

float px(float2 p) { return dot(frame.eval(p).rgb, k); }

half4 main(float2 p) {
    float tx = floor(p.x) * 4.0;
    float ty = floor(p.y) - row0;
    if (mode < 0.5) {
        float y = ty + 0.5;
        return half4(float4(px(float2(tx + 0.5, y)), px(float2(tx + 1.5, y)), px(float2(tx + 2.5, y)), px(float2(tx + 3.5, y))) + off);
    }
    // byte b of the chroma plane is sample (cx, cy); 4 bytes never cross a chroma row (cw % 4 == 0)
    float b = ty * w + tx;
    float cy = floor((b + 0.5) / cw);
    float cx = b - cy * cw;
    float sx = cx * 2.0 + 1.0;
    float sy = cy * 2.0 + 1.0;
    return half4(float4(px(float2(sx, sy)), px(float2(sx + 2.0, sy)), px(float2(sx + 4.0, sy)), px(float2(sx + 6.0, sy))) + off);
}
";

/// (weights, offset) per plane, from the integer ones in `bgra_to_yuv420`
const PLANES: [([f32; 3], f32); 3] = [
    ([11966. / 65536., 40254. / 65536., 4064. / 65536.], 16. / 255.),
    ([-6596. / 65536., -22189. / 65536., 28785. / 65536.], 128. / 255.),
    ([28785. / 65536., -26145. / 65536., -2640. / 65536.], 128. / 255.),
];

/// Host-cached memory one frame's planes are copied into, and the fence that says the copy is done.
struct Staging {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *const u8,
    commands: vk::CommandBuffer,
    fence: vk::Fence,
}

struct Slot {
    surface: Surface,
    staging: Option<Staging>,
    /// the sync fallback's bytes (no staging)
    bytes: Vec<u8>,
}

struct Vulkan {
    device: ash::Device,
    queue: vk::Queue,
    pool: vk::CommandPool,
}

pub struct GpuYuv {
    info: ImageInfo,
    effect: RuntimeEffect,
    w: i32,
    h: i32,
    slots: Vec<Slot>,
    vk: Option<Vulkan>,
    /// slots submitted and not collected, oldest first
    busy: VecDeque<usize>,
    next: usize,
}

impl GpuYuv {
    /// None when the size does not pack (the CPU path then does the frame) or the env turns it off.
    pub fn new(g: &mut crate::gpu::Gpu, w: i32, h: i32) -> Option<Result<GpuYuv>> {
        let mode = std::env::var("AIOS_GPU_YUV").unwrap_or_default();
        if w % 8 != 0 || h % 4 != 0 || mode == "0" {
            return None;
        }
        Some(Self::make(g, w, h, mode != "sync"))
    }

    fn make(g: &mut crate::gpu::Gpu, w: i32, h: i32, own_copy: bool) -> Result<GpuYuv> {
        // no colour space: the frame's bytes go into the shader as they are, like the CPU path reads them
        let info = ImageInfo::new((w / 4, h * 3 / 2), ColorType::RGBA8888, AlphaType::Premul, None);
        let effect = RuntimeEffect::make_for_shader(SKSL, None).map_err(|e| anyhow!("YUV shader: {e}"))?;
        let mut slots = Vec::new();
        for _ in 0..2 {
            let surface = gpu::surfaces::render_target(&mut g.context, gpu::Budgeted::Yes, &info, None, gpu::SurfaceOrigin::TopLeft, None, false, None)
                .ok_or_else(|| anyhow!("cannot make the {}x{} YUV surface", w / 4, h * 3 / 2))?;
            slots.push(Slot { surface, staging: None, bytes: Vec::new() });
        }
        let mut me = GpuYuv { info, effect, w, h, slots, vk: None, busy: VecDeque::new(), next: 0 };
        if own_copy {
            if let Err(e) = me.make_staging(g) {
                eprintln!("GPU YUV: own readback off ({e:#}): reading back with Skia");
                me.free_staging();
            }
        }
        Ok(me)
    }

    fn make_staging(&mut self, g: &crate::gpu::Gpu) -> Result<()> {
        let d = g.device.clone();
        let size = (self.w * self.h * 3 / 2) as u64;
        unsafe {
            let pool = d.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(g.queue_family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None)?;
            self.vk = Some(Vulkan { device: d.clone(), queue: g.queue, pool });
            for slot in &mut self.slots {
                let buffer = d.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_DST).sharing_mode(vk::SharingMode::EXCLUSIVE), None)?;
                let req = d.get_buffer_memory_requirements(buffer);
                // cached memory reads at the speed of ordinary memory; uncached (write-combined) is many times slower
                let want = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT | vk::MemoryPropertyFlags::HOST_CACHED;
                let Some(ty) = (0..g.memory.memory_type_count).find(|&i| req.memory_type_bits & (1 << i) != 0 && g.memory.memory_types[i as usize].property_flags.contains(want)) else {
                    d.destroy_buffer(buffer, None);
                    return Err(anyhow!("no host-cached memory"));
                };
                let memory = match d.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty), None) {
                    Ok(m) => m,
                    Err(e) => { d.destroy_buffer(buffer, None); return Err(e.into()); }
                };
                let staged = d.bind_buffer_memory(buffer, memory, 0).and_then(|_| d.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()));
                let mapped = match staged {
                    Ok(p) => p as *const u8,
                    Err(e) => { d.destroy_buffer(buffer, None); d.free_memory(memory, None); return Err(e.into()); }
                };
                let commands = d.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1))?[0];
                let fence = d.create_fence(&vk::FenceCreateInfo::default(), None)?;
                slot.staging = Some(Staging { buffer, memory, mapped, commands, fence });
            }
        }
        Ok(())
    }

    fn free_staging(&mut self) {
        let Some(v) = self.vk.take() else { return };
        unsafe {
            for slot in &mut self.slots {
                if let Some(s) = slot.staging.take() {
                    let _ = v.device.wait_for_fences(&[s.fence], true, 1_000_000_000);
                    v.device.destroy_fence(s.fence, None);
                    v.device.unmap_memory(s.memory);
                    v.device.destroy_buffer(s.buffer, None);
                    v.device.free_memory(s.memory, None);
                }
            }
            v.device.destroy_command_pool(v.pool, None);
        }
    }

    pub fn pending(&self) -> usize {
        self.busy.len()
    }

    fn uniforms(&self, vals: &[(&str, &[f32])]) -> Result<Data> {
        let mut d = vec![0u8; self.effect.uniform_size()];
        for (name, v) in vals {
            let u = self.effect.uniforms().iter().find(|u| u.name() == *name).ok_or_else(|| anyhow!("YUV shader has no `{name}`"))?;
            for (i, x) in v.iter().enumerate() {
                d[u.offset() + 4 * i..u.offset() + 4 * i + 4].copy_from_slice(&x.to_ne_bytes());
            }
        }
        Ok(Data::new_copy(&d))
    }

    /// Converts what `src` holds now and starts its readback; `collect` returns the bytes. At most two frames are in
    /// flight: collect one before the third submit.
    pub fn submit(&mut self, ctx: &mut gpu::DirectContext, src: &mut Surface) -> Result<()> {
        if self.busy.len() >= self.slots.len() {
            return Err(anyhow!("YUV readback: both slots busy"));
        }
        let (w, h) = (self.w, self.h);
        let k = self.next;
        self.next = (k + 1) % self.slots.len();
        {
            let img = src.image_snapshot();
            let sampled = |f: FilterMode| img.to_shader((TileMode::Clamp, TileMode::Clamp), SamplingOptions::new(f, MipmapMode::None), None).ok_or_else(|| anyhow!("frame shader"));
            let (exact, linear) = (sampled(FilterMode::Nearest)?, sampled(FilterMode::Linear)?);
            let (tw, rows) = ((w / 4) as f32, [0., h as f32, (h + h / 4) as f32]);
            for (m, (kk, off)) in PLANES.iter().enumerate() {
                let n = if m == 0 { h } else { h / 4 };
                let u = self.uniforms(&[("mode", &[m as f32]), ("row0", &[rows[m]]), ("w", &[w as f32]), ("cw", &[(w / 2) as f32]), ("k", kk), ("off", &[*off])])?;
                let child = if m == 0 { exact.clone() } else { linear.clone() };
                let shader = self.effect.make_shader(u, &[ChildPtr::Shader(child)], None).ok_or_else(|| anyhow!("YUV shader instance"))?;
                let mut p = Paint::default();
                p.set_shader(shader);
                p.set_blend_mode(BlendMode::Src);
                self.slots[k].surface.canvas().draw_rect(Rect::from_xywh(0., rows[m], tw, n as f32), &p);
            }
        }
        let size = (w * h * 3 / 2) as usize;
        let slot = &mut self.slots[k];
        match (&self.vk, &slot.staging) {
            (Some(v), Some(s)) => {
                let image = gpu::surfaces::get_backend_texture(&mut slot.surface, BackendHandleAccess::FlushRead)
                    .and_then(|t| t.vulkan_image_info())
                    .map(|i| vk::Image::from_raw(*i.image() as usize as u64))
                    .ok_or_else(|| anyhow!("the YUV surface has no Vulkan image"))?;
                // Skia records the draws and leaves the image ready to copy from; nothing waits here
                ctx.flush_surface_with_texture_state(
                    &mut slot.surface,
                    &gpu::FlushInfo::default(),
                    Some(&gpu::vk::mutable_texture_states::new_vulkan(gpu::vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::QUEUE_FAMILY_IGNORED)),
                );
                unsafe {
                    let d = &v.device;
                    d.reset_fences(&[s.fence])?;
                    d.reset_command_buffer(s.commands, vk::CommandBufferResetFlags::empty())?;
                    d.begin_command_buffer(s.commands, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
                    let region = vk::BufferImageCopy {
                        buffer_offset: 0,
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 },
                        image_offset: vk::Offset3D::default(),
                        image_extent: vk::Extent3D { width: (w / 4) as u32, height: (h * 3 / 2) as u32, depth: 1 },
                    };
                    d.cmd_copy_image_to_buffer(s.commands, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, s.buffer, &[region]);
                    // the copy's writes made visible to the host before the fence signals
                    let barrier = vk::BufferMemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::HOST_READ)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .buffer(s.buffer)
                        .offset(0)
                        .size(vk::WHOLE_SIZE);
                    d.cmd_pipeline_barrier(s.commands, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::HOST, vk::DependencyFlags::empty(), &[], &[barrier], &[]);
                    d.end_command_buffer(s.commands)?;
                    // one queue, run in order: Skia's draws, then the copy
                    if !ctx.submit(gpu::SyncCpu::No) {
                        return Err(anyhow!("Skia submit failed"));
                    }
                    let cmds = [s.commands];
                    d.queue_submit(v.queue, &[vk::SubmitInfo::default().command_buffers(&cmds)], s.fence)?;
                }
            }
            _ => {
                slot.bytes.resize(size, 0);
                if !slot.surface.read_pixels(&self.info, &mut slot.bytes, w as usize, (0, 0)) {
                    return Err(anyhow!("YUV readback"));
                }
            }
        }
        self.busy.push_back(k);
        Ok(())
    }

    /// The oldest submitted frame's planes into `out` (w*h*3/2 bytes), waiting for its copy if needed.
    pub fn collect(&mut self, out: &mut Vec<u8>) -> Result<()> {
        let k = self.busy.pop_front().ok_or_else(|| anyhow!("YUV readback: nothing submitted"))?;
        let size = (self.w * self.h * 3 / 2) as usize;
        let slot = &mut self.slots[k];
        match (&self.vk, &slot.staging) {
            (Some(v), Some(s)) => unsafe {
                v.device.wait_for_fences(&[s.fence], true, 10_000_000_000)?;
                out.resize(size, 0);
                std::ptr::copy_nonoverlapping(s.mapped, out.as_mut_ptr(), size);
            },
            _ => std::mem::swap(out, &mut slot.bytes),
        }
        Ok(())
    }
}

impl Drop for GpuYuv {
    fn drop(&mut self) {
        self.free_staging();
    }
}
