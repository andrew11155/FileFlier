//! The HTTP client used for update checks and downloading rclone.

use std::time::Duration;

/// HTTPS-only client that trusts the system's certificate store (like the
/// browser and system tools do), so it also works behind TLS-inspecting proxies.
pub fn agent(global_timeout: Duration) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .user_agent(concat!("file-flier/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(global_timeout))
        .build()
        .into()
}
