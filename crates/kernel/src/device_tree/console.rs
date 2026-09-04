use fdt_raw::Fdt;
use heapless::String;

use super::bus::PATH_LEN;
use super::table::Device;
use crate::utils::truncated;

pub const MAX_UARTS: usize = 4;

pub struct UartNode {
    pub path: String<PATH_LEN>,
    pub device: Device,
}

/// Remove the optional `:options` suffix from `stdout-path`.
pub fn console_path(stdout: &str) -> String<PATH_LEN> {
    truncated(stdout.split(':').next().unwrap_or(stdout))
}

/// Prefer `/chosen/stdout-path`; fall back to the first supported UART.
pub fn resolve(fdt: &Fdt<'_>, uarts: &[UartNode], chosen: Option<&str>) -> Option<Device> {
    let first = uarts.first().map(|uart| uart.device);
    let Some(chosen) = chosen else { return first };

    let resolved = if chosen.starts_with('/') {
        truncated(chosen)
    } else {
        match fdt.find_by_path("/aliases").and_then(|node| node.find_property_str(chosen)) {
            Some(path) => console_path(path),
            None => {
                println!(
                    "[dtb] WARNING: /chosen names console '{chosen}', which /aliases \
                          does not define; using the first UART found"
                );
                return first;
            }
        }
    };

    match uarts.iter().find(|uart| uart.path == resolved) {
        Some(uart) => Some(uart.device),
        None => {
            println!(
                "[dtb] WARNING: /chosen names console '{resolved}', which is not a UART this \
                 kernel can drive; using the first UART found"
            );
            first
        }
    }
}
