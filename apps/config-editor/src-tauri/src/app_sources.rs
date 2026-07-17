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

    pub const fn github_repository() -> Self {
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

#[derive(Clone, Copy, Debug, Default)]
pub struct GithubProvider {
    repository_mode: bool,
}

impl GithubProvider {
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

impl AppSourceProvider for GithubProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        if self.repository_mode {
            ProviderCapabilities::github_repository()
        } else {
            ProviderCapabilities::pinned_only()
        }
    }
}

pub fn capabilities_for_mode(mode: &str) -> ProviderCapabilities {
    match mode {
        "github_repository" => GithubProvider::repository().capabilities(),
        "github_release" => GithubProvider::release().capabilities(),
        "direct_apk" => DirectHttpsProvider.capabilities(),
        _ => ProviderCapabilities::pinned_only(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_repository_advertises_latest_release() {
        assert!(capabilities_for_mode("github_repository").latest_release);
        assert!(!capabilities_for_mode("github_release").latest_release);
        assert!(!capabilities_for_mode("direct_apk").latest_release);
    }
}
