use std::{
    collections::BTreeSet,
    ffi::{CStr, CString},
};

use ash::{Device, Entry, Instance, vk};
use thiserror::Error;
use tracing::{debug, info, warn};

const NVIDIA_VENDOR_ID: u32 = 0x10de;
const REQUIRED_DEVICE_EXTENSIONS: [&CStr; 4] = [
    vk::KhrExternalMemoryFdFn::name(),
    vk::ExtExternalMemoryDmaBufFn::name(),
    vk::ExtImageDrmFormatModifierFn::name(),
    vk::KhrExternalSemaphoreFdFn::name(),
];

#[derive(Debug, Error)]
pub enum NvidiaVulkanError {
    #[error("failed to load Vulkan loader: {0}")]
    Loader(#[from] ash::LoadingError),
    #[error("Vulkan operation {operation} failed: {result}")]
    Vulkan {
        operation: &'static str,
        result: vk::Result,
    },
    #[error("no NVIDIA Vulkan device suitable for graphics rendering found")]
    DeviceNotFound,
    #[error("NVIDIA device {device} is missing required DMA-BUF extensions: {missing:?}")]
    MissingExtensions {
        device: String,
        missing: Vec<String>,
    },
    #[error("NVIDIA device name contains invalid bytes")]
    InvalidDeviceName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NvidiaDeviceInfo {
    pub name: String,
    pub api_version: u32,
    pub driver_version: u32,
    pub queue_family_index: u32,
    pub dma_buf_import: bool,
    pub dma_buf_export: bool,
    pub explicit_sync: bool,
}

/// 持有 NVIDIA Vulkan 逻辑设备。
///
/// Drop 顺序必须保持 device -> instance -> entry，因此字段顺序不可作为资源释放依据，
/// 实际销毁逻辑在 `Drop` 中显式执行。
pub struct NvidiaVulkanContext {
    device: Option<Device>,
    instance: Option<Instance>,
    _entry: Entry,
    info: NvidiaDeviceInfo,
}

impl NvidiaVulkanContext {
    pub fn new() -> Result<Self, NvidiaVulkanError> {
        // SAFETY: ash 负责验证动态库符号，Entry 在 Instance 整个生命周期内被持有。
        let entry = unsafe { Entry::load()? };
        let app_name =
            CString::new("better-wallpaper").expect("fixed application name does not contain NUL");
        let app_info = vk::ApplicationInfo::builder()
            .application_name(&app_name)
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(&app_name)
            .engine_version(vk::make_api_version(0, 0, 1, 0))
            .api_version(vk::API_VERSION_1_2);
        let create_info = vk::InstanceCreateInfo::builder().application_info(&app_info);
        // SAFETY: create_info 仅引用本作用域内仍存活的 app_info/app_name。
        let instance = unsafe { entry.create_instance(&create_info, None) }
            .map_err(|result| vulkan_error("create_instance", result))?;

        match Self::create_device(&instance) {
            Ok((device, info)) => {
                info!(
                    gpu = %info.name,
                    api_version = %format_api_version(info.api_version),
                    driver_version = info.driver_version,
                    queue_family = info.queue_family_index,
                    dma_buf_import = info.dma_buf_import,
                    dma_buf_export = info.dma_buf_export,
                    explicit_sync = info.explicit_sync,
                    "NVIDIA Vulkan DMA-BUF interop capability detection completed"
                );
                Ok(Self {
                    device: Some(device),
                    instance: Some(instance),
                    _entry: entry,
                    info,
                })
            }
            Err(error) => {
                // SAFETY: 尚未创建逻辑设备，且当前没有依赖 instance 的活跃资源。
                unsafe { instance.destroy_instance(None) };
                Err(error)
            }
        }
    }

    pub fn info(&self) -> &NvidiaDeviceInfo {
        &self.info
    }

    fn create_device(instance: &Instance) -> Result<(Device, NvidiaDeviceInfo), NvidiaVulkanError> {
        // SAFETY: instance 有效，返回的物理设备句柄由它拥有。
        let physical_devices = unsafe { instance.enumerate_physical_devices() }
            .map_err(|result| vulkan_error("enumerate_physical_devices", result))?;

        let mut extension_error = None;
        for physical_device in physical_devices {
            // SAFETY: physical_device 来自当前有效 instance。
            let properties = unsafe { instance.get_physical_device_properties(physical_device) };
            if properties.vendor_id != NVIDIA_VENDOR_ID {
                debug!(
                    vendor_id = properties.vendor_id,
                    "skipping non-NVIDIA Vulkan device"
                );
                continue;
            }
            let name = device_name(&properties)?;
            // SAFETY: physical_device 来自当前有效 instance。
            let queue_families =
                unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
            let Some(queue_family_index) = queue_families
                .iter()
                .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .map(|index| index as u32)
            else {
                warn!(gpu = %name, "NVIDIA device has no graphics queue, skipping");
                continue;
            };

            // SAFETY: physical_device 来自当前有效 instance。
            let extensions =
                unsafe { instance.enumerate_device_extension_properties(physical_device) }
                    .map_err(|result| {
                        vulkan_error("enumerate_device_extension_properties", result)
                    })?;
            let available = extension_names(&extensions);
            let missing = missing_extensions(&available);
            if !missing.is_empty() {
                extension_error = Some(NvidiaVulkanError::MissingExtensions {
                    device: name,
                    missing,
                });
                continue;
            }

            let priorities = [1.0_f32];
            let queue_info = [vk::DeviceQueueCreateInfo::builder()
                .queue_family_index(queue_family_index)
                .queue_priorities(&priorities)
                .build()];
            let extension_ptrs: Vec<_> = REQUIRED_DEVICE_EXTENSIONS
                .iter()
                .map(|name| name.as_ptr())
                .collect();
            let device_info = vk::DeviceCreateInfo::builder()
                .queue_create_infos(&queue_info)
                .enabled_extension_names(&extension_ptrs);
            // SAFETY: 队列和扩展指针在调用期间有效，扩展已验证存在。
            let device = unsafe { instance.create_device(physical_device, &device_info, None) }
                .map_err(|result| vulkan_error("create_device", result))?;

            return Ok((
                device,
                NvidiaDeviceInfo {
                    name,
                    api_version: properties.api_version,
                    driver_version: properties.driver_version,
                    queue_family_index,
                    dma_buf_import: true,
                    dma_buf_export: true,
                    explicit_sync: true,
                },
            ));
        }

        Err(extension_error.unwrap_or(NvidiaVulkanError::DeviceNotFound))
    }
}

impl Drop for NvidiaVulkanContext {
    fn drop(&mut self) {
        if let Some(device) = self.device.take() {
            // SAFETY: 设备仍有效；等待完成后销毁所有由该上下文持有的设备资源。
            unsafe {
                if let Err(error) = device.device_wait_idle() {
                    warn!(
                        ?error,
                        "failed to wait for NVIDIA Vulkan device to become idle"
                    );
                }
                device.destroy_device(None);
            }
        }
        if let Some(instance) = self.instance.take() {
            // SAFETY: 逻辑设备已销毁，不再存在依赖 instance 的资源。
            unsafe { instance.destroy_instance(None) };
        }
        debug!(gpu = %self.info.name, "NVIDIA Vulkan context released");
    }
}

fn device_name(properties: &vk::PhysicalDeviceProperties) -> Result<String, NvidiaVulkanError> {
    // SAFETY: Vulkan 规范保证 device_name 是以 NUL 结尾的固定长度字符串。
    let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) };
    name.to_str()
        .map(str::to_owned)
        .map_err(|_| NvidiaVulkanError::InvalidDeviceName)
}

