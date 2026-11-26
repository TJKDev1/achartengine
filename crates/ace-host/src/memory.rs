use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Plugin, PluginError};

/// A buffer in plugin memory: `len` bytes at `ptr`.
///
/// Exports and `host_call` return one packed into a `u64` as `ptr << 32 | len` (ADR 0001).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// Start of the buffer.
    pub ptr: u32,
    /// Length in bytes.
    pub len: u32,
}

impl Region {
    /// Packs the region into the `u64` that exports and `host_call` return.
    #[must_use]
    pub fn pack(self) -> u64 {
        // `+` rather than `|`: the halves don't overlap, so both give the same result, and
        // `cargo mutants` can't catch `|` turning into `^`, which also gives the same result.
        (u64::from(self.ptr) << 32) + u64::from(self.len)
    }

    /// Unpacks a region from an export's or `host_call`'s `u64` result.
    #[must_use]
    pub fn unpack(packed: u64) -> Self {
        Self {
            ptr: (packed >> 32) as u32,
            len: (packed & 0xffff_ffff) as u32,
        }
    }
}

/// Copies `bytes` into a buffer the plugin allocates with its `alloc(len) -> ptr` export.
///
/// # Errors
///
/// [`PluginError::Abi`] if `bytes` is longer than `u32::MAX` or `alloc` returns 0, its answer
/// for "out of memory". Otherwise any error from calling `alloc` or writing the buffer.
pub fn copy_in(plugin: &mut dyn Plugin, bytes: &[u8]) -> Result<Region, PluginError> {
    let len = u32::try_from(bytes.len()).map_err(|error| {
        PluginError::Abi(format!("{} bytes don't fit in a u32: {error}", bytes.len()))
    })?;
    let ptr = alloc(plugin, len)?;
    plugin.write(ptr, bytes)?;
    Ok(Region { ptr, len })
}

/// Calls the plugin's `alloc`, which returns 0 when it is out of memory.
fn alloc(plugin: &mut dyn Plugin, len: u32) -> Result<u32, PluginError> {
    let results = plugin.call("alloc", &[u64::from(len)])?;
    let [ptr] = results[..] else {
        return Err(PluginError::Abi(format!(
            "`alloc` returned {} values instead of one",
            results.len()
        )));
    };
    match u32::try_from(ptr) {
        Ok(0) => Err(PluginError::Abi(format!(
            "`alloc` has no room for {len} bytes"
        ))),
        Ok(ptr) => Ok(ptr),
        Err(error) => Err(PluginError::Abi(format!("`alloc` returned {ptr}: {error}"))),
    }
}

/// Copies a buffer the plugin returned out of its memory, then frees it with the plugin's
/// `free(ptr, len)` export.
///
/// # Errors
///
/// [`PluginError::OutOfBounds`] if the region isn't inside plugin memory, or any error from
/// calling `free`.
pub fn copy_out(plugin: &mut dyn Plugin, region: Region) -> Result<Vec<u8>, PluginError> {
    // The plugin chose `len` and could claim 4 GiB. Read the last byte first, so the buffer
    // below is never larger than the plugin's memory.
    if let Some(last) = region.len.checked_sub(1) {
        let out_of_bounds = PluginError::OutOfBounds {
            ptr: region.ptr,
            len: region.len as usize,
        };
        let end = region.ptr.checked_add(last).ok_or(out_of_bounds.clone())?;
        plugin
            .read(end, &mut [0])
            .map_err(|_out_of_bounds| out_of_bounds)?;
    }
    let mut bytes = vec![0; region.len as usize];
    plugin.read(region.ptr, &mut bytes)?;
    plugin.call("free", &[u64::from(region.ptr), u64::from(region.len)])?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{Region, copy_in, copy_out};
    use crate::PluginError;
    use crate::test_runtime::TestPlugin;

    #[test]
    fn regions_pack_high_pointer_and_low_length() {
        let region = Region {
            ptr: 0x1234_5678,
            len: 0x9abc_def0,
        };
        assert_eq!(
            region.pack(),
            0x1234_5678_9abc_def0,
            "ptr goes in the high half"
        );
        assert_eq!(
            Region::unpack(region.pack()),
            region,
            "unpack reverses pack"
        );
    }

    #[test]
    fn bytes_copied_in_come_back_out_and_are_freed() {
        let mut plugin = TestPlugin::new();
        let region = copy_in(&mut plugin, b"hello").unwrap();
        assert_eq!(region.len, 5, "region covers the bytes");
        assert_eq!(
            copy_out(&mut plugin, region).unwrap(),
            b"hello",
            "same bytes come back"
        );
        assert_eq!(plugin.freed, [region], "copy_out frees the buffer");
    }

    #[test]
    fn alloc_returning_zero_is_out_of_memory() {
        let mut plugin = TestPlugin::new();
        plugin.memory_full = true;
        assert_eq!(
            copy_in(&mut plugin, b"hello"),
            Err(PluginError::Abi("`alloc` has no room for 5 bytes".into())),
            "a zero pointer is refused before writing"
        );
    }

    #[test]
    fn reading_outside_memory_fails() {
        let mut plugin = TestPlugin::new();
        let region = Region {
            ptr: u32::MAX,
            len: 2,
        };
        assert_eq!(
            copy_out(&mut plugin, region),
            Err(PluginError::OutOfBounds {
                ptr: u32::MAX,
                len: 2
            }),
            "the read is checked against memory size"
        );
        assert!(
            plugin.freed.is_empty(),
            "nothing is freed after a failed read"
        );
    }

    #[test]
    fn a_region_larger_than_memory_is_refused_before_allocating() {
        let mut plugin = TestPlugin::new();
        let region = Region {
            ptr: 0,
            len: u32::MAX,
        };
        assert_eq!(
            copy_out(&mut plugin, region),
            Err(PluginError::OutOfBounds {
                ptr: 0,
                len: u32::MAX as usize
            }),
            "the region is checked against memory size"
        );
        assert_eq!(
            plugin.largest_read, 1,
            "only the last byte was read, so no 4 GiB buffer was made"
        );
    }
}
