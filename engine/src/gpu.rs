//! Painting on the GPU: one Vulkan device + Skia (Ganesh) context per GPU worker, on the NVIDIA card when there is
//! one. The worker's surface lives in VRAM; images are uploaded by Skia, the frame is read back for the encoder.

use anyhow::{anyhow, Result};
use ash::vk;
use ash::vk::Handle;
use skia_safe::gpu::{self, vk as skvk, DirectContext};

pub struct Gpu {
    pub context: DirectContext,
    /// for yuv.rs's own readback: the queue Skia submits to, its family, the memory types
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub queue_family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    _instance: ash::Instance,
    _entry: ash::Entry,
    pub name: String,
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        let entry = unsafe { ash::Entry::load() }.map_err(|e| anyhow!("Vulkan loader: {e}"))?;
        let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 1, 0));
        let instance = unsafe { entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None) }?;
        let pds = unsafe { instance.enumerate_physical_devices() }?;
        // the NVIDIA card first, then any discrete GPU, then anything
        let score = |pd: &vk::PhysicalDevice| {
            let p = unsafe { instance.get_physical_device_properties(*pd) };
            (if p.vendor_id == 0x10de { 2 } else { 0 }) + (if p.device_type == vk::PhysicalDeviceType::DISCRETE_GPU { 1 } else { 0 })
        };
        let pd = *pds.iter().max_by_key(|pd| score(pd)).ok_or_else(|| anyhow!("no Vulkan device"))?;
        let props = unsafe { instance.get_physical_device_properties(pd) };
        let name = unsafe { std::ffi::CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy().to_string();
        let qf = unsafe { instance.get_physical_device_queue_family_properties(pd) }
            .iter()
            .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .ok_or_else(|| anyhow!("no graphics queue"))? as u32;
        let prio = [1.0f32];
        let qinfo = [vk::DeviceQueueCreateInfo::default().queue_family_index(qf).queue_priorities(&prio)];
        let device = unsafe { instance.create_device(pd, &vk::DeviceCreateInfo::default().queue_create_infos(&qinfo), None) }?;
        let queue = unsafe { device.get_device_queue(qf, 0) };
        let (e2, i2) = (entry.clone(), instance.clone());
        let get_proc = move |of: skvk::GetProcOf| -> skvk::GetProcResult {
            unsafe {
                match of {
                    skvk::GetProcOf::Instance(inst, name) => {
                        let f = e2.get_instance_proc_addr(vk::Instance::from_raw(inst as u64), name);
                        f.map(|f| f as *const std::ffi::c_void).unwrap_or(std::ptr::null())
                    }
                    skvk::GetProcOf::Device(dev, name) => {
                        let f = (i2.fp_v1_0().get_device_proc_addr)(vk::Device::from_raw(dev as u64), name);
                        f.map(|f| f as *const std::ffi::c_void).unwrap_or(std::ptr::null())
                    }
                }
            }
        };
        // SAFETY: instance, device and queue live in the returned Gpu, longer than the context built from them
        let backend = unsafe {
            skvk::BackendContext::new_builder(
                instance.handle().as_raw() as _,
                pd.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, qf as usize),
                &get_proc,
                None,
            )
            .build()
        };
        let context = gpu::direct_contexts::make_vulkan(&backend, None).ok_or_else(|| anyhow!("Skia could not make a Vulkan context on {name}"))?;
        let memory = unsafe { instance.get_physical_device_memory_properties(pd) };
        Ok(Gpu { context, device, queue, queue_family: qf, memory, _instance: instance, _entry: entry, name })
    }
}