fn extension_names(properties: &[vk::ExtensionProperties]) -> BTreeSet<String> {
    properties
        .iter()
        .filter_map(|property| {
            // SAFETY: Vulkan 规范保证 extension_name 以 NUL 结尾。
            unsafe { CStr::from_ptr(property.extension_name.as_ptr()) }
                .to_str()
                .ok()
                .map(str::to_owned)
        })
        .collect()
}

fn missing_extensions(available: &BTreeSet<String>) -> Vec<String> {
    REQUIRED_DEVICE_EXTENSIONS
        .iter()
        .filter_map(|required| {
            let name = required.to_string_lossy();
            (!available.contains(name.as_ref())).then(|| name.into_owned())
        })
        .collect()
}

fn vulkan_error(operation: &'static str, result: vk::Result) -> NvidiaVulkanError {
    NvidiaVulkanError::Vulkan { operation, result }
}

fn format_api_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        vk::api_version_major(version),
        vk::api_version_minor(version),
        vk::api_version_patch(version)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_all_missing_dma_buf_extensions() {
        let available = BTreeSet::from(["VK_KHR_external_memory_fd".to_owned()]);
        let missing = missing_extensions(&available);
        assert_eq!(missing.len(), 3);
        assert!(
            missing
                .iter()
                .any(|name| name == "VK_EXT_external_memory_dma_buf")
        );
        assert!(
            missing
                .iter()
                .any(|name| name == "VK_EXT_image_drm_format_modifier")
        );
        assert!(
            missing
                .iter()
                .any(|name| name == "VK_KHR_external_semaphore_fd")
        );
    }

    #[test]
    fn accepts_complete_extension_set() {
        let available = REQUIRED_DEVICE_EXTENSIONS
            .iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        assert!(missing_extensions(&available).is_empty());
    }

    #[test]
    fn formats_vulkan_api_version() {
        assert_eq!(
            format_api_version(vk::make_api_version(0, 1, 3, 280)),
            "1.3.280"
        );
    }
}
