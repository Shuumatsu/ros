//! Console selection over the UART nodes this kernel can drive.

use fdt_raw::Node;
use heapless::String;

use super::PATH_LEN;
use super::table::Device;
use crate::utils::truncated;

pub const MAX_UARTS: usize = 4;

/// A UART node this kernel has a driver for.
pub struct UartNode {
    pub path: String<PATH_LEN>,
    pub device: Device,
}

/// Remove the optional `:options` suffix a `stdout-path` carries.
pub fn console_path(stdout: &str) -> String<PATH_LEN> {
    truncated(stdout.split(':').next().unwrap_or(stdout))
}

/// Prefer the console `/chosen` names; fall back to the first supported UART.
///
/// `chosen` is a path or an alias of one, and `aliases` is the `/aliases` node that defines it.
pub fn resolve(
    uarts: &[UartNode],
    chosen: Option<&str>,
    aliases: Option<&Node<'_>>,
) -> Option<Device> {
    let first = uarts.first().map(|uart| uart.device);
    let Some(chosen) = chosen else { return first };

    let named: Option<String<PATH_LEN>> = if chosen.starts_with('/') {
        Some(truncated(chosen))
    } else {
        aliases.and_then(|node| node.find_property_str(chosen)).map(truncated)
    };
    let Some(resolved) = named else {
        println!(
            "[dtb] WARNING: /chosen names console '{chosen}', which /aliases does not define; \
             using the first UART found"
        );
        return first;
    };
    let Some(uart) = uarts.iter().find(|uart| uart.path == resolved) else {
        println!(
            "[dtb] WARNING: /chosen names console '{resolved}', which is not a UART this kernel \
             can drive; using the first UART found"
        );
        return first;
    };
    Some(uart.device)
}
