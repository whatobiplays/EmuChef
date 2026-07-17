use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub pinned_artifact: bool,
    pub latest_release: bool,
    pub prerelease_filtering: bool,
    pub deterministic_asset_filtering: bool,
}

impl ProviderCapabilities {
    pub const fn pinned_only() -> Self {
        Self {
            pinned_artifact: true,
            latest_release: false,
            prerelease_filtering: false,
            deterministic_asset_filtering: false,
        }
    }

    pub const fn repository_release_provider() -> Self {
        Self {
            pinned_artifact: true,
            latest_release: true,
            prerelease_filtering: true,
            deterministic_asset_filtering: true,
        }
    }
}

pub trait AppSourceProvider {
    fn capabilities(&self) -> ProviderCapabilities;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DirectHttpsProvider;

impl AppSourceProvider for DirectHttpsProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::pinned_only()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReleaseProvider {
    repository_mode: bool,
}

impl ReleaseProvider {
    pub const fn repository() -> Self {
        Self {
            repository_mode: true,
        }
    }

    pub const fn release() -> Self {
        Self {
            repository_mode: false,
        }
    }
}

impl AppSourceProvider for ReleaseProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        if self.repository_mode {
            ProviderCapabilities::repository_release_provider()
        } else {
            ProviderCapabilities::pinned_only()
        }
    }
}

pub fn capabilities_for_mode(mode: &str) -> ProviderCapabilities {
    match mode {
        "github_repository" | "gitlab_repository" | "forgejo_repository" => {
            ReleaseProvider::repository().capabilities()
        }
        "github_release" | "gitlab_release" | "forgejo_release" => {
            ReleaseProvider::release().capabilities()
        }
        "direct_apk" => DirectHttpsProvider.capabilities(),
        _ => ProviderCapabilities::pinned_only(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_modes_advertise_latest_release() {
        for mode in [
            "github_repository",
            "gitlab_repository",
            "forgejo_repository",
        ] {
            assert!(capabilities_for_mode(mode).latest_release, "{mode}");
        }
        for mode in [
            "github_release",
            "gitlab_release",
            "forgejo_release",
            "direct_apk",
        ] {
            assert!(!capabilities_for_mode(mode).latest_release, "{mode}");
        }
    }
}
