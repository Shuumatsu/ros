//! Choosing the console among the UARTs the walk found.
//!
//! Deferred to the end of the walk because `/chosen` may name a node the walk has not reached
//! yet, and because a node is only a candidate once it has resolved a window.

use fdt_raw::Fdt;
use heapless::String;

use super::bus::PATH_LEN;
use super::table::Device;
use crate::utils::truncated;

/// UART nodes remembered while looking for the one `/chosen` names.
pub const MAX_UARTS: usize = 4;

/// A UART the kernel could drive, kept until `/chosen` has been read and the console can be
/// chosen rather than guessed.
pub struct UartNode {
    pub path: String<PATH_LEN>,
    pub device: Device,
}

/// The node path out of a `stdout-path`, whose value is `path` or `path:options`.
pub fn console_path(stdout: &str) -> String<PATH_LEN> {
    truncated(stdout.split(':').next().unwrap_or(stdout))
}

/// Which of the UARTs found is the console.
///
/// `/chosen/stdout-path` is the tree's own answer, so it wins: a board that lists an
/// unpopulated port ahead of the real one is otherwise a silent failure, since `console` drops
/// the SBI fallback as soon as a base exists. Without `/chosen` the first UART found keeps the
/// tree bootable.
pub fn resolve(fdt: &Fdt<'_>, uarts: &[UartNode], chosen: Option<&str>) -> Option<Device> {
    let first = uarts.first().map(|uart| uart.device);
    let Some(chosen) = chosen else { return first };

    // `stdout-path` may name an alias instead of a path. Resolving it is a property lookup on
    // a well-known node, not a second search for a device.
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
