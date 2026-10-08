//! Application read models independent of transport and UI components.

/// Basic application identity for any presentation adapter.
pub struct ApplicationInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// Reads identity from the application's build metadata.
pub fn application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "Rig",
        version: env!("CARGO_PKG_VERSION"),
    }
}
